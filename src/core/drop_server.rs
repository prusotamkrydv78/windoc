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
        let mut status = STATUS.lock();
        if status.link.as_ref().is_some_and(|(url, _)| *url == self.url) {
            status.link = None;
        }
        for transfer in &mut status.transfers {
            if transfer.state == TransferState::Active {
                transfer.state = TransferState::Failed;
            }
        }
    }
}

/// Which way a file went.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// Phone to PC.
    Received,
    /// PC to phone.
    Sent,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TransferState {
    Active,
    Done,
    Failed,
}

/// One file moved (or moving) during this app run.
#[derive(Clone, Debug)]
pub struct Transfer {
    pub id: u64,
    pub name: String,
    /// The file on this PC (the saved upload, or the shared file).
    pub path: Option<PathBuf>,
    pub direction: Direction,
    pub state: TransferState,
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub started: Instant,
}

struct Status {
    /// The live session's `(url, expires)`.
    link: Option<(String, Instant)>,
    transfers: Vec<Transfer>,
    next_id: u64,
    renew: bool,
}

const MAX_HISTORY: usize = 50;

static STATUS: Mutex<Status> = Mutex::new(Status {
    link: None,
    transfers: Vec::new(),
    next_id: 1,
    renew: false,
});

/// The live session's `(url, expires)`, if any.
pub fn current_link() -> Option<(String, Instant)> {
    STATUS.lock().link.clone()
}

/// Transfers of this app run, newest first.
pub fn transfers() -> Vec<Transfer> {
    STATUS.lock().transfers.iter().rev().cloned().collect()
}

/// Ends the live session so the app starts a fresh one (new token, new ten minutes).
pub fn request_new_link() {
    let mut status = STATUS.lock();
    if status.link.is_some() {
        status.renew = true;
    }
}

/// Where uploads from the phone are saved.
pub fn received_folder() -> PathBuf {
    drop_folder()
}

fn begin_transfer(name: String, direction: Direction, total_bytes: u64) -> u64 {
    let mut status = STATUS.lock();
    let id = status.next_id;
    status.next_id += 1;
    status.transfers.push(Transfer {
        id,
        name,
        path: None,
        direction,
        state: TransferState::Active,
        total_bytes,
        done_bytes: 0,
        started: Instant::now(),
    });
    if status.transfers.len() > MAX_HISTORY {
        let excess = status.transfers.len() - MAX_HISTORY;
        status.transfers.drain(..excess);
    }
    id
}

fn update_transfer(id: u64, update: impl FnOnce(&mut Transfer)) {
    if let Some(transfer) = STATUS
        .lock()
        .transfers
        .iter_mut()
        .find(|transfer| transfer.id == id)
    {
        update(transfer);
    }
}

impl DropSession {
    /// Files received since the last call.
    pub fn take_received(&self) -> Vec<PathBuf> {
        self.received.try_iter().collect()
    }

