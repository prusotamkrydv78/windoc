//! Friends nearby: other Notchy PCs on the same Wi-Fi.
//!
//! Every 20 seconds each PC broadcasts `NOTCHY-HI <name>` on UDP 47822; anyone heard in the
//! last minute is "nearby". A wave is a direct `NOTCHY-WAVE <name>` packet: the friend's
//! robot waves and a toast says who it was. Nothing personal is sent, only the PC's name,
//! and a wave can only do that one harmless thing.

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::net::UdpSocket;

const PORT: u16 = 47822;
const ANNOUNCE_EVERY: Duration = Duration::from_secs(20);
const FORGET_AFTER: Duration = Duration::from_secs(60);
/// One wave per friend per this long, so nobody can spam a robot.
const WAVE_COOLDOWN: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct Friend {
    pub name: String,
    pub address: IpAddr,
    seen: Instant,
    waved_at: Option<Instant>,
}

struct Shared {
    friends: Vec<Friend>,
    /// Names of friends who waved, for the UI thread.
    waves: Vec<String>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    friends: Vec::new(),
    waves: Vec::new(),
});
static STARTED: AtomicBool = AtomicBool::new(false);

fn my_name() -> String {
    std::env::var("COMPUTERNAME")
        .unwrap_or_else(|_| "Notchy".to_string())
        .chars()
        .take(40)
        .collect()
}

fn own_address() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    socket.local_addr().ok().map(|address| address.ip())
}

/// Friends heard recently, by name.
pub fn nearby() -> Vec<Friend> {
    let now = Instant::now();
    let mut friends: Vec<Friend> = SHARED
        .lock()
        .friends
        .iter()
        .filter(|friend| now.saturating_duration_since(friend.seen) < FORGET_AFTER)
        .cloned()
        .collect();
    friends.sort_by(|a, b| a.name.cmp(&b.name));
    friends
}

/// Waves at `address`. `false` while the cooldown runs.
pub fn wave(address: IpAddr) -> bool {
    {
        let mut shared = SHARED.lock();
        let Some(friend) = shared.friends.iter_mut().find(|friend| friend.address == address) else {
            return false;
        };
        let now = Instant::now();
        if friend
            .waved_at
            .is_some_and(|at| now.saturating_duration_since(at) < WAVE_COOLDOWN)
        {
            return false;
        }
        friend.waved_at = Some(now);
    }
    let message = format!("NOTCHY-WAVE {}", my_name());
    std::thread::spawn(move || {
        if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
            let _ = socket.send_to(message.as_bytes(), SocketAddr::new(address, PORT));
        }
    });
    true
}

/// Starts listening (once) and returns names of friends who waved since the last call.
pub fn poll() -> Vec<String> {
    start();
    std::mem::take(&mut SHARED.lock().waves)
}

fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        STARTED.store(false, Ordering::SeqCst);
        return;
    };
    handle.spawn(async {
        let socket = match UdpSocket::bind(("0.0.0.0", PORT)).await {
            Ok(socket) => socket,
            Err(error) => {
                log::info!("Friends nearby unavailable: {error}");
                return;
            }
        };
        let _ = socket.set_broadcast(true);
        let socket = std::sync::Arc::new(socket);
        let announcer = socket.clone();
        tokio::spawn(async move {
            loop {
                let hello = format!("NOTCHY-HI {}", my_name());
                let _ = announcer
                    .send_to(hello.as_bytes(), ("255.255.255.255", PORT))
                    .await;
                tokio::time::sleep(ANNOUNCE_EVERY).await;
            }
        });
        let mut buffer = [0u8; 256];
        let mut replied: Vec<(IpAddr, Instant)> = Vec::new();
        loop {
            let Ok((count, from)) = socket.recv_from(&mut buffer).await else {
                continue;
            };
            if Some(from.ip()) == own_address() {
                continue;
            }
            let Ok(text) = std::str::from_utf8(&buffer[..count]) else {
                continue;
            };
            let clean = |name: &str| -> String {
                name.chars()
                    .filter(|character| !character.is_control())
                    .take(40)
                    .collect()
            };
            if let Some(name) = text.strip_prefix("NOTCHY-HI ") {
                let name = clean(name);
                let now = Instant::now();
                let new = {
                    let mut shared = SHARED.lock();
                    match shared.friends.iter_mut().find(|friend| friend.address == from.ip()) {
                        Some(friend) => {
                            friend.name = name;
                            friend.seen = now;
                            false
                        }
                        None => {
                            shared.friends.push(Friend {
                                name,
                                address: from.ip(),
                                seen: now,
                                waved_at: None,
                            });
                            true
                        }
                    }
                };
                // Say hi back once so a newcomer sees us without waiting 20 s.
                replied.retain(|(_, at)| now.saturating_duration_since(*at) < FORGET_AFTER);
                if new && !replied.iter().any(|(address, _)| *address == from.ip()) {
                    replied.push((from.ip(), now));
                    let hello = format!("NOTCHY-HI {}", my_name());
                    let _ = socket.send_to(hello.as_bytes(), from).await;
                }
                if new {
                    crate::platform::wake();
                }
            } else if let Some(name) = text.strip_prefix("NOTCHY-WAVE ") {
                let name = clean(name);
                let mut shared = SHARED.lock();
                if shared.waves.len() < 5 {
                    shared.waves.push(name);
                }
                drop(shared);
                crate::platform::wake();
            }
        }
    });
}
