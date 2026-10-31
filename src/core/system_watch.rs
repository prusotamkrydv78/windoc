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
    /// Whole-system CPU load over the last interval, 0 to 100.
    pub cpu_percent: f32,
    /// Whether Windows reports an internet connection.
    pub online: bool,
    /// False for the empty snapshot before the first sample.
    pub sampled: bool,
}

static SNAPSHOT: Mutex<Option<SystemSnapshot>> = Mutex::new(None);
static STARTED: OnceLock<()> = OnceLock::new();

/// Starts the worker once; later calls do nothing.
pub fn start() {
    STARTED.get_or_init(|| {
        let spawned = std::thread::Builder::new()
            .name("system-watch".into())
            .spawn(|| {
                let mut cpu = CpuSampler::default();
                loop {
                    let mut snapshot = sample();
                    snapshot.cpu_percent = cpu.percent();
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
        cpu_percent: 0.0,
        online: online(),
        sampled: true,
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

fn online() -> bool {
    use windows::Win32::Networking::WinInet::{INTERNET_CONNECTION, InternetGetConnectedState};
    let mut flags = INTERNET_CONNECTION(0);
    // SAFETY: `flags` is a valid out-pointer; the reserved argument must be 0.
    unsafe { InternetGetConnectedState(&mut flags, Some(0)) }.is_ok()
}

/// CPU load from the change in idle, kernel and user time between calls.
#[derive(Default)]
struct CpuSampler {
    last: Option<(u64, u64)>,
}

impl CpuSampler {
    fn percent(&mut self) -> f32 {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Threading::GetSystemTimes;
        let (mut idle, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        // SAFETY: All three are valid out-pointers.
        if unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.is_err() {
            return 0.0;
        }
        let ticks =
            |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
        // Kernel time includes idle time.
        let (idle, total) = (ticks(idle), ticks(kernel) + ticks(user));
        let percent = self.last.map_or(0.0, |(last_idle, last_total)| {
            let total = total.saturating_sub(last_total);
            let idle = idle.saturating_sub(last_idle);
            if total == 0 {
                0.0
            } else {
                100.0 * (1.0 - idle as f32 / total as f32)
            }
        });
        self.last = Some((idle, total));
        percent
    }
}
