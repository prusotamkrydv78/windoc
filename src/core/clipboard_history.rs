//! Connects clipboard history to the Windows clipboard monitor and stores it encrypted.
//!
//! History lives in `clipboard.bin`: the JSON snapshot encrypted with DPAPI for the
//! current Windows user, so another account or a copied disk cannot read it.

use std::time::{Duration, Instant, SystemTime};

use winisland_core::clipboard::{
    ClipboardBody, ClipboardHistory, ClipboardItem, ClipboardSettings, ClipboardSnapshot,
    SourceFormat,
};
use winisland_platform_windows::{ClipboardChange, ClipboardContent, ClipboardMonitor, dpapi};

use super::json_store;

pub(crate) const FILE_NAME: &str = "clipboard.bin";
const MAX_FILE_BYTES: u64 = 96 * 1024 * 1024;
/// Binds the encrypted file to this app; it is not a secret on its own.
const ENTROPY: &[u8] = b"winisland-next/clipboard/v1";
const SAVE_DELAY: Duration = Duration::from_secs(10);

pub struct ClipboardService {
    history: ClipboardHistory,
    monitor: Option<ClipboardMonitor>,
    save_due: Option<Instant>,
    revision: u64,
}

pub fn settings_for(config: &winisland_core::config::AppConfig) -> ClipboardSettings {
    ClipboardSettings {
        enabled: config.clipboard_history_enabled,
        ..ClipboardSettings::default()
    }
}

fn load_history(settings: ClipboardSettings) -> ClipboardHistory {
    let bytes = match json_store::read_bytes(FILE_NAME, MAX_FILE_BYTES) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return ClipboardHistory::new(settings),
        Err(error) => {
            log::warn!("Ignoring saved clipboard history: {error}");
            return ClipboardHistory::new(settings);
        }
    };
    let snapshot = dpapi::unprotect(&bytes, Some(ENTROPY))
        .map_err(|error| error.to_string())
        .and_then(|plain| {
            serde_json::from_slice::<ClipboardSnapshot>(&plain).map_err(|error| error.to_string())
        });
    match snapshot.and_then(|snapshot| {
        ClipboardHistory::restore(snapshot, settings.clone(), SystemTime::now())
            .map_err(|error| error.to_string())
    }) {
        Ok((history, _evicted)) => history,
        Err(error) => {
            log::warn!("Ignoring saved clipboard history: {error}");
            ClipboardHistory::new(settings)
        }
    }
}

impl ClipboardService {
    pub fn load(settings: ClipboardSettings) -> Self {
        let mut service = Self {
            history: load_history(settings),
            monitor: None,
            save_due: None,
            revision: 0,
        };
        service.sync_monitor();
        service
    }

    fn sync_monitor(&mut self) {
        if !self.history.settings().enabled {
            self.monitor = None;
            return;
        }
        if self.monitor.is_none() {
            match ClipboardMonitor::start() {
                Ok(monitor) => self.monitor = Some(monitor),
                Err(error) => log::warn!("Clipboard history unavailable: {error}"),
            }
        }
    }

    /// Applies changed settings. Turning history off stops monitoring immediately and
    /// keeps existing items until the user clears them.
    pub fn apply_settings(&mut self, settings: ClipboardSettings, now: Instant) {
        if *self.history.settings() == settings {
            return;
        }
        let evicted = self.history.set_settings(settings, SystemTime::now());
        if !evicted.is_empty() {
            self.changed(now);
        }
        self.sync_monitor();
    }

    /// Call from the frame loop.
    pub fn poll(&mut self, now: Instant) {
        let changes = self
            .monitor
            .as_ref()
            .map(ClipboardMonitor::drain)
            .unwrap_or_default();
        for change in changes {
            self.ingest(change, now);
        }
        if self.save_due.is_some_and(|due| now >= due) {
            self.save();
        }
    }

    fn ingest(&mut self, change: ClipboardChange, now: Instant) {
        let body = match change.content {
            ClipboardContent::Text(text) => ClipboardBody::from_text(&text),
            ClipboardContent::Files(files) => ClipboardBody::Files(files),
            ClipboardContent::Unsupported | ClipboardContent::Withheld => return,
        };
        let formats: Vec<SourceFormat<'_>> = change
            .formats
            .iter()
            .map(|format| SourceFormat {
                name: format.name,
                value: format.value,
            })
            .collect();
        let _ = self
            .history
            .capture(body, &formats, change.source_app, SystemTime::now());
        self.changed(now);
    }

    fn changed(&mut self, now: Instant) {
        self.revision = self.revision.wrapping_add(1);
        self.save_due.get_or_insert(now + SAVE_DELAY);
    }

    fn save(&mut self) {
        self.save_due = None;
        let result = serde_json::to_vec(&self.history.snapshot())
            .map_err(|error| error.to_string())
            .and_then(|plain| {
                dpapi::protect(&plain, Some(ENTROPY)).map_err(|error| error.to_string())
            })
            .and_then(|encrypted| json_store::write_bytes(FILE_NAME, &encrypted));
        if let Err(error) = result {
            log::warn!("Cannot save clipboard history: {error}");
        }
    }

    /// Saves pending changes. Call before the app exits.
    pub fn shutdown(&mut self) {
        self.monitor = None;
        if self.save_due.is_some() {
            self.save();
        }
    }

    pub fn enabled(&self) -> bool {
        self.history.settings().enabled
    }

    /// Newest first.
    pub fn items(&self) -> &[ClipboardItem] {
        self.history.items()
    }

    /// Deletes every item, pinned ones included, in memory and on disk.
    pub fn clear(&mut self) {
        let ids: Vec<u64> = self.history.items().iter().map(|item| item.id).collect();
        for id in ids {
            self.history.delete(id);
        }
        self.save_due = None;
        self.revision = self.revision.wrapping_add(1);
        if let Err(error) = json_store::remove(FILE_NAME) {
            log::warn!("Cannot delete clipboard history: {error}");
        }
    }

    /// Increases whenever history changes, so views can refresh cheaply.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}
