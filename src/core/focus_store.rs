//! Saves and restores the Focus timer across restarts as `focus.json` in the data dir.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use winisland_core::focus::{FocusEvent, FocusSnapshot, FocusTimer, TimerKind};

const FILE_NAME: &str = "focus.json";
const MAX_FILE_BYTES: u64 = 16 * 1024;

fn path() -> PathBuf {
    crate::platform::shell().data_dir().join(FILE_NAME)
}

/// Loads the saved timer, or a fresh `default_kind` timer when nothing usable is stored.
/// A phase that ended while the app was closed is returned as an event.
pub fn load(default_kind: TimerKind) -> (FocusTimer, Option<FocusEvent>) {
    match read_snapshot() {
        Ok(Some(snapshot)) => FocusTimer::restore(&snapshot, Instant::now(), SystemTime::now()),
        Ok(None) => (FocusTimer::new(default_kind), None),
        Err(error) => {
            log::warn!("Ignoring saved Focus timer: {error}");
            (FocusTimer::new(default_kind), None)
        }
    }
}

fn read_snapshot() -> Result<Option<FocusSnapshot>, String> {
    let path = path();
    let file = match fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("file exceeds size limit".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| error.to_string())
}

/// Writes the timer atomically. Failures are logged; the timer keeps running in memory.
pub fn save(timer: &FocusTimer) {
    if let Err(error) = write_snapshot(&timer.snapshot(Instant::now(), SystemTime::now())) {
        log::warn!("Cannot save Focus timer: {error}");
    }
}

fn write_snapshot(snapshot: &FocusSnapshot) -> Result<(), String> {
    let path = path();
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec(snapshot).map_err(|error| error.to_string())?;
    let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&temp, bytes).map_err(|error| error.to_string())?;
    fs::rename(&temp, &path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        error.to_string()
    })
}
