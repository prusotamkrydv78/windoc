//! Notices a mouse being plugged in by counting raw-input mice every couple of seconds.

use std::cell::Cell;
use std::time::{Duration, Instant};

use windows::Win32::UI::Input::{GetRawInputDeviceList, RAWINPUTDEVICELIST, RIM_TYPEMOUSE};

const CHECK_INTERVAL: Duration = Duration::from_secs(2);

thread_local! {
    static NEXT_CHECK: Cell<Option<Instant>> = const { Cell::new(None) };
    static MICE: Cell<Option<usize>> = const { Cell::new(None) };
}

fn mouse_count() -> Option<usize> {
    let size = std::mem::size_of::<RAWINPUTDEVICELIST>() as u32;
    let mut count = 0u32;
    // SAFETY: With no buffer, Windows only writes the device count.
    unsafe { GetRawInputDeviceList(None, &mut count, size) };
    let mut devices = vec![RAWINPUTDEVICELIST::default(); count as usize];
    // SAFETY: `devices` holds `count` entries of the size passed.
    let written = unsafe { GetRawInputDeviceList(Some(devices.as_mut_ptr()), &mut count, size) };
    if written == u32::MAX {
        return None;
    }
    Some(
        devices[..written as usize]
            .iter()
            .filter(|device| device.dwType == RIM_TYPEMOUSE)
            .count(),
    )
}

/// True once when more mice are attached than at the last check. Call from the frame loop.
pub fn mouse_arrived(now: Instant) -> bool {
    if NEXT_CHECK.with(Cell::get).is_some_and(|next| now < next) {
        return false;
    }
    NEXT_CHECK.with(|next| next.set(Some(now + CHECK_INTERVAL)));
    let Some(count) = mouse_count() else {
        return false;
    };
    let previous = MICE.with(|mice| mice.replace(Some(count)));
    previous.is_some_and(|previous| count > previous)
}
