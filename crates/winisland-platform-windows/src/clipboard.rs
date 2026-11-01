//! Event-driven clipboard monitoring.
//!
//! A message-only window on a dedicated thread receives `WM_CLIPBOARDUPDATE`. On each
//! change the exclusion markers set by password managers are checked first; when any is
//! present the content is never read. Text (`CF_UNICODETEXT`), file lists (`CF_HDROP`) and
//! images (the registered `PNG` format, else `CF_DIBV5` / `CF_DIB`) are supported; other
//! formats are reported as unsupported.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Foundation::{HANDLE, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, GetClipboardData, GetClipboardOwner,
    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
    RemoveClipboardFormatListener,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowThreadProcessId, HWND_MESSAGE, MSG, PostThreadMessageW, RegisterClassW,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE, WM_QUIT, WNDCLASSW,
};
use windows::core::w;
use winisland_platform::PlatformError;

use crate::process;

/// Set by password managers and other apps to keep content out of clipboard monitors.
pub const EXCLUDE_FROM_MONITORING: &str = "ExcludeClipboardContentFromMonitorProcessing";
/// A DWORD of 0 means the content must not be kept in clipboard history.
pub const CAN_INCLUDE_IN_HISTORY: &str = "CanIncludeInClipboardHistory";
/// A DWORD of 0 means the content must not be synced; treated as private here too.
pub const CAN_UPLOAD_TO_CLOUD: &str = "CanUploadToCloudClipboard";

const CF_UNICODETEXT: u32 = 13;
const CF_HDROP: u32 = 15;
const CF_DIB: u32 = 8;
const CF_DIBV5: u32 = 17;
/// Images larger than this as raw RGBA are not captured.
pub const MAX_IMAGE_BYTES: u64 = 40 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_FILES: u32 = 256;
const CHANNEL_CAPACITY: usize = 32;
const OPEN_ATTEMPTS: u32 = 5;
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(20);
/// Apps often write the clipboard in several steps; opening it the instant it changes can
/// make their next step fail, so reading waits briefly and then takes the settled content.
const SETTLE_DELAY: Duration = Duration::from_millis(100);

/// One registered clipboard format present on the clipboard, with its DWORD value when it
/// is one of the marker formats.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardFormat {
    pub name: &'static str,
    pub value: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardContent {
    Text(String),
    Files(Vec<PathBuf>),
    /// Straight (not premultiplied) RGBA, `width * height * 4` bytes, top row first.
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    /// Other formats, and images over [`MAX_IMAGE_BYTES`].
    Unsupported,
    /// An exclusion marker was present, so the content was not read.
    Withheld,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardChange {
    /// Marker formats found on the clipboard.
    pub formats: Vec<ClipboardFormat>,
    pub content: ClipboardContent,
    /// Executable name of the clipboard owner, when it can be resolved.
    pub source_app: Option<String>,
}

thread_local! {
    static SINK: RefCell<Option<SyncSender<ClipboardChange>>> = const { RefCell::new(None) };
}

/// Owns the monitoring thread. Dropping it removes the listener and stops the thread.
pub struct ClipboardMonitor {
    thread_id: u32,
    receiver: Receiver<ClipboardChange>,
    thread: Option<JoinHandle<()>>,
}

impl ClipboardMonitor {
    pub fn start() -> Result<Self, PlatformError> {
        let (sender, receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let (ready_sender, ready_receiver) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("winisland-clipboard".into())
            .spawn(move || run(sender, ready_sender))
            .map_err(PlatformError::backend)?;
        let thread_id = ready_receiver
            .recv()
            .map_err(PlatformError::backend)?
            .map_err(PlatformError::backend)?;
        Ok(Self {
            thread_id,
            receiver,
            thread: Some(thread),
        })
    }

    /// Returns all changes queued since the last call, oldest first.
    pub fn drain(&self) -> Vec<ClipboardChange> {
        self.receiver.try_iter().collect()
    }
}

impl Drop for ClipboardMonitor {
    fn drop(&mut self) {
        // SAFETY: Posting WM_QUIT to the monitor thread's queue only ends its loop.
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(sender: SyncSender<ClipboardChange>, ready: mpsc::Sender<Result<u32, String>>) {
    let window = match create_window() {
        Ok(window) => window,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    SINK.with(|sink| *sink.borrow_mut() = Some(sender));
    // SAFETY: The window belongs to this thread and lives until the loop below ends.
    if let Err(error) = unsafe { AddClipboardFormatListener(window) } {
        let _ = ready.send(Err(error.to_string()));
        destroy(window);
        return;
    }
    // SAFETY: Reading the calling thread's id has no preconditions.
    let thread_id = unsafe { GetCurrentThreadId() };
    if ready.send(Ok(thread_id)).is_ok() {
        let mut message = MSG::default();
        // SAFETY: `message` is a valid out-pointer and the queue belongs to this thread.
        // WM_CLIPBOARDUPDATE is posted, so it reaches the window procedure only through
        // DispatchMessageW.
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
            // SAFETY: `message` was just filled in by GetMessageW for this thread.
            unsafe { DispatchMessageW(&message) };
        }
    }
    // SAFETY: The listener was added for this window on this thread.
    let _ = unsafe { RemoveClipboardFormatListener(window) };
    destroy(window);
    SINK.with(|sink| sink.borrow_mut().take());
}

fn create_window() -> Result<HWND, String> {
    // SAFETY: Querying the current module handle has no preconditions.
    let module = unsafe { GetModuleHandleW(None) }.map_err(|error| error.to_string())?;
    let instance = HINSTANCE(module.0);
    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: w!("WinIslandClipboardMonitor"),
        ..WNDCLASSW::default()
    };
    // SAFETY: The class structure is fully initialised. Registering an existing class
    // fails harmlessly, and CreateWindowExW below reports any real problem.
    unsafe { RegisterClassW(&class) };
    // SAFETY: A message-only window with a registered class and no creation data.
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("WinIslandClipboardMonitor"),
            w!(""),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance),
            None,
        )
    }
    .map_err(|error| error.to_string())
}