    /// Also `true` once after [`request_new_link`], so the app replaces the session.
    pub fn expired(&self, now: Instant) -> bool {
        let renew = std::mem::take(&mut STATUS.lock().renew);
        renew || now >= self.expires
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
        .join("Notchy Drop")
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
    let url = format!("http://{address}:{port}/{token}/");
    let expires = Instant::now() + SESSION_LENGTH;
    {
        let mut status = STATUS.lock();
        status.link = Some((url.clone(), expires));
        status.renew = false;
    }
    Some(DropSession {
        url,
        expires,
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
            if !path.is_file() {
                return None;
            }
            let name = escape(&path.file_name()?.to_string_lossy());
            let size = std::fs::metadata(path).map_or(0, |meta| meta.len());
            Some(format!(
                "<li class=card><div class=ic>&#8595;</div><div class=meta><b>{name}</b>\
                 <small data-size=\"{size}\"></small></div>\
                 <a class=dl href=\"/{token}/file/{index}\" download=\"{name}\">Get</a></li>",
                token = state.token
            ))
        })
        .collect();
    let files = if files.is_empty() {
        "<p class=empty>Nothing shared yet. Files on the PC's Shelf show up here.</p>".to_string()
    } else {
        format!("<ul class=list>{files}</ul>")
    };
    let seconds = current_link()
        .map_or(0, |(_, expires)| expires.saturating_duration_since(Instant::now()).as_secs());
    let pc = escape(&std::env::var("COMPUTERNAME").unwrap_or_else(|_| "your PC".to_string()));
    PAGE.replace("{{TOKEN}}", &state.token)
        .replace("{{PC}}", &pc)
        .replace("{{SECONDS}}", &seconds.to_string())
        .replace("{{FILES}}", &files)
}

/// The phone page: one self-contained document, no external resources (the phone may have
/// no internet). Files upload one per request so each gets its own progress bar.
const PAGE: &str = r##"<!doctype html><html lang=en><head><meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1,viewport-fit=cover">
<meta name=theme-color content="#0b0c0f"><title>WinIsland Drop</title>
<style>
:root{--bg:#0b0c0f;--card:#17191e;--card2:#1f2229;--line:rgba(255,255,255,.09);--text:#eef0f4;--dim:#9aa1ae;--accent:#e8622c;--ok:#58c482;--err:#ea605a}
*{box-sizing:border-box;-webkit-tap-highlight-color:transparent}
html,body{margin:0;background:var(--bg);color:var(--text);font:16px/1.4 system-ui,-apple-system,"Segoe UI",Roboto,sans-serif}
main{max-width:560px;margin:0 auto;padding:calc(16px + env(safe-area-inset-top)) calc(16px + env(safe-area-inset-right)) calc(24px + env(safe-area-inset-bottom)) calc(16px + env(safe-area-inset-left))}
header{display:flex;align-items:center;gap:12px;margin:4px 0 18px}
.logo{width:40px;height:40px;border-radius:12px;background:var(--card2);display:grid;place-items:center;font-weight:700}
header h1{font-size:19px;margin:0}header p{margin:0;color:var(--dim);font-size:13px;display:flex;align-items:center;gap:6px}
.dot{width:8px;height:8px;border-radius:50%;background:var(--ok);box-shadow:0 0 0 3px rgba(88,196,130,.2)}
.dot.off{background:var(--err);box-shadow:none}
section{background:var(--card);border:1px solid var(--line);border-radius:20px;padding:16px;margin-bottom:14px}
h2{font-size:13px;letter-spacing:.06em;text-transform:uppercase;color:var(--dim);margin:0 0 12px;font-weight:600}
.zone{display:flex;flex-direction:column;align-items:center;justify-content:center;gap:6px;min-height:150px;border:2px dashed rgba(255,255,255,.18);border-radius:16px;text-align:center;padding:20px;cursor:pointer}
.zone:active,.zone.over{border-color:var(--accent);background:rgba(232,98,44,.07)}
.zone b{font-size:17px}.zone span{color:var(--dim);font-size:14px}.plus{font-size:34px;line-height:1;color:var(--accent)}
input[type=file]{display:none}
.list{list-style:none;margin:12px 0 0;padding:0;display:flex;flex-direction:column;gap:8px}
.card{display:flex;align-items:center;gap:12px;background:var(--card2);border-radius:14px;padding:10px 12px;min-height:60px}
.thumb,.ic{width:44px;height:44px;border-radius:10px;flex:none;object-fit:cover;background:#2a2e37;display:grid;place-items:center;color:var(--dim);font-size:12px;font-weight:600;text-transform:uppercase}
.meta{flex:1;min-width:0}.meta b{display:block;font-weight:500;font-size:15px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.meta small{color:var(--dim);font-size:13px}
.bar{height:4px;border-radius:2px;background:#2d313b;margin-top:6px;overflow:hidden}.bar i{display:block;height:100%;width:0;background:var(--accent);transition:width .15s}
.st{flex:none;min-width:44px;height:44px;display:grid;place-items:center;font-size:20px;border:0;background:none;color:var(--dim);border-radius:12px}
.st.ok{color:var(--ok)}.st.err{color:var(--err);font-size:14px;font-weight:600;background:rgba(234,96,90,.12);padding:0 12px}
button.rm{font-size:22px}
.send{width:100%;margin-top:14px;min-height:54px;border:0;border-radius:16px;background:var(--accent);color:#fff;font:600 17px system-ui,sans-serif}
.send:disabled{opacity:.4}
.dl{flex:none;min-height:40px;padding:0 16px;border-radius:12px;background:var(--card);border:1px solid var(--line);color:var(--text);text-decoration:none;display:grid;place-items:center;font-weight:600;font-size:14px}
.empty{color:var(--dim);font-size:14px;margin:0}
footer{text-align:center;color:var(--dim);font-size:13px;margin-top:8px}
</style></head><body><main>
<header><div class=logo>W</div><div><h1>WinIsland Drop</h1><p><span class=dot id=dot></span><span id=conn>Connected to {{PC}}</span></p></div></header>
<section><h2>Send to PC</h2>
<label class=zone id=zone><div class=plus>+</div><b>Tap to choose photos or files</b><span>or drop them here</span>
<input type=file id=pick multiple></label>
<ul class=list id=queue></ul>
<button class=send id=send disabled>Send</button></section>
<section><h2>Files from PC</h2>{{FILES}}</section>
<footer id=foot></footer>
</main><script>
(function(){
var T="{{TOKEN}}",left={{SECONDS}},queue=[],busy=false;
var $=function(id){return document.getElementById(id)};
function size(n){var u=["B","KB","MB","GB"],i=0;while(n>=1024&&i<3){n/=1024;i++}return(i?n.toFixed(1):n)+" "+u[i]}
document.querySelectorAll("[data-size]").forEach(function(e){e.textContent=size(+e.dataset.size)});
function ext(n){var m=/\.([^.]{1,4})$/.exec(n);return m?m[1]:"file"}
function add(files){for(var i=0;i<files.length;i++){var f=files[i],li=document.createElement("li");li.className="card";
var t;if(/^image\//.test(f.type)){t=document.createElement("img");t.className="thumb";t.src=URL.createObjectURL(f)}else{t=document.createElement("div");t.className="ic";t.textContent=ext(f.name)}
var m=document.createElement("div");m.className="meta";var b=document.createElement("b");b.textContent=f.name;var s=document.createElement("small");s.textContent=size(f.size);
var bar=document.createElement("div");bar.className="bar";bar.hidden=true;var fill=document.createElement("i");bar.appendChild(fill);m.append(b,s,bar);
var st=document.createElement("button");st.className="st rm";st.textContent="×";st.setAttribute("aria-label","Remove");
var item={file:f,li:li,fill:fill,bar:bar,st:st,state:"new"};
st.onclick=function(it){return function(){if(it.state=="new"){queue.splice(queue.indexOf(it),1);it.li.remove();update()}else if(it.state=="err"){it.state="new";run()}}}(item);
li.append(t,m,st);$("queue").appendChild(li);queue.push(item)}update()}
function pending(){return queue.filter(function(q){return q.state=="new"})}
function update(){var p=pending(),total=p.reduce(function(a,q){return a+q.file.size},0);
$("send").disabled=busy||!p.length;$("send").textContent=busy?"Sending…":p.length?"Send "+p.length+(p.length>1?" files":" file")+" · "+size(total):"Send"}
function upload(it,done){it.state="up";it.bar.hidden=false;it.st.className="st";it.st.textContent="";
var fd=new FormData();fd.append("file",it.file,it.file.name);var x=new XMLHttpRequest();x.open("POST","/"+T+"/upload");
x.upload.onprogress=function(e){if(e.lengthComputable)it.fill.style.width=(e.loaded/e.total*100)+"%"};
x.onload=function(){if(x.status==200){it.state="ok";it.fill.style.width="100%";it.st.className="st ok";it.st.textContent="✓"}else fail(it);done()};
x.onerror=function(){fail(it);done()};x.send(fd)}
function fail(it){it.state="err";it.st.className="st err";it.st.textContent="Retry"}
function run(){if(busy)return;var next=pending()[0];if(!next){update();return}busy=true;update();upload(next,function(){busy=false;run()})}
$("pick").onchange=function(){add(this.files);this.value=""};
$("send").onclick=run;
var z=$("zone");["dragenter","dragover"].forEach(function(n){z.addEventListener(n,function(e){e.preventDefault();z.classList.add("over")})});
["dragleave","drop"].forEach(function(n){z.addEventListener(n,function(e){e.preventDefault();z.classList.remove("over")})});
z.addEventListener("drop",function(e){add(e.dataTransfer.files)});
function tick(){if(left<=0){$("foot").textContent="This link has expired. Scan the new code on your PC.";$("dot").className="dot off";$("conn").textContent="Link expired";return}
$("foot").textContent="Link expires in "+Math.floor(left/60)+":"+("0"+left%60).slice(-2)+" · same Wi-Fi only";left--;setTimeout(tick,1000)}
tick();update();
})();
</script></body></html>"##;

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
        return match file.and_then(|path| std::fs::read(&path).ok().map(|bytes| (path, bytes))) {
            Some((path, bytes)) => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let total = bytes.len() as u64;
                let id = begin_transfer(name, Direction::Sent, total);
                update_transfer(id, |transfer| transfer.path = Some(path));
                let result =
                    respond(&mut stream, "200 OK", "application/octet-stream", &bytes).await;
                update_transfer(id, |transfer| {
                    if result.is_ok() {
                        transfer.done_bytes = total;
                        transfer.state = TransferState::Done;
                    } else {
                        transfer.state = TransferState::Failed;
                    }
                });
                result
            }
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
        let id = begin_transfer("Receiving…".to_string(), Direction::Received, length as u64);
        let mut body = vec![0u8; length];
        let mut read = 0;
        let mut named = false;
        while read < length {
            let count = match reader.read(&mut body[read..]).await {
                Ok(0) => {
                    update_transfer(id, |transfer| transfer.state = TransferState::Failed);
                    return Err(std::io::ErrorKind::UnexpectedEof.into());
                }
                Ok(count) => count,
                Err(error) => {
                    update_transfer(id, |transfer| transfer.state = TransferState::Failed);
                    return Err(error);
                }
            };
            read += count;
            let name = if named {
                None
            } else {
                upload_name(&body[..read])
            };
            named |= name.is_some();
            update_transfer(id, |transfer| {
                transfer.done_bytes = read as u64;
                if let Some(name) = name {
                    transfer.name = name;
                }
            });
        }
        drop(reader);
        let saved = boundary.and_then(|boundary| save_upload(&body, &boundary));
        update_transfer(id, |transfer| match &saved {
            Some(path) => {
                transfer.state = TransferState::Done;
                if let Some(name) = path.file_name() {
                    transfer.name = name.to_string_lossy().into_owned();
                }
                transfer.total_bytes = std::fs::metadata(path).map_or(0, |meta| meta.len());
                transfer.done_bytes = transfer.total_bytes;
                transfer.path = Some(path.clone());
            }
            None => transfer.state = TransferState::Failed,
        });
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

/// The sanitised file name of a partly read multipart body, once its headers arrived.
fn upload_name(body: &[u8]) -> Option<String> {
    let start = find(body, b"filename=\"")? + 10;
    let end = body[start..].iter().position(|byte| *byte == b'"')? + start;
    Some(sanitize(&String::from_utf8_lossy(&body[start..end])))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
