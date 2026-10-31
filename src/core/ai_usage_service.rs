//! Scans local Claude Code and Codex logs for usage metadata when the user opts in.
//!
//! Logs are read-only and read incrementally (only bytes appended since the last scan).
//! Only parsed metadata is kept in memory; nothing is written or sent.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use winisland_core::ai_usage::{self, UsageEvent, UsageReport};

const SCAN_INTERVAL: Duration = Duration::from_secs(60);
/// Files untouched for longer than this are skipped.
const MAX_AGE: Duration = Duration::from_secs(8 * 24 * 60 * 60);
/// A session written to within this window counts as running.
const ACTIVE_WINDOW: Duration = Duration::from_secs(2 * 60);
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
struct FileState {
    offset: u64,
    codex_model: String,
}

pub struct AiUsageService {
    files: HashMap<PathBuf, FileState>,
    events: Vec<UsageEvent>,
    report: UsageReport,
    active_sessions: usize,
    next_scan: Instant,
    revision: u64,
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

fn recently_modified(path: &Path, window: Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age <= window)
}

impl AiUsageService {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            events: Vec::new(),
            report: UsageReport::default(),
            active_sessions: 0,
            next_scan: Instant::now(),
            revision: 0,
        }
    }

    /// Call from the frame loop. Scans at most once a minute, only while `enabled`.
    pub fn poll(&mut self, enabled: bool, now: Instant) {
        if !enabled {
            if !self.events.is_empty() {
                *self = Self::new();
            }
            return;
        }
        if now < self.next_scan {
            return;
        }
        self.next_scan = now + SCAN_INTERVAL;
        self.scan();
    }

    fn scan(&mut self) {
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let mut claude = Vec::new();
        walk(&home.join(".claude").join("projects"), &mut claude, 0);
        let mut codex = Vec::new();
        walk(&home.join(".codex").join("sessions"), &mut codex, 0);
        let mut changed = false;
        let mut active = 0;
        for (path, is_codex) in claude
            .into_iter()
            .map(|path| (path, false))
            .chain(codex.into_iter().map(|path| (path, true)))
        {
            if !recently_modified(&path, MAX_AGE) {
                continue;
            }
            if recently_modified(&path, ACTIVE_WINDOW) {
                active += 1;
            }
            changed |= self.read_new_lines(&path, is_codex);
        }
        if changed || active != self.active_sessions {
            self.active_sessions = active;
            self.report = UsageReport::from_events(&self.events);
            self.revision = self.revision.wrapping_add(1);
        }
    }

    fn read_new_lines(&mut self, path: &Path, is_codex: bool) -> bool {
        let Ok(mut file) = File::open(path) else {
            return false;
        };
        let state = self.files.entry(path.to_path_buf()).or_default();
        let length = file.metadata().map_or(0, |metadata| metadata.len());
        if length < state.offset {
            // The log was truncated or replaced; read it again from the start.
            state.offset = 0;
        }
        if length == state.offset || file.seek(SeekFrom::Start(state.offset)).is_err() {
            return false;
        }
        let session = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let mut added = false;
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
            let event = if is_codex {
                if let Some(model) = ai_usage::codex_model(&line) {
                    state.codex_model = model;
                    None
                } else {
                    ai_usage::parse_codex_line(&line, &session, &state.codex_model)
                }
            } else {
                ai_usage::parse_claude_line(&line)
            };
            if let Some(event) = event {
                self.events.push(event);
                added = true;
            }
        }
        added
    }

    pub fn report(&self) -> &UsageReport {
        &self.report
    }

    pub fn active_sessions(&self) -> usize {
        self.active_sessions
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }
}
