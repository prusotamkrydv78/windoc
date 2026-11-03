//! Notchy Link: a lasting, encrypted connection to the Notchy phone app.
//!
//! - Pairing: the Phone tab shows a QR code holding this PC's address and a random 256-bit
//!   key. Whoever holds the key is the paired phone; "Pair a new phone" makes a new key.
//! - Transport: TCP on port 47820 (or the next free one). Every frame is
//!   `[u32 length][12-byte nonce][ChaCha20-Poly1305 ciphertext]` of one JSON message.
//! - Handshake: the PC sends a random challenge; the phone's first message must be a
//!   `hello` that repeats it, so a recorded session cannot be replayed.
//! - Discovery: the phone finds the PC again after an address change by broadcasting
//!   `NOTCHY?<pair id>` to UDP 47821; the PC answers `NOTCHY!<port>`.
//!
//! The network side runs on tokio. Anything that must happen on the UI thread (clipboard,
//! the robot, toasts) is queued as an [`Event`] and drained by the app's frame.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

const FILE: &str = "phone_link.json";
const PORT: u16 = 47820;
const DISCOVERY_PORT: u16 = 47821;
const MAX_FRAME: usize = 1024 * 1024;
const IDLE_TIMEOUT: Duration = Duration::from_secs(70);
const MAX_NOTIFICATIONS: usize = 30;
/// How long the phone may be gone before "lock when my phone leaves" locks the PC.
const LEAVE_GRACE: Duration = Duration::from_secs(90);
/// No keyboard or mouse for this long and the robot "follows" the user to the phone.
const AWAY_AFTER: Duration = Duration::from_secs(5 * 60);

/// Saved pairing and the user's choices, in `phone_link.json`.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Pairing {
    /// The 32-byte key as hex.
    pub key: String,
    /// The paired phone's name, once it has connected.
    pub device: Option<String>,
    pub clipboard_sync: bool,
    pub alerts: bool,
    pub lock_on_leave: bool,
}

