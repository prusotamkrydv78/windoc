//! Clipboard history: capture policy, retention and search.
//!
//! This is the pure domain layer. Reading the Windows clipboard, DPAPI encryption and
//! blob storage live in the platform and storage layers; they hand this module a
//! [`ClipboardBody`] plus the clipboard format names that were present, and persist
//! [`ClipboardSnapshot`]s. Times are wall-clock [`SystemTime`]s supplied by the caller,
//! because retention is measured in calendar age and must survive restarts.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

/// Registered clipboard format that password managers set to keep an entry out of
/// clipboard monitors and history. Its presence alone excludes the entry.
pub const EXCLUDE_FROM_MONITORING_FORMAT: &str = "ExcludeClipboardContentFromMonitorProcessing";
/// Windows clipboard history opt-out. A DWORD value of 0 excludes the entry.
pub const CAN_INCLUDE_IN_HISTORY_FORMAT: &str = "CanIncludeInClipboardHistory";
/// Windows cloud clipboard opt-out. A DWORD value of 0 marks the entry as sensitive, so it
/// is excluded from history as well.
pub const CAN_UPLOAD_TO_CLOUD_FORMAT: &str = "CanUploadToCloudClipboard";

pub const SNAPSHOT_VERSION: u32 = 1;
pub const DEFAULT_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub const DEFAULT_MAX_ITEMS: usize = 200;
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

/// One clipboard format present on the source data. `value` carries the DWORD stored in
/// the format when it holds one (used by the `Can*` opt-out formats).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceFormat<'a> {
    pub name: &'a str,
    pub value: Option<u32>,
}

/// `true` when the source marked the data as private (for example a password manager).
pub fn is_excluded(formats: &[SourceFormat<'_>]) -> bool {
    formats.iter().any(|format| {
        format.name == EXCLUDE_FROM_MONITORING_FORMAT
            || ((format.name == CAN_INCLUDE_IN_HISTORY_FORMAT
                || format.name == CAN_UPLOAD_TO_CLOUD_FORMAT)
                && format.value == Some(0))
    })
}

/// Clipboard content. Bodies are kept behind this enum so the storage layer can encrypt
/// text and move images into content-addressed blobs without changing the history logic.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardBody {
    Text(String),
    /// A single http or https URL.
    Link(String),
    /// `blob_id` is expected to be content-addressed (for example a SHA-256 of the pixels),
    /// so identical images compare equal and are deduplicated.
    Image {
        width: u32,
        height: u32,
        blob_id: String,
    },
    /// References to files; the files themselves are not copied.
    Files(Vec<PathBuf>),
}

impl ClipboardBody {
    /// Classifies copied text, turning a lone http or https URL into a [`Self::Link`].
    pub fn from_text(text: &str) -> Self {
        match detect_link(text) {
            Some(link) => Self::Link(link.to_string()),
            None => Self::Text(text.to_string()),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            Self::Text(text) => text.trim().is_empty(),
            Self::Link(link) => link.is_empty(),
            Self::Image { width, height, .. } => *width == 0 || *height == 0,
            Self::Files(paths) => paths.is_empty(),
        }
    }

    /// Approximate storage cost used for the total-size cap. Images count as 32-bit
    /// pixels; file references count only their path text.
    pub fn size_bytes(&self) -> u64 {
        match self {
            Self::Text(text) | Self::Link(text) => text.len() as u64,
            Self::Image { width, height, .. } => u64::from(*width)
                .saturating_mul(u64::from(*height))
                .saturating_mul(4),
            Self::Files(paths) => paths.iter().map(|path| path.as_os_str().len() as u64).sum(),
        }
    }

    fn matches(&self, needle: &str) -> bool {
        match self {
            Self::Text(text) | Self::Link(text) => text.to_lowercase().contains(needle),
            Self::Image { .. } => false,
            Self::Files(paths) => paths
                .iter()
                .any(|path| path.to_string_lossy().to_lowercase().contains(needle)),
        }
    }
}

