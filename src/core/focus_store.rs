//! Saves and restores the Focus timer across restarts as `focus.json` in the data dir.

use std::time::{Instant, SystemTime};

use winisland_core::focus::{FocusEvent, FocusSnapshot, FocusTimer, TimerKind};

use super::json_store;

pub(crate) const FILE_NAME: &str = "focus.json";
const MAX_FILE_BYTES: u64 = 16 * 1024;

/// Loads the saved timer, or a fresh `default_kind` timer when nothing usable is stored.
/// A phase that ended while the app was closed is returned as an event.
pub fn load(default_kind: TimerKind) -> (FocusTimer, Option<FocusEvent>) {
    match json_store::read::<FocusSnapshot>(FILE_NAME, MAX_FILE_BYTES) {
        Ok(Some(snapshot)) => FocusTimer::restore(&snapshot, Instant::now(), SystemTime::now()),
        Ok(None) => (FocusTimer::new(default_kind), None),
        Err(error) => {
            log::warn!("Ignoring saved Focus timer: {error}");
            (FocusTimer::new(default_kind), None)
        }
    }
}

/// Writes the timer atomically. Failures are logged; the timer keeps running in memory.
pub fn save(timer: &FocusTimer) {
    let snapshot = timer.snapshot(Instant::now(), SystemTime::now());
    if let Err(error) = json_store::write(FILE_NAME, &snapshot) {
        log::warn!("Cannot save Focus timer: {error}");
    }
}