impl Default for Pairing {
    fn default() -> Self {
        Self {
            key: String::new(),
            device: None,
            clipboard_sync: true,
            alerts: true,
            lock_on_leave: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PhoneNotification {
    /// The phone's notification key, for replying.
    pub key: String,
    /// The phone can reply to it from here.
    pub can_reply: bool,
    pub app: String,
    pub title: String,
    pub text: String,
    /// A one-time code found in the text.
    pub otp: Option<String>,
    pub at: Instant,
}

#[derive(Clone, Debug, Default)]
pub struct PhoneMedia {
    pub title: String,
    pub artist: String,
    pub playing: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Connection {
    pub name: String,
    pub battery: Option<(u8, bool)>,
}

/// Work for the UI thread.
#[derive(Clone, Debug)]
pub enum Event {
    /// Put text on the PC clipboard.
    Clip(String),
    OpenUrl(String),
    /// "prev", "toggle" or "next" for the PC's media.
    Media(String),
    Toast(String, String),
    /// The phone connected.
    Arrived,
    /// The phone threw the robot over.
    Thrown,
    /// "Find my PC".
    Ring,
    /// A file from the phone, saved in the Drop folder.
    File(std::path::PathBuf),
    /// The phone's battery ran low.
    PhoneLow(u8),
    /// The robot was petted on the phone.
    Petted,
    /// A phone note for the PC's Notes tab.
    Note(String),
    /// The phone wants the PC's notes list.
    NotesWanted,
    /// An "on this day" photo arrived: `(path, years ago)`.
    Photo(std::path::PathBuf, u32),
}

/// Photo ids announced with `photo_memory`, so the finished file is labelled.
static PHOTO_YEARS: Mutex<Vec<(u64, u32)>> = Mutex::new(Vec::new());

/// The PC's notes, for the phone's Notes page.
pub fn send_notes(notes: Vec<(String, String)>) {
    let notes: Vec<Value> = notes
        .into_iter()
        .map(|(title, detail)| json!({"title": title, "detail": detail}))
        .collect();
    send(json!({"t": "notes", "items": notes}));
}

static WEATHER_SENT: Mutex<Option<String>> = Mutex::new(None);

/// The weather, so the phone robot wears sunglasses on a clear day and a scarf when cold.
pub fn publish_weather(weather: Option<(&str, &str)>) {
    let wear = weather.and_then(|(condition, temperature)| {
        let degrees: f32 = temperature
            .chars()
            .take_while(|character| character.is_ascii_digit() || *character == '-' || *character == '.')
            .collect::<String>()
            .parse()
            .ok()?;
        Some(if degrees <= 15.0 { "cold" } else if condition == "weather_clear" { "sun" } else { "" })
    })
    .unwrap_or("");
    let connected = SHARED.lock().outbox.is_some();
    let mut sent = WEATHER_SENT.lock();
    if !connected {
        *sent = None;
        return;
    }
    if sent.as_deref() != Some(wear) {
        *sent = Some(wear.to_string());
        send(json!({"t": "wear", "wear": wear}));
    }
}

static BRIEF_DAY: Mutex<String> = Mutex::new(String::new());

/// The morning brief, once a day after 5 am while the phone is linked.
pub fn publish_brief(meetings: usize, weather: Option<String>) {
    let local = crate::platform::shell().local_datetime();
    if local.hour < 5 || SHARED.lock().outbox.is_none() {
        return;
    }
    let today = format!("{}-{}-{}", local.year, local.month, local.day);
    let mut day = BRIEF_DAY.lock();
    if *day == today {
        return;
    }
    *day = today;
    send(json!({"t": "brief", "meetings": meetings, "weather": weather}));
}

/// Smart Focus: a nudge the phone robot repeats.
pub fn send_nudge(text: &str) {
    send(json!({"t": "nudge", "text": text}));
}

/// A file arriving from the phone in chunks.
struct Incoming {
    id: u64,
    path: std::path::PathBuf,
    file: std::fs::File,
    expected: u64,
    written: u64,
}

static INCOMING: Mutex<Option<Incoming>> = Mutex::new(None);
const MAX_FILE: u64 = 2 * 1024 * 1024 * 1024;

struct Shared {
    pairing: Option<Pairing>,
    connection: Option<Connection>,
    outbox: Option<UnboundedSender<String>>,
    /// Ends the live connection when a new one takes over or the pairing changes.
    generation: u64,
    port: u16,
    notifications: Vec<PhoneNotification>,
    media: Option<PhoneMedia>,
    events: Vec<Event>,
    last_seen: Option<Instant>,
    /// The last clipboard text that came from the phone, so it is not echoed back.
    echo: Option<String>,
    revision: u64,
    call: Option<Call>,
    live: Vec<LiveItem>,
    phone_screen: Option<(u32, Vec<(String, u32)>)>,
}

/// A phone call, mirrored on the PC.
#[derive(Clone, Debug)]
pub struct Call {
    pub name: String,
    /// "ringing" or "ongoing".
    pub state: String,
    pub since: Instant,
    /// Buttons the phone's call notification offers.
    pub actions: Vec<String>,
}

/// A delivery, ride or other live progress from the phone.
#[derive(Clone, Debug)]
pub struct LiveItem {
    pub key: String,
    pub app: String,
    pub title: String,
    pub text: String,
    /// 0–100, or `None` without a bar.
    pub progress: Option<u8>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    pairing: None,
    connection: None,
    outbox: None,
    generation: 0,
    port: 0,
    notifications: Vec::new(),
    media: None,
    events: Vec::new(),
    last_seen: None,
    echo: None,
    revision: 0,
    call: None,
    live: Vec::new(),
    phone_screen: None,
});
static STARTED: AtomicBool = AtomicBool::new(false);
static LOADED: AtomicBool = AtomicBool::new(false);

fn changed(shared: &mut Shared) {
    shared.revision = shared.revision.wrapping_add(1);
    crate::platform::wake();
}

fn load() {
    if LOADED.swap(true, Ordering::SeqCst) {
        return;
    }
    let pairing = match crate::core::json_store::read::<Pairing>(FILE, 64 * 1024) {
        Ok(pairing) => pairing.filter(|pairing| decode_key(&pairing.key).is_some()),
        Err(error) => {
            log::warn!("Cannot read {FILE}: {error}");
            None
        }
    };
    SHARED.lock().pairing = pairing;
}

fn save(pairing: &Pairing) {
    if let Err(error) = crate::core::json_store::write(FILE, pairing) {
        log::warn!("Cannot save {FILE}: {error}");
    }
}

fn decode_key(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut key = [0u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(key)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn base64url(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Short public id of a key, used by discovery so the PC only answers its own phone.
fn pair_id(key: &[u8; 32]) -> String {
    hex(&Sha256::digest(key)[..8])
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_millis() as u64)
}

fn pc_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".to_string())
}

fn lan_address() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let address = socket.local_addr().ok()?.ip();
    (!address.is_loopback() && !address.is_unspecified()).then_some(address)
}

// ---- Public API (UI thread) -------------------------------------------------------------

/// Whether a phone has ever been paired (or a pairing code is waiting).
pub fn paired() -> bool {
    load();
    SHARED
        .lock()
        .pairing
        .as_ref()
        .is_some_and(|pairing| pairing.device.is_some())
}

pub fn pairing() -> Option<Pairing> {
    load();
    SHARED.lock().pairing.clone()
}

/// The QR text for pairing, making a key first if there is none. `None` without a network.
pub fn pairing_code() -> Option<String> {
    load();
    let address = lan_address()?;
    let key = {
        let mut shared = SHARED.lock();
        if shared.pairing.is_none() {
            let mut key = [0u8; 32];
            getrandom::fill(&mut key).ok()?;
            let pairing = Pairing {
                key: hex(&key),
                ..Pairing::default()
            };
            save(&pairing);
            shared.pairing = Some(pairing);
            changed(&mut shared);
        }
        decode_key(&shared.pairing.as_ref()?.key)?
    };
    ensure_started();
    let port = SHARED.lock().port;
    let port = if port == 0 { PORT } else { port };
    Some(format!(
        "notchy://pair?h={address}&p={port}&k={}&n={}",
        base64url(&key),
        pc_name().replace(['&', '=', ' '], "-")
    ))
}

/// Forgets the phone and makes a fresh key, so the old phone can no longer connect.
pub fn pair_again() {
    load();
    let mut shared = SHARED.lock();
    let mut key = [0u8; 32];
    if getrandom::fill(&mut key).is_err() {
        return;
    }
    let mut pairing = shared.pairing.clone().unwrap_or_default();
    pairing.key = hex(&key);
    pairing.device = None;
    save(&pairing);
    shared.pairing = Some(pairing);
    shared.generation += 1;
    shared.connection = None;
    shared.outbox = None;
    shared.notifications.clear();
    shared.media = None;
    changed(&mut shared);
}

/// Changes one of the saved switches.
pub fn update_settings(update: impl FnOnce(&mut Pairing)) {
    load();
    let mut shared = SHARED.lock();
    if let Some(pairing) = shared.pairing.as_mut() {
        update(pairing);
        save(pairing);
    }
    changed(&mut shared);
}

pub fn connection() -> Option<Connection> {
    SHARED.lock().connection.clone()
}

pub fn notifications() -> Vec<PhoneNotification> {
    SHARED.lock().notifications.iter().rev().cloned().collect()
}

pub fn phone_media() -> Option<PhoneMedia> {
    SHARED.lock().media.clone()
}

pub fn clear_notifications() {
    let mut shared = SHARED.lock();
    shared.notifications.clear();
    changed(&mut shared);
}

fn send(message: Value) -> bool {
    let shared = SHARED.lock();
    shared
        .outbox
        .as_ref()
        .is_some_and(|outbox| outbox.send(message.to_string()).is_ok())
}

static SENDING: Mutex<Option<u8>> = Mutex::new(None);
const MAX_SEND: u64 = 1024 * 1024 * 1024;

/// Percent of the file going to the phone, while one is.
pub fn file_progress() -> Option<u8> {
    *SENDING.lock()
}

/// Sends files to the phone's Downloads, one after another. Blocks: call off the UI thread.
pub fn send_files(paths: Vec<std::path::PathBuf>) {
    use base64::Engine;
    use std::io::Read;
    let total: u64 = paths
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum::<u64>()
        .max(1);
    *SENDING.lock() = Some(0);
    let mut done = 0u64;
    'files: for path in paths {
        let Ok(mut file) = std::fs::File::open(&path) else {
            continue;
        };
        let size = file.metadata().map_or(0, |meta| meta.len());
        if size > MAX_SEND {
            continue;
        }
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());
        let id = epoch_ms();
        if !send(json!({"t": "file_start", "id": id, "name": name, "size": size})) {
            break;
        }
        crate::core::memory::record(crate::core::memory::Kind::FileOut, name.clone(), "Sent to phone", "");
        let mut buffer = vec![0u8; 192 * 1024];
        loop {
            let read = match file.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => read,
                Err(_) => break 'files,
            };
            let data = base64::engine::general_purpose::STANDARD.encode(&buffer[..read]);
            if !send(json!({"t": "file_chunk", "id": id, "data": data})) {
                break 'files;
            }
            done += read as u64;
            *SENDING.lock() = Some((done * 100 / total).min(99) as u8);
            crate::platform::wake();
            // The outbox is unbounded; pace it so a big file does not pile up in memory.
            std::thread::sleep(Duration::from_millis(12));
        }
        send(json!({"t": "file_end", "id": id}));
    }
    *SENDING.lock() = None;
    crate::platform::wake();
}