fn destroy(window: HWND) {
    // SAFETY: The window was created by this thread and is destroyed exactly once.
    let _ = unsafe { DestroyWindow(window) };
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_CLIPBOARDUPDATE {
        // Only this monitor thread waits; repeated updates read the same content and the
        // history drops the duplicates.
        std::thread::sleep(SETTLE_DELAY);
        if let Some(change) = read_clipboard(window) {
            SINK.with(|sink| {
                if let Some(sender) = sink.borrow().as_ref() {
                    // A full queue drops this change; the next one supersedes it.
                    let _ = sender.try_send(change);
                }
            });
        }
        return LRESULT(0);
    }
    // SAFETY: Unhandled messages go to the default procedure with their original arguments.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn open_clipboard(window: HWND) -> bool {
    for attempt in 0..OPEN_ATTEMPTS {
        // SAFETY: The window belongs to this thread; the clipboard is closed in read_clipboard.
        if unsafe { OpenClipboard(Some(window)) }.is_ok() {
            return true;
        }
        if attempt + 1 < OPEN_ATTEMPTS {
            std::thread::sleep(OPEN_RETRY_DELAY);
        }
    }
    false
}

fn read_clipboard(window: HWND) -> Option<ClipboardChange> {
    let source_app = clipboard_owner();
    if !open_clipboard(window) {
        log::warn!("Clipboard changed but could not be opened");
        return None;
    }
    let formats = marker_formats();
    let excluded = formats
        .iter()
        .any(|format| format.name == EXCLUDE_FROM_MONITORING || format.value == Some(0));
    let content = if excluded {
        ClipboardContent::Withheld
    } else if let Some(text) = read_text() {
        ClipboardContent::Text(text)
    } else if let Some(files) = read_files() {
        ClipboardContent::Files(files)
    } else if let Some((width, height, rgba)) = read_image() {
        ClipboardContent::Image {
            width,
            height,
            rgba,
        }
    } else {
        ClipboardContent::Unsupported
    };
    // SAFETY: The clipboard was opened above by this thread.
    let _ = unsafe { CloseClipboard() };
    Some(ClipboardChange {
        formats,
        content,
        source_app,
    })
}

fn clipboard_owner() -> Option<String> {
    // SAFETY: Reading the clipboard owner has no preconditions.
    let owner = unsafe { GetClipboardOwner() }.ok()?;
    if owner.0.is_null() {
        return None;
    }
    let mut process_id = 0;
    // SAFETY: owner is a window handle from Windows and process_id is writable.
    unsafe { GetWindowThreadProcessId(owner, Some(&mut process_id)) };
    if process_id == 0 {
        return None;
    }
    let handle = process::open(process_id)?;
    process::executable_name(*handle)
}

