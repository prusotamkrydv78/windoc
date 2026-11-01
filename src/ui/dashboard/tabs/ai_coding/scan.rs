//! Reads local Claude Code and Codex logs for the AI coding tab, on a background thread.
//!
//! The app's `AiUsageService` keeps only per-day totals, so the tab reads the same logs
//! itself (only while the user's AI usage setting is on and the tab is open) to get what
//! the page shows: local hours, prompts, tool calls, projects and per-model costs. Like
//! the service, it opens logs read-only, reads only bytes appended since the last scan and
//! keeps only metadata: no prompt or response text is stored.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use winisland_core::ai_usage::{Tool, UsageEvent, estimate_cost_usd};

/// Logs untouched for longer than this are skipped (covers the current month).
const MAX_AGE: Duration = Duration::from_secs(35 * 24 * 60 * 60);
/// Days older than this are dropped from memory.
const KEEP_DAYS: i64 = 40;
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;
const DAY: i64 = 24 * 60 * 60;

/// One tool's activity on one local day.
#[derive(Clone, Debug, Default)]
pub struct DayStats {
    pub tokens: u64,
    /// Estimated cost in US dollars at API list prices.
    pub cost: f64,
    /// Prompts the user sent.
    pub messages: u32,
    pub tool_calls: u32,
    pub sessions: BTreeSet<String>,
    /// Tokens per local hour.
    pub hours: [u64; 24],
    /// Tokens and cost per model.
    pub models: BTreeMap<String, (u64, f64)>,
    /// The most recent project folder: `(unix seconds, name)`.
    pub project: Option<(i64, String)>,
}

impl DayStats {
    pub fn is_active(&self) -> bool {
        self.tokens > 0 || self.messages > 0
    }

    fn touch(&mut self, session: &str, project: &str, at: i64) {
        if !session.is_empty() {
            self.sessions.insert(session.to_string());
        }
        if !project.is_empty() && self.project.as_ref().is_none_or(|(seen, _)| *seen <= at) {
            self.project = Some((at, project.to_string()));
        }
    }
}

/// What the last finished scan found, keyed by `(local day number, tool)`.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub days: BTreeMap<(i64, Tool), DayStats>,
}

#[derive(Default)]
struct FileState {
    offset: u64,
    /// Codex: model, session and project from the session's context lines.
    model: String,
    session: String,
    project: String,
    /// Codex: last cumulative token total, to skip repeated `token_count` events.
    last_total: u64,
    /// Claude Code: message ids already counted (one response spans several lines).
    seen: HashSet<String>,
}

#[derive(Default)]
struct Scanner {
    files: HashMap<PathBuf, FileState>,
    days: BTreeMap<(i64, Tool), DayStats>,
}

static SCANNER: LazyLock<Mutex<Scanner>> = LazyLock::new(|| Mutex::new(Scanner::default()));
static PUBLISHED: Mutex<Option<Arc<Snapshot>>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Starts a scan on a background thread unless one is running. `offset` is the local
/// time zone's offset from UTC in seconds. `true` when a scan started.
pub fn start(offset: i64) -> bool {
    if RUNNING.swap(true, Ordering::AcqRel) {
        return false;
    }
    let spawned = std::thread::Builder::new()
        .name("ai-coding-scan".to_string())
        .spawn(move || {
            let snapshot = {
                let mut scanner = lock(&SCANNER);
                scanner.scan(offset);
                Arc::new(Snapshot {
                    days: scanner.days.clone(),
                })
            };
            *lock(&PUBLISHED) = Some(snapshot);
            RUNNING.store(false, Ordering::Release);
        });
    if let Err(error) = spawned {
        log::warn!("Cannot start the AI coding scan: {error}");
        RUNNING.store(false, Ordering::Release);
        return false;
    }
    true
}

/// The latest finished scan, or `None` before the first one.
pub fn snapshot() -> Option<Arc<Snapshot>> {
    lock(&PUBLISHED).clone()
}

pub fn scanning() -> bool {
    RUNNING.load(Ordering::Acquire)
}

/// Forgets everything read, for when the user turns AI usage off.
pub fn reset() {
    if RUNNING.load(Ordering::Acquire) {
        return;
    }
    let mut published = lock(&PUBLISHED);
    if published.is_none() {
        return;
    }
    *published = None;
    *lock(&SCANNER) = Scanner::default();
}

