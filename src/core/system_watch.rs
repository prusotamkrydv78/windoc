//! Background sampling of system state the notch and top bar show.
//!
//! These queries block for a few milliseconds each (a registry scan, window enumeration with
//! a process lookup per window, shell and power calls), so a worker thread refreshes them
//! every two seconds and the UI thread only reads the latest snapshot.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use winisland_platform_windows::shell_bar::{self, AppWindow};

const INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Default)]
pub struct SystemSnapshot {
    pub microphone: bool,
    pub camera: bool,
    pub do_not_disturb: bool,
    pub battery: Option<(u8, bool)>,
    /// Taskbar-style app windows.
    pub apps: Vec<AppWindow>,
}

static SNAPSHOT: Mutex<Option<SystemSnapshot>> = Mutex::new(None);
static STARTED: OnceLock<()> = OnceLock::new();

/// Starts the worker once; later calls do nothing.
pub fn start() {
    STARTED.get_or_init(|| {
        let spawned = std::thread::Builder::new()
            .name("system-watch".into())
            .spawn(|| {
                loop {
                    let snapshot = sample();
                    if let Ok(mut slot) = SNAPSHOT.lock() {
                        *slot = Some(snapshot);
                    }
                    crate::platform::wake();
                    std::thread::sleep(INTERVAL);
                }
            });
        if let Err(error) = spawned {
            log::warn!("System watcher could not start: {error}");
        }
    });
}

fn sample() -> SystemSnapshot {
    let (microphone, camera) = crate::platform::capture_in_use();
    SystemSnapshot {
        microphone,
        camera,
        do_not_disturb: crate::platform::do_not_disturb(),
        battery: crate::platform::battery(),
        apps: shell_bar::app_windows(),
    }
}

/// The latest snapshot, or an empty one before the first sample.
pub fn snapshot() -> SystemSnapshot {
    SNAPSHOT
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or_default()
}