/// Returns the trimmed URL when `text` is exactly one http or https link.
fn detect_link(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if trimmed.chars().any(char::is_whitespace) {
        return None;
    }
    let scheme_len = ["http://", "https://"].into_iter().find_map(|scheme| {
        trimmed
            .get(..scheme.len())
            .filter(|prefix| prefix.eq_ignore_ascii_case(scheme))
            .map(|_| scheme.len())
    })?;
    (trimmed.len() > scheme_len).then_some(trimmed)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ClipboardItem {
    pub id: u64,
    pub body: ClipboardBody,
    /// Friendly name of the application that owned the clipboard, when known.
    pub source_app: Option<String>,
    pub captured_at: SystemTime,
    pub pinned: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ClipboardSettings {
    /// Clipboard monitoring is opt-in: when off, capture does nothing.
    pub enabled: bool,
    /// Unpinned items older than this are removed.
    pub retention: Duration,
    /// Maximum number of unpinned items.
    pub max_items: usize,
    /// Maximum total [`ClipboardBody::size_bytes`] of unpinned items.
    pub max_total_bytes: u64,
}

impl Default for ClipboardSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            retention: DEFAULT_RETENTION,
            max_items: DEFAULT_MAX_ITEMS,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureOutcome {
    Added(u64),
    /// Monitoring is turned off.
    Disabled,
    /// The source marked the data as private.
    Excluded,
    /// Identical to the newest entry.
    Duplicate,
    Empty,
}

/// Result of a capture. `evicted` lists entries removed by retention or caps so the
/// storage layer can delete their blobs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    pub outcome: CaptureOutcome,
    pub evicted: Vec<ClipboardItem>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ClipboardSnapshot {
    pub version: u32,
    pub next_id: u64,
    /// Newest first.
    pub items: Vec<ClipboardItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    UnsupportedVersion(u32),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported clipboard snapshot version {version}"
                )
            }
        }
    }
}

impl std::error::Error for SnapshotError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardHistory {
    settings: ClipboardSettings,
    /// Newest first.
    items: Vec<ClipboardItem>,
    next_id: u64,
}

impl ClipboardHistory {
    pub fn new(settings: ClipboardSettings) -> Self {
        Self {
            settings,
            items: Vec::new(),
            next_id: 1,
        }
    }

    pub fn settings(&self) -> &ClipboardSettings {
        &self.settings
    }

    /// Replaces the settings and applies the new limits immediately.
    pub fn set_settings(
        &mut self,
        settings: ClipboardSettings,
        now: SystemTime,
    ) -> Vec<ClipboardItem> {
        self.settings = settings;
        self.enforce_limits(now)
    }