/// Seconds since the Unix epoch, now.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// The local time zone's offset from UTC in seconds, from the shell's local clock.
pub fn utc_offset() -> i64 {
    let local = crate::platform::shell().local_datetime();
    let local_seconds = days_from_civil(
        i64::from(local.year),
        i64::from(local.month),
        i64::from(local.day),
    ) * DAY
        + i64::from(local.hour) * 3600
        + i64::from(local.minute) * 60
        + i64::from(local.second);
    let offset = local_seconds - unix_now();
    // Zones are whole quarter hours; rounding hides the clock reads' small gap.
    (offset as f64 / 900.0).round() as i64 * 900
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
pub fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// `(year, month, day)` of a day number from [`days_from_civil`].
pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Unix seconds of an ISO 8601 UTC timestamp such as `2026-10-31T12:34:56.789Z`.
fn parse_timestamp(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[13] != b':' {
        return None;
    }
    let field = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let days = days_from_civil(field(0..4)?, field(5..7)?, field(8..10)?);
    Some(days * DAY + field(11..13)? * 3600 + field(14..16)? * 60 + field(17..19)?)
}

/// The last folder name of a path, for the project label.
fn folder_name(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .unwrap_or_default()
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn walk(root: &Path, files: &mut Vec<PathBuf>, depth: usize) {
    if depth > 5 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, files, depth + 1);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "jsonl")
        {
            files.push(path);
        }
    }
}

fn recently_modified(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age <= MAX_AGE)
}

impl Scanner {
    fn scan(&mut self, offset: i64) {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let mut claude = Vec::new();
        walk(&home.join(".claude").join("projects"), &mut claude, 0);
        let mut codex = Vec::new();
        walk(&home.join(".codex").join("sessions"), &mut codex, 0);
        for (path, tool) in claude
            .into_iter()
            .map(|path| (path, Tool::ClaudeCode))
            .chain(codex.into_iter().map(|path| (path, Tool::Codex)))
        {
            if recently_modified(&path) {
                self.read_new_lines(&path, tool, offset);
            }
        }
        let oldest = (unix_now() + offset).div_euclid(DAY) - KEEP_DAYS;
        self.days.retain(|(day, _), _| *day >= oldest);
    }

    fn read_new_lines(&mut self, path: &Path, tool: Tool, offset: i64) {
        let Ok(mut file) = File::open(path) else {
            return;
        };
        let state = self.files.entry(path.to_path_buf()).or_default();
        let length = file.metadata().map_or(0, |metadata| metadata.len());
        if length < state.offset {
            // The log was truncated or replaced; read it again from the start.
            *state = FileState::default();
        }
        if length == state.offset || file.seek(SeekFrom::Start(state.offset)).is_err() {
            return;
        }
        if state.session.is_empty() {
            state.session = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            let Ok(read) = reader.read_line(&mut line) else {
                break;
            };
            // Stop at a partial last line; it is read again on the next scan.
            if read == 0 || !line.ends_with('\n') {
                break;
            }
            state.offset += read as u64;
            if line.len() > MAX_LINE_BYTES {
                continue;
            }
            match tool {
                Tool::ClaudeCode => claude_line(&line, state, &mut self.days, offset),
                Tool::Codex => codex_line(&line, state, &mut self.days, offset),
            }
        }
    }
}

/// Local day number and hour of a log line's timestamp.
fn local_time(value: &Value, offset: i64) -> Option<(i64, usize, i64)> {
    let at = parse_timestamp(value.get("timestamp")?.as_str()?)?;
    let local = at + offset;
    Some((
        local.div_euclid(DAY),
        (local.rem_euclid(DAY) / 3600) as usize,
        at,
    ))
}