static HISTORY: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The PC's recent clipboard texts, newest first, for the phone's "PC clipboard" list.
pub fn set_history(items: Vec<String>) {
    *HISTORY.lock() = items;
}

/// Clipboard history texts containing every word of `query`, newest first.
pub fn history_matches(query: &str, limit: usize) -> Vec<String> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    HISTORY
        .lock()
        .iter()
        .filter(|item| {
            let lower = item.to_lowercase();
            words.iter().all(|word| lower.contains(word))
        })
        .take(limit)
        .cloned()
        .collect()
}

static OFFERED: Mutex<Vec<(u64, std::path::PathBuf)>> = Mutex::new(Vec::new());

/// A download finished on the PC: the phone shows it, with "Get it here".
pub fn offer_download(path: std::path::PathBuf) {
    let Some(name) = path.file_name().map(|name| name.to_string_lossy().into_owned()) else {
        return;
    };
    let size = std::fs::metadata(&path).map_or(0, |meta| meta.len());
    let id = epoch_ms();
    crate::core::memory::record(crate::core::memory::Kind::Download, name.clone(), path.display().to_string(), "");
    if send(json!({"t": "download", "id": id, "name": name, "size": size})) {
        let mut offered = OFFERED.lock();
        offered.push((id, path));
        if offered.len() > 10 {
            offered.remove(0);
        }
    }
}

/// A file the phone asked for (a download, or a Shelf file).
fn send_offered(id: u64) {
    let path = OFFERED
        .lock()
        .iter()
        .find(|(offered, _)| *offered == id)
        .map(|(_, path)| path.clone());
    if let Some(path) = path
        && file_progress().is_none()
    {
        std::thread::spawn(move || send_files(vec![path]));
    }
}

pub fn call() -> Option<Call> {
    SHARED.lock().call.clone()
}

pub fn live_items() -> Vec<LiveItem> {
    SHARED.lock().live.clone()
}

/// The phone's screen time today: minutes and the top apps.
pub fn phone_screen_time() -> Option<(u32, Vec<(String, u32)>)> {
    SHARED.lock().phone_screen.clone()
}

/// Answer, decline, hang up or mute the phone's call.
pub fn call_action(action: &str) -> bool {
    send(json!({"t": "call_action", "a": action}))
}

/// Replies to a phone notification through its own reply box.
pub fn reply(key: &str, text: &str) -> bool {
    send(json!({"t": "reply", "key": key, "text": text}))
}

/// An answer to a question the phone asked.
pub fn send_answer(question: &str, answer: &str) {
    send(json!({"t": "answer", "q": question, "text": answer}));
}

#[derive(Default)]
struct ContextState {
    meeting: Option<(String, i64)>,
    minutes: Option<u64>,
    sent_minutes_at: Option<Instant>,
    online: Option<bool>,
}

static CONTEXT: Mutex<ContextState> = Mutex::new(ContextState {
    meeting: None,
    minutes: None,
    sent_minutes_at: None,
    online: None,
});

/// What else the PC knows, mirrored to the phone when it changes: the next meeting (for a
/// Fluid Cloud countdown with Join), the PC's screen time today, and whether it is online.
pub fn publish_context(
    meeting: Option<(String, i64, Option<String>)>,
    screen_minutes: Option<u64>,
    online: bool,
) {
    if SHARED.lock().outbox.is_none() {
        *CONTEXT.lock() = ContextState::default();
        return;
    }
    let mut context = CONTEXT.lock();
    let key = meeting.as_ref().map(|(title, until, _)| (title.clone(), *until));
    if key != context.meeting {
        context.meeting = key;
        match &meeting {
            Some((title, until, link)) => {
                let starts = epoch_ms() as i64 + until * 60_000;
                send(json!({"t": "next_meeting", "title": title, "starts": starts, "link": link}));
            }
            None => {
                send(json!({"t": "next_meeting", "title": ""}));
            }
        }
    }
    if context
        .sent_minutes_at
        .is_none_or(|at| at.elapsed() > Duration::from_secs(300))
        && screen_minutes != context.minutes
    {
        context.minutes = screen_minutes;
        context.sent_minutes_at = Some(Instant::now());
        send(json!({"t": "pc_screen_time", "minutes": screen_minutes}));
    }
    if context.online != Some(online) {
        let was = context.online;
        context.online = Some(online);
        // Only after the PC was online and dropped: the phone offers its hotspot.
        if was == Some(true) && !online {
            send(json!({"t": "need_internet"}));
        }
    }
}

/// A friend's PC waved: the phone robot passes it on.
pub fn forward_wave(name: &str) {
    send(json!({"t": "wave", "name": name}));
}

/// A recap the phone asked for.
pub fn send_recap(text: &str, model: Option<&str>) {
    send(json!({"t": "recap", "text": text, "model": model}));
}

/// Makes the phone ring at full volume, even on silent.
pub fn ring_phone() -> bool {
    send(json!({"t": "ring"}))
}

