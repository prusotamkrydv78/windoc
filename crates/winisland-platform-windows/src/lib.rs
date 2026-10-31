//! Windows implementations of the platform capability contracts.
//!
//! This crate owns the event loop, windows, and operating system calls; rendering stays in the application.

pub mod accessibility;
mod audio;
mod backdrop;
mod clipboard;
mod com;
mod display;
pub mod dpapi;
mod file;
mod foreground;
mod input;
mod media;
mod metrics;
mod notify;
mod process;
mod shell;
mod timezone;
pub mod window;

pub use audio::WindowsAudio;
pub use clipboard::{
    ClipboardChange, ClipboardContent, ClipboardFormat, ClipboardMonitor,
    set_text as set_clipboard_text,
};
pub use display::WindowsDisplay;
pub use file::replace_file;
pub use foreground::{ForegroundChange, ForegroundTracker, input_idle_duration};
pub use input::WindowsInput;
pub use media::WindowsMedia;
pub use metrics::{WindowsMetrics, battery_status, capture_in_use, do_not_disturb};
pub use notify::WindowsNotifications;
pub use shell::WindowsShell;
pub use timezone::{TimeZoneEntry, now_in_zone, time_zones};

use winisland_platform::{AudioProvider, Capabilities, MediaProvider, Platform, ShellIntegration};

pub struct WindowsPlatform;

impl WindowsPlatform {
    pub fn probe_capabilities() -> Capabilities {
        Capabilities {
            host_backdrop: true,
            tray: true,
            toast_events: notify::events_available(),
            media_session: WindowsMedia.open_context().is_ok(),
            audio_loopback: audio::process_loopback_available(),
            volume_control: WindowsAudio.volume().is_ok(),
            brightness_control: display::brightness_available(),
            autostart: WindowsShell.autostart_enabled().is_ok(),
            input_hooks: true,
        }
    }

    pub fn create() -> Platform {
        Platform {
            shell: Box::new(WindowsShell),
            metrics: Box::new(WindowsMetrics),
            display: Box::new(WindowsDisplay),
            audio: Box::new(WindowsAudio),
            media: Box::new(WindowsMedia),
            notify: Box::new(WindowsNotifications),
            input: Box::new(WindowsInput),
            capabilities: Self::probe_capabilities(),
        }
    }
}
