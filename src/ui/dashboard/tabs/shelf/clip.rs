//! Puts files on the clipboard as a file drop (`CF_HDROP`), so Explorer and apps paste the
//! files themselves rather than their paths.

use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::Duration;

use windows::Win32::Foundation::{GlobalFree, HANDLE};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};

const CF_HDROP: u32 = 15;
/// `DROPFILES`: `pFiles` (u32), `pt` (2 × i32), `fNC` (i32), `fWide` (i32).
const HEADER_BYTES: usize = 20;
const OPEN_ATTEMPTS: usize = 5;
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(15);

/// Replaces the clipboard with `paths` as a file list.
pub fn set_files(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("no files".into());
    }
    let mut wide: Vec<u16> = Vec::new();
    for path in paths {
        wide.extend(path.as_os_str().encode_wide());
        wide.push(0);
    }
    wide.push(0);
    let bytes = HEADER_BYTES + wide.len() * 2;
    // SAFETY: Allocating movable global memory has no preconditions.
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) }.map_err(|error| error.to_string())?;
    // SAFETY: The allocation is `bytes` long and stays locked until GlobalUnlock.
    let pointer = unsafe { GlobalLock(memory) }.cast::<u8>();
    if pointer.is_null() {
        // SAFETY: `memory` was allocated above and is not owned by the clipboard.
        let _ = unsafe { GlobalFree(Some(memory)) };
        return Err("cannot lock clipboard memory".into());
    }
    let header: [u32; 5] = [HEADER_BYTES as u32, 0, 0, 0, 1];
    // SAFETY: Both copies stay inside the `bytes`-long allocation and never overlap.
    unsafe {
        std::ptr::copy_nonoverlapping(header.as_ptr().cast::<u8>(), pointer, HEADER_BYTES);
        std::ptr::copy_nonoverlapping(
            wide.as_ptr().cast::<u8>(),
            pointer.add(HEADER_BYTES),
            wide.len() * 2,
        );
        let _ = GlobalUnlock(memory);
    }
    let mut opened = false;
    for _ in 0..OPEN_ATTEMPTS {
        // SAFETY: No owner window is needed to set data; the clipboard is closed below.
        if unsafe { OpenClipboard(None) }.is_ok() {
            opened = true;
            break;
        }
        std::thread::sleep(OPEN_RETRY_DELAY);
    }
    if !opened {
        // SAFETY: The clipboard never took `memory`.
        let _ = unsafe { GlobalFree(Some(memory)) };
        return Err("clipboard is busy".into());
    }
    // SAFETY: The clipboard is open on this thread; on success it owns `memory`.
    let result = unsafe {
        EmptyClipboard()
            .and_then(|()| SetClipboardData(CF_HDROP, Some(HANDLE(memory.0))))
            .map(|_| ())
    };
    // SAFETY: Opened above on this thread.
    let _ = unsafe { CloseClipboard() };
    result.map_err(|error| {
        // SAFETY: SetClipboardData failed, so the memory is still ours.
        let _ = unsafe { GlobalFree(Some(memory)) };
        error.to_string()
    })
}