/// Sends text to the phone's clipboard.
pub fn send_clip(text: &str) -> bool {
    if text.is_empty() || text.len() > 200_000 {
        return false;
    }
    send(json!({"t": "clip", "text": text}))
}

/// Asks the phone's player to go back, play/pause or skip.
pub fn phone_media_command(command: &str) -> bool {
    send(json!({"t": "phone_media", "cmd": command}))
}

/// A new clipboard text on the PC; sent on when clipboard sync is on.
pub fn publish_clip(text: &str) {
    {
        let mut shared = SHARED.lock();
        let on = shared
            .pairing
            .as_ref()
            .is_some_and(|pairing| pairing.clipboard_sync);
        if !on || shared.outbox.is_none() {
            return;
        }
        if shared.echo.as_deref() == Some(text) {
            shared.echo = None;
            return;
        }
    }
    send_clip(text);
}

/// The PC's now playing, mirrored to the phone.
pub fn publish_media(title: &str, artist: &str, playing: bool) {
    send(json!({"t": "media", "title": title, "artist": artist, "playing": playing}));
}

/// The Focus timer: `Some(seconds left)` while a session runs, else `None`.
pub fn publish_focus(remaining: Option<Duration>) {
    let until = remaining.map_or(0, |left| epoch_ms() + left.as_millis() as u64);
    send(json!({"t": "focus", "until": until}));
}

/// The robot's shared life: what it remembers on the PC, for the phone robot.
fn publish_robot() {
    use crate::core::mascot_memory::{self, Memory};
    let memory = Memory::load();
    let focus_minutes_today = memory
        .focus_days
        .iter()
        .find(|(day, _)| *day == memory.day)
        .map_or(0, |(_, seconds)| (*seconds / 60.0) as u32);
    send(json!({
        "t": "robot",
        "days": memory.active_days,
        "streak": memory.streak,
        "focus_today": memory.focus_sessions,
        "focus_minutes": focus_minutes_today,
        "breaks": memory.breaks,
        "copies": memory.copies,
        "unlocked": mascot_memory::unlocked_count(memory.active_days),
        "plant": mascot_memory::plant_stage(memory.active_days),
    }));
}

fn publish_presence(away: bool) {
    let battery = crate::platform::battery();
    send(json!({
        "t": "pc",
        "away": away,
        "battery": battery.map(|(percent, _)| percent),
        "charging": battery.is_some_and(|(_, charging)| charging),
    }));
}

#[derive(Default)]
struct PollState {
    sent_media: Option<(String, String, bool)>,
    sent_focus: Option<u64>,
    away: bool,
    presence_at: Option<Instant>,
    gone_since: Option<Instant>,
    locked: bool,
    meeting: bool,
    meeting_at: Option<Instant>,
    away_since: Option<u64>,
    robot_at: Option<Instant>,
}

thread_local! {
    static POLL: std::cell::RefCell<PollState> = std::cell::RefCell::new(PollState::default());
}

/// Called every frame on the UI thread: starts the server once paired and returns the
/// events that arrived. `media` is the PC's now playing; `focus` the Focus time left.
pub fn poll(now: Instant, media: Option<(&str, &str, bool)>, focus: Option<Duration>) -> Vec<Event> {
    load();
    if SHARED.lock().pairing.is_some() {
        ensure_started();
    }
    let connected = SHARED.lock().outbox.is_some();
    POLL.with(|cell| {
        let mut state = cell.borrow_mut();
        if !connected {
            state.sent_media = None;
            state.sent_focus = None;
            state.presence_at = None;
            state.meeting = false;
            state.meeting_at = None;
            state.robot_at = None;
            let lock_on_leave = SHARED
                .lock()
                .pairing
                .as_ref()
                .is_some_and(|pairing| pairing.lock_on_leave && pairing.device.is_some());
            let was_seen = SHARED.lock().last_seen.is_some();
            let gone = *state.gone_since.get_or_insert(now);
            if lock_on_leave
                && was_seen
                && !state.locked
                && now.saturating_duration_since(gone) >= LEAVE_GRACE
            {
                state.locked = true;
                log::info!("Phone left: locking the PC");
                let _ = std::process::Command::new("rundll32.exe")
                    .arg("user32.dll,LockWorkStation")
                    .spawn();
            }
        } else {
            state.gone_since = None;
            state.locked = false;
            let media = media
                .filter(|(title, _, _)| !title.is_empty())
                .map(|(title, artist, playing)| (title.to_string(), artist.to_string(), playing));
            if media != state.sent_media {
                match &media {
                    Some((title, artist, playing)) => publish_media(title, artist, *playing),
                    None => publish_media("", "", false),
                }
                state.sent_media = media;
            }
            // Whole seconds only change the sent end time when the timer was paused or moved.
            let until = focus.map(|left| (epoch_ms() + left.as_millis() as u64) / 2000);
            if until != state.sent_focus {
                publish_focus(focus);
                state.sent_focus = until;
            }
            // One robot, two bodies: share its life with the phone every minute.
            if state
                .robot_at
                .is_none_or(|at| now.saturating_duration_since(at) > Duration::from_secs(60))
            {
                state.robot_at = Some(now);
                publish_robot();
            }
            // Meeting mode: the camera or mic is in use, so the phone goes quiet too.
            if state
                .meeting_at
                .is_none_or(|at| now.saturating_duration_since(at) > Duration::from_secs(5))
            {
                state.meeting_at = Some(now);
                let (camera, microphone) = winisland_platform_windows::capture_in_use();
                let meeting = camera || microphone;
                if meeting != state.meeting {
                    state.meeting = meeting;
                    crate::core::memory::record(
                        crate::core::memory::Kind::Meeting,
                        if meeting { "Meeting started" } else { "Meeting ended" },
                        if camera { "Camera on" } else { "Microphone on" },
                        "",
                    );
                    send(json!({"t": "meeting", "on": meeting, "camera": camera}));
                }
            }
            let away = idle_for().is_some_and(|idle| idle >= AWAY_AFTER);
            // Back at the PC after a while: the robot recaps what the phone got meanwhile.
            if away && !state.away {
                state.away_since = Some(
                    crate::core::memory::now_ms().saturating_sub(AWAY_AFTER.as_millis() as u64),
                );
            }
            if !away && state.away
                && let Some(since) = state.away_since.take()
            {
                let missed = crate::core::memory::since(since)
                    .iter()
                    .filter(|entry| entry.kind == crate::core::memory::Kind::PhoneAlert)
                    .count();
                if missed >= 2 {
                    crate::core::recap::request(since, crate::core::recap::Reason::Back);
                }
            }
            if away != state.away
                || state
                    .presence_at
                    .is_none_or(|at| now.saturating_duration_since(at) > Duration::from_secs(60))
            {
                state.away = away;
                state.presence_at = Some(now);
                publish_presence(away);
            }
        }
    });
    std::mem::take(&mut SHARED.lock().events)
}

