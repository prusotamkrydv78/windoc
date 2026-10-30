//! Event-driven clipboard monitoring.
//!
//! A message-only window on a dedicated thread receives `WM_CLIPBOARDUPDATE`. On each
//! change the exclusion markers set by password managers are checked first; when any is
//! present the content is never read. Text (`CF_UNICODETEXT`) and file lists (`CF_HDROP`)
//! are supported; other formats are reported as unsupported.

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
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetMessageW, GetWindowThreadProcessId,
    HWND_MESSAGE, MSG, PostThreadMessageW, RegisterClassW, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_CLIPBOARDUPDATE, WM_QUIT, WNDCLASSW,
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
const MAX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_FILES: u32 = 256;
const CHANNEL_CAPACITY: usize = 32;
const OPEN_ATTEMPTS: u32 = 5;
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(20);

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
    /// Images and other formats are not captured yet.
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
        // WM_CLIPBOARDUPDATE is a sent message, dispatched to the window procedure here.
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {}
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
