//! Local data the app keeps, for the "Data on this PC" controls (workflow §22).

use super::json_store;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataKind {
    ScreenTime,
    Clipboard,
    Weather,
    Focus,
}

impl DataKind {
    pub const ALL: [Self; 4] = [
        Self::ScreenTime,
        Self::Clipboard,
        Self::Weather,
        Self::Focus,
    ];

    pub const fn file_name(self) -> &'static str {
        match self {
            Self::ScreenTime => super::screen_time::FILE_NAME,
            Self::Clipboard => super::clipboard_history::FILE_NAME,
            Self::Weather => super::weather_service::FILE_NAME,
            Self::Focus => super::focus_store::FILE_NAME,
        }
    }

    pub const fn label_key(self) -> &'static str {
        match self {
            Self::ScreenTime => "data_screen_time",
            Self::Clipboard => "data_clipboard",
            Self::Weather => "data_weather",
            Self::Focus => "data_focus",
        }
    }

    /// Bytes stored on disk, or `None` when nothing is stored.
    pub fn stored_bytes(self) -> Option<u64> {
        json_store::file_size(self.file_name())
    }
}

/// Human-readable size such as `12 KB`.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