fn idle_for() -> Option<Duration> {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: `info` is a valid, sized LASTINPUTINFO.
    if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        return None;
    }
    // SAFETY: no arguments.
    let ticks = unsafe { GetTickCount() };
    Some(Duration::from_millis(u64::from(ticks.wrapping_sub(info.dwTime))))
}

// ---- Server ----------------------------------------------------------------------------

fn ensure_started() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        STARTED.store(false, Ordering::SeqCst);
        return;
    };
    handle.spawn(async {
        let listener = match TcpListener::bind(("0.0.0.0", PORT)).await {
            Ok(listener) => listener,
            Err(_) => match TcpListener::bind(("0.0.0.0", 0)).await {
                Ok(listener) => listener,
                Err(error) => {
                    log::warn!("Notchy Link cannot listen: {error}");
                    return;
                }
            },
        };
        let port = listener.local_addr().map_or(PORT, |address| address.port());
        SHARED.lock().port = port;
        log::info!("Notchy Link listening on port {port}");
        tokio::spawn(discovery(port));
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            };
            tokio::spawn(async move {
                if let Err(error) = session(stream, peer).await {
                    log::info!("Notchy Link session from {peer} ended: {error}");
                }
            });
        }
    });
}

async fn discovery(port: u16) {
    let socket = match UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)).await {
        Ok(socket) => socket,
        Err(error) => {
            log::warn!("Notchy Link discovery unavailable: {error}");
            return;
        }
    };
    let mut buffer = [0u8; 128];
    loop {
        let Ok((count, from)) = socket.recv_from(&mut buffer).await else {
            continue;
        };
        let Some(asked) = std::str::from_utf8(&buffer[..count])
            .ok()
            .and_then(|text| text.strip_prefix("NOTCHY?"))
        else {
            continue;
        };
        let ours = SHARED
            .lock()
            .pairing
            .as_ref()
            .and_then(|pairing| decode_key(&pairing.key))
            .map(|key| pair_id(&key));
        if ours.as_deref() == Some(asked.trim()) {
            let _ = socket.send_to(format!("NOTCHY!{port}").as_bytes(), from).await;
        }
    }
}

fn seal(cipher: &ChaCha20Poly1305, plain: &[u8]) -> Option<Vec<u8>> {
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).ok()?;
    let sealed = cipher.encrypt(Nonce::from_slice(&nonce), plain).ok()?;
    let length = (nonce.len() + sealed.len()) as u32;
    let mut frame = Vec::with_capacity(4 + length as usize);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&nonce);
    frame.extend_from_slice(&sealed);
    Some(frame)
}

async fn read_frame<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    cipher: &ChaCha20Poly1305,
) -> std::io::Result<Value> {
    let mut length = [0u8; 4];
    reader.read_exact(&mut length).await?;
    let length = u32::from_be_bytes(length) as usize;
    if !(12 + 16..=MAX_FRAME).contains(&length) {
        return Err(std::io::Error::other("bad frame length"));
    }
    let mut frame = vec![0u8; length];
    reader.read_exact(&mut frame).await?;
    let plain = cipher
        .decrypt(Nonce::from_slice(&frame[..12]), &frame[12..])
        .map_err(|_| std::io::Error::other("wrong key"))?;
    serde_json::from_slice(&plain).map_err(std::io::Error::other)
}

async fn session(stream: TcpStream, peer: SocketAddr) -> std::io::Result<()> {
    let key = SHARED
        .lock()
        .pairing
        .as_ref()
        .and_then(|pairing| decode_key(&pairing.key))
        .ok_or_else(|| std::io::Error::other("not paired"))?;
    let cipher = Arc::new(ChaCha20Poly1305::new(Key::from_slice(&key)));
    let _ = stream.set_nodelay(true);
    let (mut reader, mut writer) = stream.into_split();

    let mut challenge = [0u8; 16];
    getrandom::fill(&mut challenge).map_err(|_| std::io::Error::other("no randomness"))?;
    let challenge = hex(&challenge);
    let frame = seal(&cipher, json!({"t": "challenge", "c": challenge, "name": pc_name()}).to_string().as_bytes())
        .ok_or_else(|| std::io::Error::other("seal failed"))?;
    writer.write_all(&frame).await?;

    let hello = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut reader, &cipher))
        .await
        .map_err(|_| std::io::Error::other("no hello"))??;
    if hello["t"] != "hello" || hello["c"].as_str() != Some(challenge.as_str()) {
        return Err(std::io::Error::other("bad hello"));
    }
    let name = hello["name"].as_str().unwrap_or("Phone").chars().take(60).collect::<String>();

    let (outbox, mut queue) = unbounded_channel::<String>();
    let generation = {
        let mut shared = SHARED.lock();
        shared.generation += 1;
        shared.outbox = Some(outbox);
        shared.connection = Some(Connection {
            name: name.clone(),
            battery: None,
        });
        shared.last_seen = Some(Instant::now());
        let first = shared
            .pairing
            .as_ref()
            .is_some_and(|pairing| pairing.device.is_none());
        if let Some(pairing) = shared.pairing.as_mut() {
            if pairing.device.as_deref() != Some(name.as_str()) {
                pairing.device = Some(name.clone());
                let pairing = pairing.clone();
                save(&pairing);
            }
        }
        shared.events.push(Event::Arrived);
        if first {
            shared.events.push(Event::Toast(
                "Phone paired".to_string(),
                format!("{name} is now linked to Notchy"),
            ));
        }
        changed(&mut shared);
        shared.generation
    };
    log::info!("Notchy Link: {name} connected from {peer}");

    let write_cipher = cipher.clone();
    let writer_task = tokio::spawn(async move {
        while let Some(message) = queue.recv().await {
            let Some(frame) = seal(&write_cipher, message.as_bytes()) else {
                break;
            };
            if writer.write_all(&frame).await.is_err() {
                break;
            }
        }
    });
    send(json!({"t": "welcome", "name": pc_name()}));

    let result = loop {
        if SHARED.lock().generation != generation {
            break Ok(());
        }
        let message = match tokio::time::timeout(IDLE_TIMEOUT, read_frame(&mut reader, &cipher)).await {
            Ok(Ok(message)) => message,
            Ok(Err(error)) => break Err(error),
            Err(_) => break Err(std::io::Error::other("timed out")),
        };
        SHARED.lock().last_seen = Some(Instant::now());
        handle(&message);
    };

    writer_task.abort();
    let mut shared = SHARED.lock();
    if shared.generation == generation {
        shared.outbox = None;
        shared.connection = None;
        shared.media = None;
        shared.call = None;
        shared.live.clear();
        changed(&mut shared);
    }
    result
}