fn marker_formats() -> Vec<ClipboardFormat> {
    let mut formats = Vec::new();
    for (name, wide) in [
        (
            EXCLUDE_FROM_MONITORING,
            w!("ExcludeClipboardContentFromMonitorProcessing"),
        ),
        (CAN_INCLUDE_IN_HISTORY, w!("CanIncludeInClipboardHistory")),
        (CAN_UPLOAD_TO_CLOUD, w!("CanUploadToCloudClipboard")),
    ] {
        // SAFETY: Registering a format name only returns its id.
        let id = unsafe { RegisterClipboardFormatW(wide) };
        // SAFETY: The clipboard is open on this thread.
        if id == 0 || unsafe { IsClipboardFormatAvailable(id) }.is_err() {
            continue;
        }
        let value = if name == EXCLUDE_FROM_MONITORING {
            None
        } else {
            read_dword(id)
        };
        formats.push(ClipboardFormat { name, value });
    }
    formats
}

/// Locks a clipboard memory handle, passes its bytes to `read` and unlocks it.
fn with_global<T>(handle: HANDLE, read: impl FnOnce(&[u8]) -> Option<T>) -> Option<T> {
    let memory = HGLOBAL(handle.0);
    // SAFETY: The handle comes from GetClipboardData while the clipboard is open.
    let size = unsafe { GlobalSize(memory) };
    // SAFETY: As above; the pointer is valid until GlobalUnlock.
    let pointer = unsafe { GlobalLock(memory) };
    if pointer.is_null() || size == 0 {
        return None;
    }
    // SAFETY: GlobalLock returned a pointer to `size` readable bytes, unlocked below.
    let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), size) };
    let result = read(bytes);
    // SAFETY: Balances the GlobalLock above. A zero lock count is reported as an error.
    let _ = unsafe { GlobalUnlock(memory) };
    result
}

fn read_dword(format: u32) -> Option<u32> {
    // SAFETY: The clipboard is open on this thread.
    let handle = unsafe { GetClipboardData(format) }.ok()?;
    with_global(handle, |bytes| {
        let raw: [u8; 4] = bytes.get(..4)?.try_into().ok()?;
        Some(u32::from_le_bytes(raw))
    })
}

fn read_text() -> Option<String> {
    // SAFETY: The clipboard is open on this thread.
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT) }.ok()?;
    with_global(handle, |bytes| {
        let bytes = &bytes[..bytes.len().min(MAX_TEXT_BYTES)];
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|unit| *unit != 0)
            .collect();
        let text = String::from_utf16_lossy(&units);
        (!text.trim().is_empty()).then_some(text)
    })
}

fn read_files() -> Option<Vec<PathBuf>> {
    // SAFETY: The clipboard is open on this thread.
    let handle = unsafe { GetClipboardData(CF_HDROP) }.ok()?;
    let hdrop = HDROP(handle.0);
    // SAFETY: Index 0xFFFFFFFF with no buffer returns the file count.
    let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) }.min(MAX_FILES);
    let mut files = Vec::new();
    for index in 0..count {
        // SAFETY: With no buffer the call returns the path length in characters.
        let length = unsafe { DragQueryFileW(hdrop, index, None) };
        if length == 0 {
            continue;
        }
        let mut buffer = vec![0u16; length as usize + 1];
        // SAFETY: The buffer holds the path and its terminator.
        let copied = unsafe { DragQueryFileW(hdrop, index, Some(&mut buffer)) } as usize;
        files.push(PathBuf::from(String::from_utf16_lossy(
            &buffer[..copied.min(length as usize)],
        )));
    }
    (!files.is_empty()).then_some(files)
}

fn png_format() -> u32 {
    // SAFETY: Registering a format name only returns its id.
    unsafe { RegisterClipboardFormatW(w!("PNG")) }
}

fn image_fits(width: u32, height: u32) -> bool {
    width > 0 && height > 0 && u64::from(width) * u64::from(height) * 4 <= MAX_IMAGE_BYTES
}

