//! Top bar session: reserves the strip at the top of the primary monitor and hides the classic
//! taskbar while the "Top bar" setting is on.
//!
//! Safety first: a marker file records that the taskbar is hidden. It is removed on every
//! clean restore, so if the app crashes the next launch finds it and brings the taskbar back
//! before doing anything else.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use winisland_platform_windows::shell_bar::{self, AppBarReservation, ScreenRect};

/// Height of the bar in logical pixels; multiplied by the monitor scale.
pub const BAR_HEIGHT: f32 = 32.0;
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(1);
const MARKER: &str = "taskbar_hidden.flag";

fn marker_path() -> PathBuf {
    super::persistence::get_config_path().with_file_name(MARKER)
}

/// Brings the taskbar back if a previous run hid it and did not exit cleanly. Call once at
/// startup, before the top bar is (re)enabled.
pub fn recover_after_crash() {
    let path = marker_path();
    if let Ok(text) = std::fs::read_to_string(&path) {
        log::warn!("Restoring the Windows taskbar hidden by a previous session");
        shell_bar::restore_taskbar(text.trim().parse().ok());
        let _ = std::fs::remove_file(path);
    }
}

/// Restores the taskbar from a panic hook or exit path, whatever state the session is in.
pub fn emergency_restore() {
    // Only touch the taskbar when this app hid it; otherwise leave the user's setup alone.
    if marker_path().exists() {
        shell_bar::restore_taskbar(None);
        let _ = std::fs::remove_file(marker_path());
    }
}

pub struct TopBarSession {
    reservation: AppBarReservation,
    last_watchdog: Instant,
}

impl TopBarSession {
    /// Reserves `monitor`'s top strip and hides the taskbar. `scale` is the monitor DPI scale.
    pub fn start(monitor: ScreenRect, scale: f32) -> Option<Self> {
        let height = (BAR_HEIGHT * scale).round() as i32;
        let reservation = AppBarReservation::new(monitor, height)?;
        let previous = shell_bar::hide_taskbar(monitor);
        if let Err(error) = std::fs::write(marker_path(), previous.to_string()) {
            log::warn!("Could not write the taskbar marker: {error}");
        }
        log::info!("Top bar on: reserved {:?}", reservation.reserved());
        Some(Self {
            reservation,
            last_watchdog: Instant::now(),
        })
    }

    /// Keeps the taskbar hidden (Explorer recreates it after a restart) and follows monitor
    /// changes. Call from the app's periodic tick.
    pub fn poll(&mut self, monitor: ScreenRect, scale: f32, now: Instant) {
        if now.duration_since(self.last_watchdog) < WATCHDOG_INTERVAL {
            return;
        }
        self.last_watchdog = now;
        shell_bar::keep_taskbar_hidden(monitor);
        let height = (BAR_HEIGHT * scale).round() as i32;
        let wanted = ScreenRect {
            top: monitor.bottom - height,
            ..monitor
        };
        if self.reservation.reserved() != wanted {
            self.reservation.set_position(monitor, height);
        }
    }

    /// Lets the taskbar stay visible for `duration`, for example while its tray overflow is open.
    pub fn pause_watchdog(&mut self, duration: Duration) {
        self.last_watchdog = Instant::now() + duration;
    }

    /// The strip Windows granted, in physical screen pixels.
    pub fn reserved(&self) -> ScreenRect {
        self.reservation.reserved()
    }
}

impl Drop for TopBarSession {
    fn drop(&mut self) {
        shell_bar::restore_taskbar(None);
        let _ = std::fs::remove_file(marker_path());
        log::info!("Top bar off: taskbar restored");
    }
}