fn text(message: &Value, field: &str, limit: usize) -> String {
    message[field]
        .as_str()
        .unwrap_or_default()
        .chars()
        .take(limit)
        .collect()
}

/// One message from the phone. Runs on the network task; UI work is queued.
fn handle(message: &Value) {
    match message["t"].as_str().unwrap_or_default() {
        "ping" => {
            send(json!({"t": "pong"}));
        }
        "battery" => {
            let mut shared = SHARED.lock();
            let percent = message["pct"].as_u64().unwrap_or(0).min(100) as u8;
            let charging = message["charging"].as_bool().unwrap_or(false);
            let before = shared.connection.as_ref().and_then(|connection| connection.battery);
            if let Some(connection) = shared.connection.as_mut() {
                connection.battery = Some((percent, charging));
            }
            // Battery buddy: say so once when the phone drops to 20% or 10%.
            let crossed = |level: u8| {
                percent <= level && before.is_some_and(|(previous, _)| previous > level)
            };
            if !charging && (crossed(20) || crossed(10)) {
                shared.events.push(Event::PhoneLow(percent));
            }
            changed(&mut shared);
        }
        "file_start" => {
            let name = sanitize_name(&text(message, "name", 120));
            let size = message["size"].as_u64().unwrap_or(0);
            let id = message["id"].as_u64().unwrap_or(0);
            if size > MAX_FILE {
                send(json!({"t": "file_error", "id": id, "error": "too large"}));
                return;
            }
            let folder = crate::core::drop_server::received_folder();
            let _ = std::fs::create_dir_all(&folder);
            let path = unique(&folder, &name);
            match std::fs::File::create(&path) {
                Ok(file) => {
                    *INCOMING.lock() = Some(Incoming {
                        id,
                        path,
                        file,
                        expected: size,
                        written: 0,
                    });
                }
                Err(error) => {
                    send(json!({"t": "file_error", "id": id, "error": error.to_string()}));
                }
            }
        }
        "file_chunk" => {
            use base64::Engine;
            use std::io::Write;
            let mut incoming = INCOMING.lock();
            let Some(current) = incoming.as_mut() else {
                return;
            };
            if message["id"].as_u64() != Some(current.id) {
                return;
            }
            let Ok(bytes) = base64::engine::general_purpose::STANDARD
                .decode(message["data"].as_str().unwrap_or_default())
            else {
                return;
            };
            if current.written + bytes.len() as u64 > current.expected
                || current.file.write_all(&bytes).is_err()
            {
                let path = current.path.clone();
                let id = current.id;
                *incoming = None;
                let _ = std::fs::remove_file(path);
                send(json!({"t": "file_error", "id": id, "error": "write failed"}));
                return;
            }
            current.written += bytes.len() as u64;
        }
        "file_end" => {
            let Some(done) = INCOMING.lock().take() else {
                return;
            };
            drop(done.file);
            if done.written == done.expected {
                send(json!({"t": "file_ok", "id": done.id}));
                let years = PHOTO_YEARS
                    .lock()
                    .iter()
                    .find(|(id, _)| *id == done.id)
                    .map(|(_, years)| *years);
                let mut shared = SHARED.lock();
                shared.events.push(match years {
                    Some(years) => Event::Photo(done.path, years),
                    None => Event::File(done.path),
                });
                changed(&mut shared);
            } else {
                let _ = std::fs::remove_file(&done.path);
                send(json!({"t": "file_error", "id": done.id, "error": "incomplete"}));
            }
        }
        "notif" => {
            let notification = PhoneNotification {
                key: text(message, "key", 200),
                can_reply: message["reply"].as_bool().unwrap_or(false),
                app: text(message, "app", 40),
                title: text(message, "title", 120),
                text: text(message, "text", 400),
                otp: None,
                at: Instant::now(),
            };
            let otp = find_otp(&format!("{} {}", notification.title, notification.text));
            let notification = PhoneNotification { otp, ..notification };
            // Codes are never kept in Memory. Queued alerts happened while the PC was away:
            // keep their time and don't pop them up now.
            let queued = message["queued"].as_bool().unwrap_or(false);
            let at = message["at"].as_u64().unwrap_or_else(crate::core::memory::now_ms);
            crate::core::memory::record_at(
                at,
                crate::core::memory::Kind::PhoneAlert,
                notification.title.clone(),
                if notification.otp.is_some() { "A one-time code".to_string() } else { notification.text.clone() },
                notification.app.clone(),
            );
            let mut shared = SHARED.lock();
            let alerts = shared.pairing.as_ref().is_some_and(|pairing| pairing.alerts) && !queued;
            if queued {
                // Shown in the list and the timeline only.
            } else if let Some(code) = &notification.otp {
                shared.echo = Some(code.clone());
                shared.events.push(Event::Clip(code.clone()));
                shared.events.push(Event::Toast(
                    format!("Code {code} copied"),
                    format!("From {} on your phone", notification.app),
                ));
            } else if alerts {
                let title = if notification.title.is_empty() {
                    notification.app.clone()
                } else {
                    format!("{} · {}", notification.app, notification.title)
                };
                shared.events.push(Event::Toast(title, notification.text.clone()));
            }
            shared.notifications.push(notification);
            if shared.notifications.len() > MAX_NOTIFICATIONS {
                shared.notifications.remove(0);
            }
            changed(&mut shared);
        }
        "clip" => {
            let clip = text(message, "text", 200_000);
            if !clip.is_empty() {
                crate::core::memory::record(crate::core::memory::Kind::Clip, "From phone", clip.clone(), "");
                let mut shared = SHARED.lock();
                shared.echo = Some(clip.clone());
                shared.events.push(Event::Clip(clip));
                changed(&mut shared);
            }
        }
        "url" => {
            let url = text(message, "url", 4000);
            if url.starts_with("http://") || url.starts_with("https://") {
                let mut shared = SHARED.lock();
                shared.events.push(Event::OpenUrl(url));
                changed(&mut shared);
            }
        }
        "media_cmd" => {
            let command = text(message, "cmd", 10);
            if matches!(command.as_str(), "prev" | "toggle" | "next") {
                let mut shared = SHARED.lock();
                shared.events.push(Event::Media(command));
                changed(&mut shared);
            }
        }
        "phone_media" => {
            let mut shared = SHARED.lock();
            let title = text(message, "title", 120);
            shared.media = (!title.is_empty()).then(|| PhoneMedia {
                title,
                artist: text(message, "artist", 120),
                playing: message["playing"].as_bool().unwrap_or(false),
            });
            changed(&mut shared);
        }
        "clip_history" => {
            let items: Vec<String> = HISTORY
                .lock()
                .iter()
                .take(40)
                .map(|item| item.chars().take(2000).collect())
                .collect();
            send(json!({"t": "clip_history", "items": items}));
        }
        "recap" => {
            let hours = message["hours"].as_u64().unwrap_or(3).clamp(1, 48);
            let since = crate::core::memory::now_ms().saturating_sub(hours * 3_600_000);
            crate::core::recap::request(since, crate::core::recap::Reason::Phone);
        }
        "memory_search" => {
            let query = text(message, "q", 80);
            let items: Vec<Value> = crate::core::memory::search(&query, 60)
                .into_iter()
                .map(|entry| {
                    json!({
                        "at": entry.at,
                        "kind": entry.kind.label(),
                        "title": entry.title,
                        "detail": entry.detail.chars().take(200).collect::<String>(),
                        "app": entry.app,
                    })
                })
                .collect();
            send(json!({"t": "memory", "items": items}));
        }
        "ask" => {
            let question = text(message, "q", 400);
            if !question.is_empty() {
                crate::core::assistant::ask(question, crate::core::assistant::Asker::Phone);
            }
        }
        "call" => {
            let mut shared = SHARED.lock();
            let name = text(message, "name", 80);
            let state = text(message, "state", 20);
            let ringing_now = state == "ringing" && shared.call.as_ref().is_none_or(|call| call.state != "ringing");
            shared.call = Some(Call {
                name: name.clone(),
                state,
                since: shared.call.as_ref().map_or_else(Instant::now, |call| call.since),
                actions: message["actions"]
                    .as_array()
                    .map(|actions| actions.iter().filter_map(|action| action.as_str().map(str::to_string)).take(4).collect())
                    .unwrap_or_default(),
            });
            if ringing_now {
                shared.events.push(Event::Toast("Phone is ringing".to_string(), name.clone()));
                crate::core::memory::record(crate::core::memory::Kind::PhoneAlert, name, "Incoming call", "Phone");
            }
            changed(&mut shared);
        }
        "call_end" => {
            let mut shared = SHARED.lock();
            shared.call = None;
            changed(&mut shared);
        }
        "live" => {
            let item = LiveItem {
                key: text(message, "key", 200),
                app: text(message, "app", 40),
                title: text(message, "title", 120),
                text: text(message, "text", 200),
                progress: message["progress"].as_u64().map(|value| value.min(100) as u8),
            };
            let mut shared = SHARED.lock();
            match shared.live.iter_mut().find(|known| known.key == item.key) {
                Some(known) => *known = item,
                None => {
                    crate::core::memory::record(crate::core::memory::Kind::PhoneAlert, item.title.clone(), item.text.clone(), item.app.clone());
                    shared.live.push(item);
                    if shared.live.len() > 4 {
                        shared.live.remove(0);
                    }
                }
            }
            changed(&mut shared);
        }
        "live_end" => {
            let key = text(message, "key", 200);
            let mut shared = SHARED.lock();
            shared.live.retain(|item| item.key != key);
            changed(&mut shared);
        }
        "screen_time" => {
            let minutes = message["minutes"].as_u64().unwrap_or(0) as u32;
            let top = message["top"]
                .as_array()
                .map(|apps| {
                    apps.iter()
                        .filter_map(|app| Some((app[0].as_str()?.chars().take(30).collect(), app[1].as_u64()? as u32)))
                        .take(5)
                        .collect()
                })
                .unwrap_or_default();
            let mut shared = SHARED.lock();
            shared.phone_screen = Some((minutes, top));
            changed(&mut shared);
        }
        "note" => {
            let note = text(message, "text", 600);
            if !note.is_empty() {
                crate::core::memory::record(crate::core::memory::Kind::Note, "Note from phone", note.clone(), "");
                // Shared notes: it also lands in the PC's Notes tab.
                let mut shared = SHARED.lock();
                shared.events.push(Event::Note(note));
                changed(&mut shared);
            }
        }
        "notes_list" => {
            let mut shared = SHARED.lock();
            shared.events.push(Event::NotesWanted);
            changed(&mut shared);
        }
        "expense" => {
            let amount = message["amount"].as_f64().unwrap_or(0.0);
            let merchant = text(message, "merchant", 80);
            crate::core::memory::record(
                crate::core::memory::Kind::Expense,
                format!("₹{amount:.2} · {}", if merchant.is_empty() { "payment" } else { &merchant }),
                text(message, "text", 300),
                text(message, "app", 40),
            );
        }
        "photo_memory" => {
            let id = message["id"].as_u64().unwrap_or(0);
            let years = message["years"].as_u64().unwrap_or(1) as u32;
            let mut photos = PHOTO_YEARS.lock();
            photos.push((id, years));
            if photos.len() > 5 {
                photos.remove(0);
            }
        }
        "wave_friends" => {
            for friend in crate::core::friends::nearby() {
                crate::core::friends::wave(friend.address);
            }
        }
        "want_file" => send_offered(message["id"].as_u64().unwrap_or(0)),
        "pet" => {
            let mut shared = SHARED.lock();
            shared.events.push(Event::Petted);
            changed(&mut shared);
        }
        "throw" => {
            let mut shared = SHARED.lock();
            shared.events.push(Event::Thrown);
            changed(&mut shared);
        }
        "find_pc" => {
            let mut shared = SHARED.lock();
            shared.events.push(Event::Ring);
            changed(&mut shared);
        }
        "lock_pc" => {
            let _ = std::process::Command::new("rundll32.exe")
                .arg("user32.dll,LockWorkStation")
                .spawn();
        }
        "mouse" => {
            let dx = message["dx"].as_f64().unwrap_or(0.0).clamp(-400.0, 400.0) as i32;
            let dy = message["dy"].as_f64().unwrap_or(0.0).clamp(-400.0, 400.0) as i32;
            remote::mouse_move(dx, dy);
        }
        "click" => remote::click(message["b"].as_str() == Some("right")),
        "scroll" => {
            remote::scroll(message["d"].as_f64().unwrap_or(0.0).clamp(-20.0, 20.0) as i32);
        }
        "key" => remote::key(message["k"].as_str().unwrap_or_default()),
        "type" => remote::type_text(&text(message, "text", 2000)),
        _ => {}
    }
}

