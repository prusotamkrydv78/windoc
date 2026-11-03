//! Notchy Memory: a small local timeline of what crossed between the PC and the phone.
//!
//! One JSON line per entry in `~/.winisland/memory/YYYY-MM.jsonl` (append-only, easy to read
//! and to delete). The newest few thousand entries stay in memory for search. Files older than
//! three months are deleted on start. Nothing leaves the PC; one-time codes are never stored.

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

const KEEP_IN_MEMORY: usize = 4000;
const KEEP_MONTHS: i64 = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A notification from the phone.
    PhoneAlert,
    /// Text that came from the phone's clipboard.
    Clip,
    /// A file from the phone.
    FileIn,
    /// A file sent to the phone.
    FileOut,
    Download,
    Focus,
    Meeting,
    /// A recap the robot wrote.
    Recap,
    /// A quick note from the phone.
    Note,
}

impl Kind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::PhoneAlert => "Alert",
            Self::Clip => "Clip",
            Self::FileIn => "File in",
            Self::FileOut => "File out",
            Self::Download => "Download",
            Self::Focus => "Focus",
            Self::Meeting => "Meeting",
            Self::Recap => "Recap",
            Self::Note => "Note",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    /// Unix time in milliseconds.
    pub at: u64,
    pub kind: Kind,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// The phone app a notification came from.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub app: String,
}

struct Store {
    entries: Vec<Entry>,
    revision: u64,
}

static STORE: Mutex<Store> = Mutex::new(Store {
    entries: Vec::new(),
    revision: 0,
});
static LOADED: AtomicBool = AtomicBool::new(false);

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_millis() as u64)
}

fn folder() -> PathBuf {
    crate::platform::shell().data_dir().join("memory")
}

/// `(year, month)` of a Unix time in milliseconds (UTC; close enough for file names).
fn year_month(ms: u64) -> (i64, i64) {
    let days = (ms / 86_400_000) as i64;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month)
}

fn file_for(ms: u64) -> PathBuf {
    let (year, month) = year_month(ms);
    folder().join(format!("{year:04}-{month:02}.jsonl"))
}

fn load() {
    if LOADED.swap(true, Ordering::SeqCst) {
        return;
    }
    let (year, month) = year_month(now_ms());
    let oldest = year * 12 + month - KEEP_MONTHS;
    let mut files: Vec<(i64, PathBuf)> = std::fs::read_dir(folder())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|item| {
            let path = item.path();
            let stem = path.file_stem()?.to_str()?.to_string();
            let (y, m) = stem.split_once('-')?;
            Some((y.parse::<i64>().ok()? * 12 + m.parse::<i64>().ok()?, path))
        })
        .collect();
    files.sort();
    let mut entries = Vec::new();
    for (index, path) in files {
        if index < oldest {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            if let Ok(entry) = serde_json::from_str::<Entry>(&line) {
                entries.push(entry);
            }
        }
    }
    if entries.len() > KEEP_IN_MEMORY {
        entries.drain(..entries.len() - KEEP_IN_MEMORY);
    }
    let mut store = STORE.lock();
    entries.append(&mut store.entries);
    store.entries = entries;
    store.revision += 1;
}

/// Adds an entry (and appends it to this month's file).
pub fn record(kind: Kind, title: impl Into<String>, detail: impl Into<String>, app: impl Into<String>) {
    record_at(now_ms(), kind, title, detail, app);
}

/// [`record`] with the time it happened (alerts the phone kept while the PC was away).
pub fn record_at(at: u64, kind: Kind, title: impl Into<String>, detail: impl Into<String>, app: impl Into<String>) {
    load();
    let entry = Entry {
        at: at.min(now_ms()),
        kind,
        title: title.into().chars().take(200).collect(),
        detail: detail.into().chars().take(600).collect(),
        app: app.into().chars().take(60).collect(),
    };
    crate::core::memory_sql::push(&entry);
    if let Ok(line) = serde_json::to_string(&entry) {
        let path = file_for(entry.at);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(file, "{line}");
        }
    }
    let mut store = STORE.lock();
    store.entries.push(entry);
    store.entries.sort_by_key(|entry| entry.at);
    if store.entries.len() > KEEP_IN_MEMORY {
        store.entries.remove(0);
    }
    store.revision += 1;
    drop(store);
    crate::platform::wake();
}

/// Rows loaded from SQL Server: adds the ones this PC does not have yet.
pub fn merge(rows: Vec<Entry>) {
    load();
    let mut store = STORE.lock();
    let known: std::collections::HashSet<(u64, String)> = store
        .entries
        .iter()
        .map(|entry| (entry.at, entry.title.clone()))
        .collect();
    store
        .entries
        .extend(rows.into_iter().filter(|row| !known.contains(&(row.at, row.title.clone()))));
    store.entries.sort_by_key(|entry| entry.at);
    if store.entries.len() > KEEP_IN_MEMORY {
        let excess = store.entries.len() - KEEP_IN_MEMORY;
        store.entries.drain(..excess);
    }
    store.revision += 1;
}

/// Bumps on every change, for redraws.
pub fn revision() -> u64 {
    load();
    STORE.lock().revision
}

/// Newest first, matching every word of `query` (case-insensitive) when it is not empty.
pub fn search(query: &str, limit: usize) -> Vec<Entry> {
    load();
    let words: Vec<String> = query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    STORE
        .lock()
        .entries
        .iter()
        .rev()
        .filter(|entry| {
            words.is_empty() || {
                let haystack = format!(
                    "{} {} {} {}",
                    entry.title,
                    entry.detail,
                    entry.app,
                    entry.kind.label()
                )
                .to_lowercase();
                words.iter().all(|word| haystack.contains(word))
            }
        })
        .take(limit)
        .cloned()
        .collect()
}

/// Entries since `ms`, oldest first.
pub fn since(ms: u64) -> Vec<Entry> {
    load();
    STORE
        .lock()
        .entries
        .iter()
        .filter(|entry| entry.at >= ms)
        .cloned()
        .collect()
}

/// Deletes every memory file and entry.
pub fn forget_all() {
    let _ = std::fs::remove_dir_all(folder());
    let mut store = STORE.lock();
    store.entries.clear();
    store.revision += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_from_unix_time() {
        assert_eq!(year_month(0), (1970, 1));
        // 2026-11-02T12:00:00Z
        assert_eq!(year_month(1_793_620_800_000), (2026, 11));
        // 2024-02-29T00:00:00Z
        assert_eq!(year_month(1_709_164_800_000), (2024, 2));
    }
}