fn claude_line(
    line: &str,
    state: &mut FileState,
    days: &mut BTreeMap<(i64, Tool), DayStats>,
    offset: i64,
) {
    // Cheap filter first: most lines are tool results the tab does not need.
    let is_assistant = line.contains("\"type\":\"assistant\"");
    let is_user = line.contains("\"type\":\"user\"") && !line.contains("\"tool_result\"");
    if !is_assistant && !is_user {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let Some((day, hour, at)) = local_time(&value, offset) else {
        return;
    };
    let session = value
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or(state.session.as_str())
        .to_string();
    let project = value
        .get("cwd")
        .and_then(Value::as_str)
        .map(folder_name)
        .unwrap_or_default();
    let Some(message) = value.get("message") else {
        return;
    };
    match value.get("type").and_then(Value::as_str) {
        Some("user") => {
            let flagged = |key: &str| value.get(key).and_then(Value::as_bool) == Some(true);
            if flagged("isMeta") || flagged("isSidechain") {
                return;
            }
            let prompt = match message.get("content") {
                Some(Value::String(text)) => {
                    !text.trim().is_empty() && !text.starts_with("<local-command")
                }
                Some(Value::Array(items)) => items
                    .iter()
                    .any(|item| item.get("type").and_then(Value::as_str) == Some("text")),
                _ => false,
            };
            if prompt {
                let stats = days.entry((day, Tool::ClaudeCode)).or_default();
                stats.messages += 1;
                stats.touch(&session, project, at);
            }
        }
        Some("assistant") => {
            let model = message
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if model.starts_with('<') {
                // Synthetic messages written by the client, not a model response.
                return;
            }
            let tool_calls = message
                .get("content")
                .and_then(Value::as_array)
                .map_or(0, |items| {
                    items
                        .iter()
                        .filter(|item| item.get("type").and_then(Value::as_str) == Some("tool_use"))
                        .count() as u32
                });
            let stats = days.entry((day, Tool::ClaudeCode)).or_default();
            stats.tool_calls += tool_calls;
            stats.touch(&session, project, at);
            // A response is written as one line per content block, each repeating the usage.
            let id = message.get("id").and_then(Value::as_str).unwrap_or("");
            if !id.is_empty() && !state.seen.insert(id.to_string()) {
                return;
            }
            let Some(usage) = message.get("usage") else {
                return;
            };
            let event = UsageEvent {
                tool: Tool::ClaudeCode,
                date: String::new(),
                session,
                model: model.to_string(),
                input_tokens: number(usage, "input_tokens"),
                output_tokens: number(usage, "output_tokens"),
                cache_tokens: number(usage, "cache_read_input_tokens")
                    + number(usage, "cache_creation_input_tokens"),
            };
            add_usage(stats, &event, hour);
        }
        _ => {}
    }
}

fn codex_line(
    line: &str,
    state: &mut FileState,
    days: &mut BTreeMap<(i64, Tool), DayStats>,
    offset: i64,
) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let Some(payload) = value.get("payload") else {
        return;
    };
    let text = |value: &Value, key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|text| !text.is_empty())
    };
    match value.get("type").and_then(Value::as_str) {
        Some("session_meta") => {
            if let Some(cwd) = text(payload, "cwd") {
                state.project = folder_name(&cwd).to_string();
            }
            if let Some(id) = text(payload, "id") {
                state.session = id;
            }
            return;
        }
        Some("turn_context") => {
            if let Some(model) = text(payload, "model") {
                state.model = model;
            }
            if let Some(cwd) = text(payload, "cwd") {
                state.project = folder_name(&cwd).to_string();
            }
            return;
        }
        _ => {}
    }
    let Some((day, hour, at)) = local_time(&value, offset) else {
        return;
    };
    let kind = payload.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "user_message" => {
            let stats = days.entry((day, Tool::Codex)).or_default();
            stats.messages += 1;
            stats.touch(&state.session, &state.project, at);
        }
        "function_call" | "custom_tool_call" | "local_shell_call" => {
            let stats = days.entry((day, Tool::Codex)).or_default();
            stats.tool_calls += 1;
            stats.touch(&state.session, &state.project, at);
        }
        "token_count" => {
            let Some(info) = payload.get("info").filter(|info| !info.is_null()) else {
                return;
            };
            // The same count is sometimes written twice; the running total tells.
            let total = info
                .get("total_token_usage")
                .map_or(0, |usage| number(usage, "total_tokens"));
            if total != 0 && total == state.last_total {
                return;
            }
            state.last_total = total;
            let Some(usage) = info.get("last_token_usage") else {
                return;
            };
            // Codex counts cached input inside `input_tokens`.
            let cached = number(usage, "cached_input_tokens");
            let event = UsageEvent {
                tool: Tool::Codex,
                date: String::new(),
                session: state.session.clone(),
                model: if state.model.is_empty() {
                    "codex".to_string()
                } else {
                    state.model.clone()
                },
                input_tokens: number(usage, "input_tokens").saturating_sub(cached),
                output_tokens: number(usage, "output_tokens")
                    + number(usage, "reasoning_output_tokens"),
                cache_tokens: cached,
            };
            let stats = days.entry((day, Tool::Codex)).or_default();
            stats.touch(&state.session, &state.project, at);
            add_usage(stats, &event, hour);
        }
        _ => {}
    }
}

fn add_usage(stats: &mut DayStats, event: &UsageEvent, hour: usize) {
    let tokens = event.total_tokens();
    let cost = estimate_cost_usd(event);
    stats.tokens += tokens;
    stats.cost += cost;
    stats.hours[hour.min(23)] += tokens;
    let model = stats.models.entry(event.model.clone()).or_default();
    model.0 += tokens;
    model.1 += cost;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trip() {
        for days in [-1000, 0, 19_000, 20_757] {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(days_from_civil(year, month, day), days);
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
    }

    #[test]
    fn parses_timestamps() {
        assert_eq!(
            parse_timestamp("1970-01-02T01:00:05.000Z"),
            Some(DAY + 3605)
        );
        assert_eq!(parse_timestamp("nope"), None);
    }

    #[test]
    fn folder_names() {
        assert_eq!(folder_name("C:\\Users\\me\\design-tokens"), "design-tokens");
        assert_eq!(folder_name("/home/me/app/"), "app");
    }
}