fn sanitize_name(name: &str) -> String {
    let name: String = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("file")
        .chars()
        .filter(|character| !"<>:\"|?*".contains(*character) && !character.is_control())
        .collect();
    let name = name.trim().trim_start_matches('.');
    if name.is_empty() { "file".to_string() } else { name.to_string() }
}

fn unique(folder: &std::path::Path, name: &str) -> std::path::PathBuf {
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) => (stem.to_string(), format!(".{extension}")),
        None => (name.to_string(), String::new()),
    };
    let mut candidate = folder.join(name);
    let mut counter = 2;
    while candidate.exists() {
        candidate = folder.join(format!("{stem} ({counter}){extension}"));
        counter += 1;
    }
    candidate
}

/// A 4–8 digit code in a message that talks about codes or passwords.
fn find_otp(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    const WORDS: [&str; 9] = [
        "otp", "code", "verification", "verify", "password", "passcode", "pin", "one-time", "2fa",
    ];
    if !WORDS.iter().any(|word| lower.contains(word)) {
        return None;
    }
    let mut digits = String::new();
    let mut found = None;
    for character in text.chars().chain(std::iter::once(' ')) {
        if character.is_ascii_digit() {
            digits.push(character);
        } else {
            if (4..=8).contains(&digits.len()) {
                found = Some(digits.clone());
                break;
            }
            digits.clear();
        }
    }
    found
}

