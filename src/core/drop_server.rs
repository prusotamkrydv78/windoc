//! Local-network file transfer with a phone ("Drop", workflow §17).
//!
//! Security model:
//! - Binds only to this PC's LAN address, never to all interfaces.
//! - Every URL contains a 128-bit random token; requests without it get 404.
//! - The session stops after ten minutes or when the Drop page closes.
//! - Uploads are size-capped, names are sanitised, and existing files are never replaced.
//!
//! The only HTML here is the small page served to the phone's browser.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

const SESSION_LENGTH: Duration = Duration::from_secs(10 * 60);
const MAX_UPLOAD_BYTES: usize = 512 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 16 * 1024;

pub struct DropSession {
    pub url: String,
    pub expires: Instant,
    cancel: CancellationToken,
    received: Receiver<PathBuf>,
}

impl Drop for DropSession {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl DropSession {
    /// Files received since the last call.
    pub fn take_received(&self) -> Vec<PathBuf> {
        self.received.try_iter().collect()
    }

    pub fn expired(&self, now: Instant) -> bool {
        now >= self.expires
    }
}

fn lan_address() -> Option<IpAddr> {
    // Connecting a UDP socket sends nothing; it only selects the outgoing interface.
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let address = socket.local_addr().ok()?.ip();
    (!address.is_loopback() && !address.is_unspecified()).then_some(address)
}

fn token() -> Option<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).ok()?;
    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn drop_folder() -> PathBuf {
    dirs::download_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("WinIsland Drop")
}

fn sanitize(name: &str) -> String {
    let name: String = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("file")
        .chars()
        .filter(|character| !"<>:\"|?*".contains(*character) && !character.is_control())
        .take(120)
        .collect();
    let name = name.trim().trim_start_matches('.');
    if name.is_empty() {
        "file".to_string()
    } else {
        name.to_string()
    }
}

fn unique_path(directory: &Path, name: &str) -> PathBuf {
    let mut candidate = directory.join(name);
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) => (stem.to_string(), format!(".{extension}")),
        None => (name.to_string(), String::new()),
    };
    let mut counter = 2;
    while candidate.exists() {
        candidate = directory.join(format!("{stem} ({counter}){extension}"));
        counter += 1;
    }
    candidate
}

/// Starts a session serving `shared` files for download. Returns `None` without a LAN.
pub fn start(shared: Vec<PathBuf>) -> Option<DropSession> {
    let address = lan_address()?;
    let token = token()?;
    let std_listener = std::net::TcpListener::bind(SocketAddr::new(address, 0)).ok()?;
    std_listener.set_nonblocking(true).ok()?;
    let port = std_listener.local_addr().ok()?.port();
    let listener = TcpListener::from_std(std_listener).ok()?;
    let cancel = CancellationToken::new();
    let (sender, received) = mpsc::channel();
    let state = Arc::new(ServerState {
        token: token.clone(),
        shared: Mutex::new(shared),
        sender: Mutex::new(sender),
    });
    let server_cancel = cancel.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                () = server_cancel.cancelled() => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    let state = state.clone();
                    tokio::spawn(async move {
                        if let Err(error) = handle(stream, &state).await {
                            log::warn!("Drop request failed: {error}");
                        }
                    });
                }
            }
        }
    });
    let cancel_after = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(SESSION_LENGTH).await;
        cancel_after.cancel();
    });
    Some(DropSession {
        url: format!("http://{address}:{port}/{token}/"),
        expires: Instant::now() + SESSION_LENGTH,
        cancel,
        received,
    })
}

struct ServerState {
    token: String,
    shared: Mutex<Vec<PathBuf>>,
    sender: Mutex<Sender<PathBuf>>,
}

