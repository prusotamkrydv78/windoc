//! Safe mode: started with `--safe-mode`, the app loads no plugins and keeps every optional
//! module (audio capture, screen time, clipboard history, weather) off without changing
//! the saved settings, so a broken plugin or module cannot stop the app from starting.

use std::sync::atomic::{AtomicBool, Ordering};

pub const ARG: &str = "--safe-mode";

static ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn enable() {
    ACTIVE.store(true, Ordering::Relaxed);
}

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}