/// The phone as a touchpad and keyboard: `SendInput` from the network task.
mod remote {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
        KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSE_EVENT_FLAGS, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
        MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput, VIRTUAL_KEY, VK_DOWN, VK_ESCAPE, VK_LEFT,
        VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_RETURN, VK_RIGHT,
        VK_SPACE, VK_UP, VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP, VK_BACK, VK_TAB,
    };

    fn mouse(flags: MOUSE_EVENT_FLAGS, dx: i32, dy: i32, data: i32) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: data as _,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn keyboard(key: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn send(inputs: &[INPUT]) {
        // SAFETY: the slice holds fully initialised INPUT values.
        unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    }

    pub fn mouse_move(dx: i32, dy: i32) {
        send(&[mouse(MOUSEEVENTF_MOVE, dx, dy, 0)]);
    }

    pub fn click(right: bool) {
        let (down, up) = if right {
            (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)
        } else {
            (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)
        };
        send(&[mouse(down, 0, 0, 0), mouse(up, 0, 0, 0)]);
    }

    pub fn scroll(lines: i32) {
        send(&[mouse(MOUSEEVENTF_WHEEL, 0, 0, lines * 120)]);
    }

    pub fn key(name: &str) {
        let key = match name {
            "vol_up" => VK_VOLUME_UP,
            "vol_down" => VK_VOLUME_DOWN,
            "mute" => VK_VOLUME_MUTE,
            "play" => VK_MEDIA_PLAY_PAUSE,
            "next" => VK_MEDIA_NEXT_TRACK,
            "prev" => VK_MEDIA_PREV_TRACK,
            "left" => VK_LEFT,
            "right" => VK_RIGHT,
            "up" => VK_UP,
            "down" => VK_DOWN,
            "space" => VK_SPACE,
            "esc" => VK_ESCAPE,
            "enter" => VK_RETURN,
            "back" => VK_BACK,
            "tab" => VK_TAB,
            _ => return,
        };
        send(&[
            keyboard(key, 0, KEYBD_EVENT_FLAGS(0)),
            keyboard(key, 0, KEYEVENTF_KEYUP),
        ]);
    }

    pub fn type_text(text: &str) {
        let inputs: Vec<INPUT> = text
            .encode_utf16()
            .flat_map(|unit| {
                [
                    keyboard(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE),
                    keyboard(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
                ]
            })
            .collect();
        if !inputs.is_empty() {
            send(&inputs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_codes_only_in_code_messages() {
        assert_eq!(find_otp("Your OTP is 482913. Do not share."), Some("482913".into()));
        assert_eq!(find_otp("G-552811 is your Google verification code"), Some("552811".into()));
        assert_eq!(find_otp("Meet at 1930 near gate 4"), None);
    }

    #[test]
    fn frames_round_trip() {
        let key = [7u8; 32];
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
        let frame = seal(&cipher, br#"{"t":"ping"}"#).unwrap();
        let length = u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize;
        assert_eq!(length, frame.len() - 4);
        let plain = cipher
            .decrypt(Nonce::from_slice(&frame[4..16]), &frame[16..])
            .unwrap();
        assert_eq!(plain, br#"{"t":"ping"}"#);
    }

    #[test]
    fn keys_round_trip() {
        let key = [0xabu8; 32];
        assert_eq!(decode_key(&hex(&key)), Some(key));
        assert_eq!(decode_key("zz"), None);
    }
}