async fn respond(
    stream: &mut TcpStream,
    status: &str,
    kind: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(body).await
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn page(state: &ServerState) -> String {
    let files: String = state
        .shared
        .lock()
        .iter()
        .enumerate()
        .filter_map(|(index, path)| {
            let name = path.file_name()?.to_string_lossy().into_owned();
            Some(format!(
                "<li><a href=\"/{}/file/{index}\">{}</a></li>",
                state.token,
                escape(&name)
            ))
        })
        .collect();
    format!(
        "<!doctype html><meta name=viewport content=\"width=device-width\"><title>Drop</title>\
         <body style=\"font-family:system-ui;margin:24px\"><h2>Send to PC</h2>\
         <form method=post enctype=multipart/form-data action=\"/{token}/upload\">\
         <input type=file name=file required> <button>Send</button></form>\
         <h2>Files on the PC shelf</h2><ul>{files}</ul></body>",
        token = state.token
    )
}

async fn handle(mut stream: TcpStream, state: &ServerState) -> std::io::Result<()> {
    let mut reader = BufReader::new(&mut stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).await?;
    let mut headers = Vec::new();
    let mut header_bytes = 0;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await?;
        header_bytes += read;
        if read == 0 || line == "\r\n" || header_bytes > MAX_HEADER_BYTES {
            break;
        }
        headers.push(line.trim_end().to_string());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let prefix = format!("/{}/", state.token);
    let Some(route) = path.strip_prefix(&prefix) else {
        drop(reader);
        return respond(&mut stream, "404 Not Found", "text/plain", b"Not found").await;
    };
    let route = route.to_string();
    if method == "GET" && route.is_empty() {
        drop(reader);
        return respond(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            page(state).as_bytes(),
        )
        .await;
    }
    if method == "GET"
        && let Some(index) = route
            .strip_prefix("file/")
            .and_then(|index| index.parse::<usize>().ok())
    {
        drop(reader);
        let file = state.shared.lock().get(index).cloned();
        return match file.and_then(|path| std::fs::read(path).ok()) {
            Some(bytes) => respond(&mut stream, "200 OK", "application/octet-stream", &bytes).await,
            None => respond(&mut stream, "404 Not Found", "text/plain", b"Not found").await,
        };
    }
    if method == "POST" && route == "upload" {
        let length: usize = headers
            .iter()
            .find_map(|header| {
                let (name, value) = header.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())
                    .flatten()
            })
            .unwrap_or(0);
        let boundary = headers.iter().find_map(|header| {
            header
                .split_once("boundary=")
                .map(|(_, boundary)| boundary.trim().trim_matches('"').to_string())
        });
        if length == 0 || length > MAX_UPLOAD_BYTES {
            drop(reader);
            return respond(
                &mut stream,
                "413 Payload Too Large",
                "text/plain",
                b"Too large",
            )
            .await;
        }
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).await?;
        drop(reader);
        let saved = boundary.and_then(|boundary| save_upload(&body, &boundary));
        return match saved {
            Some(path) => {
                let _ = state.sender.lock().send(path);
                respond(
                    &mut stream,
                    "200 OK",
                    "text/html; charset=utf-8",
                    b"<p>Sent. You can go back and send another file.</p>",
                )
                .await
            }
            None => {
                respond(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain",
                    b"Upload failed",
                )
                .await
            }
        };
    }
    drop(reader);
    respond(&mut stream, "404 Not Found", "text/plain", b"Not found").await
}

/// Extracts the first file part of a multipart body and saves it to the Drop folder.
fn save_upload(body: &[u8], boundary: &str) -> Option<PathBuf> {
    let marker = format!("--{boundary}");
    let start = find(body, marker.as_bytes())? + marker.len();
    let headers_end = find(&body[start..], b"\r\n\r\n")? + start;
    let headers = String::from_utf8_lossy(&body[start..headers_end]);
    let name = headers
        .split("filename=\"")
        .nth(1)?
        .split('"')
        .next()
        .map(sanitize)?;
    let content_start = headers_end + 4;
    let end_marker = format!("\r\n--{boundary}");
    let content_end = find(&body[content_start..], end_marker.as_bytes())? + content_start;
    let folder = drop_folder();
    std::fs::create_dir_all(&folder).ok()?;
    let path = unique_path(&folder, &name);
    std::fs::write(&path, &body[content_start..content_end]).ok()?;
    Some(path)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