fn decode_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let (width, height) = reader.into_dimensions().ok()?;
    if !image_fits(width, height) {
        return None;
    }
    let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    Some((decoded.width(), decoded.height(), decoded.into_raw()))
}

/// Reads an image as RGBA, preferring the lossless `PNG` format apps add next to the DIB.
fn read_image() -> Option<(u32, u32, Vec<u8>)> {
    let png = png_format();
    // SAFETY: The clipboard is open on this thread.
    if png != 0
        && let Ok(handle) = unsafe { GetClipboardData(png) }
        && let Some(image) = with_global(handle, decode_png)
    {
        return Some(image);
    }
    [CF_DIBV5, CF_DIB].into_iter().find_map(|format| {
        // SAFETY: The clipboard is open on this thread.
        let handle = unsafe { GetClipboardData(format) }.ok()?;
        with_global(handle, decode_dib)
    })
}

fn le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Decodes an uncompressed 24- or 32-bit packed DIB (a `BITMAPINFOHEADER` or `BITMAPV5HEADER`
/// followed by pixels) to top-down RGBA.
fn decode_dib(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    const BI_RGB: u32 = 0;
    const BI_BITFIELDS: u32 = 3;
    let header = le_u32(bytes, 0)? as usize;
    let width = le_u32(bytes, 4)? as i32;
    let height = le_u32(bytes, 8)? as i32;
    let bits = u16::from_le_bytes(bytes.get(14..16)?.try_into().ok()?);
    let compression = le_u32(bytes, 16)?;
    if header < 40 || width <= 0 || height == 0 || !matches!(bits, 24 | 32) {
        return None;
    }
    // BITFIELDS masks follow a plain 40-byte header; larger headers carry them inside.
    // Only the standard BGR(A) layout is read; the masks are assumed to match it.
    let masks = match compression {
        BI_BITFIELDS if header == 40 => 12,
        BI_RGB | BI_BITFIELDS => 0,
        _ => return None,
    };
    let colors_used = le_u32(bytes, 32).unwrap_or(0) as usize;
    let (width, rows) = (width as u32, height.unsigned_abs());
    if !image_fits(width, rows) {
        return None;
    }
    let top_down = height < 0;
    let pixel_bytes = usize::from(bits / 8);
    let stride = (width as usize * pixel_bytes).div_ceil(4) * 4;
    let start = header + masks + colors_used * 4;
    let pixels = bytes.get(start..start + stride * rows as usize)?;
    let mut rgba = Vec::with_capacity(width as usize * rows as usize * 4);
    let mut any_alpha = false;
    for row in 0..rows as usize {
        let source = if top_down { row } else { rows as usize - 1 - row };
        let line = &pixels[source * stride..source * stride + width as usize * pixel_bytes];
        for pixel in line.chunks_exact(pixel_bytes) {
            let alpha = if bits == 32 { pixel[3] } else { 255 };
            any_alpha |= alpha != 0;
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], alpha]);
        }
    }
    // Most 32-bit DIBs leave the fourth byte zero: treat that as opaque.
    if !any_alpha {
        rgba.chunks_exact_mut(4).for_each(|pixel| pixel[3] = 255);
    }
    Some((width, rows, rgba))
}

/// Opens the clipboard for writing, retrying while another app holds it.
fn open_for_write() -> Result<(), PlatformError> {
    for _ in 0..OPEN_ATTEMPTS {
        // SAFETY: No owner window is needed to set data; callers close the clipboard.
        if unsafe { OpenClipboard(None) }.is_ok() {
            return Ok(());
        }
        std::thread::sleep(OPEN_RETRY_DELAY);
    }
    Err(PlatformError::backend("clipboard is busy"))
}