    /// Turning monitoring off stops capture without deleting existing history.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.settings.enabled = enabled;
    }

    /// Newest first.
    pub fn items(&self) -> &[ClipboardItem] {
        &self.items
    }

    pub fn get(&self, id: u64) -> Option<&ClipboardItem> {
        self.items.iter().find(|item| item.id == id)
    }

    pub fn capture(
        &mut self,
        body: ClipboardBody,
        formats: &[SourceFormat<'_>],
        source_app: Option<String>,
        now: SystemTime,
    ) -> Capture {
        let outcome = if !self.settings.enabled {
            CaptureOutcome::Disabled
        } else if is_excluded(formats) {
            CaptureOutcome::Excluded
        } else if body.is_empty() {
            CaptureOutcome::Empty
        } else if self.items.first().is_some_and(|newest| newest.body == body) {
            CaptureOutcome::Duplicate
        } else {
            let id = self.next_id;
            self.next_id = self.next_id.saturating_add(1);
            self.items.insert(
                0,
                ClipboardItem {
                    id,
                    body,
                    source_app,
                    captured_at: now,
                    pinned: false,
                },
            );
            CaptureOutcome::Added(id)
        };
        let evicted = if matches!(outcome, CaptureOutcome::Added(_)) {
            self.enforce_limits(now)
        } else {
            Vec::new()
        };
        Capture { outcome, evicted }
    }

    /// Returns `false` when no item has this id.
    pub fn set_pinned(&mut self, id: u64, pinned: bool) -> bool {
        match self.items.iter_mut().find(|item| item.id == id) {
            Some(item) => {
                item.pinned = pinned;
                true
            }
            None => false,
        }
    }

    pub fn delete(&mut self, id: u64) -> Option<ClipboardItem> {
        let index = self.items.iter().position(|item| item.id == id)?;
        Some(self.items.remove(index))
    }

    /// Removes every unpinned item and returns them.
    pub fn clear_unpinned(&mut self) -> Vec<ClipboardItem> {
        let (pinned, removed): (Vec<_>, Vec<_>) = std::mem::take(&mut self.items)
            .into_iter()
            .partition(|item| item.pinned);
        self.items = pinned;
        removed
    }

    /// Case-insensitive substring search over text, links, file paths and source app.
    /// An empty query returns every item. Results are newest first.
    pub fn search(&self, query: &str) -> Vec<&ClipboardItem> {
        let needle = query.trim().to_lowercase();
        self.items
            .iter()
            .filter(|item| {
                needle.is_empty()
                    || item.body.matches(&needle)
                    || item
                        .source_app
                        .as_ref()
                        .is_some_and(|app| app.to_lowercase().contains(&needle))
            })
            .collect()
    }

    /// Applies retention age, count cap and size cap to unpinned items, oldest first.
    /// Pinned items are never removed and do not count toward the caps.
    pub fn enforce_limits(&mut self, now: SystemTime) -> Vec<ClipboardItem> {
        let retention = self.settings.retention;
        let mut evicted = Vec::new();
        let mut kept = Vec::with_capacity(self.items.len());
        for item in std::mem::take(&mut self.items) {
            // A capture time in the future (clock moved back) counts as age zero.
            let age = now
                .duration_since(item.captured_at)
                .unwrap_or(Duration::ZERO);
            if !item.pinned && age > retention {
                evicted.push(item);
            } else {
                kept.push(item);
            }
        }
        self.items = kept;

        let mut unpinned_count = self.items.iter().filter(|item| !item.pinned).count();
        let mut unpinned_bytes: u64 = self
            .items
            .iter()
            .filter(|item| !item.pinned)
            .map(|item| item.body.size_bytes())
            .sum();
        while unpinned_count > self.settings.max_items
            || unpinned_bytes > self.settings.max_total_bytes
        {
            let Some(index) = self.items.iter().rposition(|item| !item.pinned) else {
                break;
            };
            let item = self.items.remove(index);
            unpinned_count -= 1;
            unpinned_bytes = unpinned_bytes.saturating_sub(item.body.size_bytes());
            evicted.push(item);
        }
        evicted
    }

    pub fn snapshot(&self) -> ClipboardSnapshot {
        ClipboardSnapshot {
            version: SNAPSHOT_VERSION,
            next_id: self.next_id,
            items: self.items.clone(),
        }
    }

    /// Rebuilds history from a snapshot and applies the current limits.
    pub fn restore(
        snapshot: ClipboardSnapshot,
        settings: ClipboardSettings,
        now: SystemTime,
    ) -> Result<(Self, Vec<ClipboardItem>), SnapshotError> {
        if snapshot.version != SNAPSHOT_VERSION {
            return Err(SnapshotError::UnsupportedVersion(snapshot.version));
        }
        let max_id = snapshot.items.iter().map(|item| item.id).max().unwrap_or(0);
        let mut history = Self {
            settings,
            items: snapshot.items,
            next_id: snapshot.next_id.max(max_id.saturating_add(1)),
        };
        let evicted = history.enforce_limits(now);
        Ok((history, evicted))
    }
}

#[cfg(test)]
mod tests;