/// Copies `bytes` into movable global memory and hands it to the clipboard as `format`.
fn set_global(format: u32, bytes: &[u8]) -> Result<(), PlatformError> {
    use windows::Win32::System::DataExchange::SetClipboardData;
    use windows::Win32::Foundation::GlobalFree;
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc};
    // SAFETY: Allocating movable global memory has no preconditions.
    let memory =
        unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }.map_err(PlatformError::backend)?;
    // SAFETY: The allocation is `bytes.len()` long and locked until GlobalUnlock.
    let pointer = unsafe { GlobalLock(memory) };
    if pointer.is_null() {
        // SAFETY: The memory was allocated above and is not owned by the clipboard.
        let _ = unsafe { GlobalFree(Some(memory)) };
        return Err(PlatformError::backend("cannot lock clipboard memory"));
    }
    // SAFETY: Source and destination are valid for `bytes.len()` bytes and do not overlap.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len()) };
    // SAFETY: Balances the lock above.
    let _ = unsafe { GlobalUnlock(memory) };
    // SAFETY: Ownership of the memory passes to the clipboard on success.
    match unsafe { SetClipboardData(format, Some(HANDLE(memory.0))) } {
        Ok(_) => Ok(()),
        Err(error) => {
            // SAFETY: The clipboard did not take the memory, so it is still ours.
            let _ = unsafe { GlobalFree(Some(memory)) };
            Err(PlatformError::backend(error))
        }
    }
}

/// Puts an RGBA image (straight alpha, top row first) on the clipboard as `CF_DIB` and `PNG`.
pub fn set_image(width: u32, height: u32, rgba: &[u8]) -> Result<(), PlatformError> {
    use windows::Win32::System::DataExchange::EmptyClipboard;
    if !image_fits(width, height) || rgba.len() != width as usize * height as usize * 4 {
        return Err(PlatformError::backend("image size does not match its pixels"));
    }
    // A bottom-up 32-bit BI_RGB DIB: the format every app reads.
    let mut dib = Vec::with_capacity(40 + rgba.len());
    dib.extend_from_slice(&40u32.to_le_bytes());
    dib.extend_from_slice(&(width as i32).to_le_bytes());
    dib.extend_from_slice(&(height as i32).to_le_bytes());
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&0u32.to_le_bytes());
    dib.extend_from_slice(&(rgba.len() as u32).to_le_bytes());
    dib.extend_from_slice(&[0; 16]);
    for row in rgba.chunks_exact(width as usize * 4).rev() {
        for pixel in row.chunks_exact(4) {
            dib.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    let mut png = Vec::new();
    let encoded = image::RgbaImage::from_raw(width, height, rgba.to_vec()).is_some_and(|image| {
        image
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .is_ok()
    });
    open_for_write()?;
    let result = (|| {
        // SAFETY: The clipboard is open on this thread.
        unsafe { EmptyClipboard() }.map_err(PlatformError::backend)?;
        set_global(CF_DIB, &dib)?;
        let png_id = png_format();
        if encoded && png_id != 0 {
            // The DIB is already set; a missing PNG only loses transparency.
            let _ = set_global(png_id, &png);
        }
        Ok(())
    })();
    // SAFETY: The clipboard was opened above by this thread.
    let _ = unsafe { CloseClipboard() };
    result
}

/// Puts `text` on the clipboard as Unicode text.
pub fn set_text(text: &str) -> Result<(), PlatformError> {
    use windows::Win32::System::DataExchange::{EmptyClipboard, SetClipboardData};
    use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc};

    let mut units: Vec<u16> = text.encode_utf16().collect();
    units.push(0);
    let bytes = units.len() * 2;
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
        return Err(PlatformError::backend("clipboard is busy"));
    }
    let result = (|| {
        // SAFETY: The clipboard is open on this thread.
        unsafe { EmptyClipboard() }.map_err(PlatformError::backend)?;
        // SAFETY: Allocating movable global memory has no preconditions.
        let memory =
            unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) }.map_err(PlatformError::backend)?;
        // SAFETY: The allocation is `bytes` long and locked until GlobalUnlock.
        let pointer = unsafe { GlobalLock(memory) };
        if pointer.is_null() {
            return Err(PlatformError::backend("cannot lock clipboard memory"));
        }
        // SAFETY: Source and destination are valid for `bytes` bytes and do not overlap.
        unsafe {
            std::ptr::copy_nonoverlapping(units.as_ptr().cast::<u8>(), pointer.cast::<u8>(), bytes);
        }
        // SAFETY: Balances the lock above.
        let _ = unsafe { GlobalUnlock(memory) };
        // SAFETY: Ownership of the memory passes to the clipboard on success.
        unsafe { SetClipboardData(CF_UNICODETEXT, Some(HANDLE(memory.0))) }
            .map(|_| ())
            .map_err(PlatformError::backend)
    })();
    // SAFETY: The clipboard was opened above by this thread.
    let _ = unsafe { CloseClipboard() };
    result
}
