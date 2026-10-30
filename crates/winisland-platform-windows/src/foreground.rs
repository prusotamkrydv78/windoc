//! Event-driven foreground-window tracking for screen time.
//!
//! A dedicated thread installs an out-of-context `EVENT_SYSTEM_FOREGROUND` hook and pumps
//! its message queue, so nothing is polled. Each change resolves the owning process's
//! executable name; window titles are read only when the caller opts in.

use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_SYSTEM_FOREGROUND, GetMessageW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_QUIT, WM_USER,
};
use winisland_platform::PlatformError;

use crate::process;

/// Queued changes beyond this are dropped; the next change supersedes them anyway.
const CHANNEL_CAPACITY: usize = 64;
const MAX_TITLE_UNITS: i32 = 512;
/// `OBJID_WINDOW`: the event refers to the window itself, not a child object.
const OBJECT_ID_WINDOW: i32 = 0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForegroundChange {
    /// Executable file name as reported by Windows, such as `Code.exe`.
    pub executable: String,
    /// Window title, present only when titles were requested and the window has one.
    pub title: Option<String>,
}

struct HookSink {
    sender: SyncSender<ForegroundChange>,
    include_titles: bool,
}

thread_local! {
    static SINK: RefCell<Option<HookSink>> = const { RefCell::new(None) };
}

/// Owns the tracking thread. Dropping it unhooks and stops the thread.
pub struct ForegroundTracker {
    thread_id: u32,
    receiver: Receiver<ForegroundChange>,
    thread: Option<JoinHandle<()>>,
}

impl ForegroundTracker {
    pub fn start(include_titles: bool) -> Result<Self, PlatformError> {
        let (sender, receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let (ready_sender, ready_receiver) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("winisland-foreground".into())
            .spawn(move || run(sender, include_titles, ready_sender))
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
    pub fn drain(&self) -> Vec<ForegroundChange> {
        self.receiver.try_iter().collect()
    }
}

impl Drop for ForegroundTracker {
    fn drop(&mut self) {
        // SAFETY: Posting WM_QUIT to the tracking thread's queue only ends its loop.
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(
    sender: SyncSender<ForegroundChange>,
    include_titles: bool,
    ready: mpsc::Sender<Result<u32, String>>,
) {
    let mut message = MSG::default();
    // SAFETY: Peeking creates this thread's message queue so PostThreadMessageW can reach it.
    let _ = unsafe { PeekMessageW(&mut message, None, WM_USER, WM_USER, PM_NOREMOVE) };
    SINK.with(|sink| {
        *sink.borrow_mut() = Some(HookSink {
            sender,
            include_titles,
        });
    });
    // SAFETY: The callback is a plain function, and out-of-context events are delivered
    // through this thread's message loop below.
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };
    if hook.is_invalid() {
        let _ = ready.send(Err("SetWinEventHook failed".into()));
        return;
    }
    // SAFETY: Reading the calling thread's id has no preconditions.
    let thread_id = unsafe { GetCurrentThreadId() };
    if ready.send(Ok(thread_id)).is_err() {
        unhook(hook);
        return;
    }
    // SAFETY: `message` is a valid out-pointer and the queue belongs to this thread.
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {}
    unhook(hook);
    SINK.with(|sink| sink.borrow_mut().take());
}

fn unhook(hook: HWINEVENTHOOK) {
    // SAFETY: The hook was installed by this thread and is removed exactly once.
    let _ = unsafe { UnhookWinEvent(hook) };
}

unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object_id: i32,
    _child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if event != EVENT_SYSTEM_FOREGROUND || object_id != OBJECT_ID_WINDOW || hwnd.0.is_null() {
        return;
    }
    SINK.with(|sink| {
        let sink = sink.borrow();
        let Some(sink) = sink.as_ref() else {
            return;
        };
        let Some(executable) = window_executable(hwnd) else {
            return;
        };
        let title = if sink.include_titles {
            window_title(hwnd)
        } else {
            None
        };
        // A full queue drops this change; a disconnected one means the tracker is stopping.
        let _ = sink.sender.try_send(ForegroundChange { executable, title });
    });
}

fn window_executable(hwnd: HWND) -> Option<String> {
    let mut process_id = 0;
    // SAFETY: hwnd comes from the foreground event and process_id is writable.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    if process_id == 0 {
        return None;
    }
    let handle = process::open(process_id)?;
    process::executable_name(*handle)
}

fn window_title(hwnd: HWND) -> Option<String> {
    // SAFETY: hwnd comes from the foreground event; the call only reads a length.
    let length = unsafe { GetWindowTextLengthW(hwnd) }.clamp(0, MAX_TITLE_UNITS);
    if length == 0 {
        return None;
    }
    let mut buffer = vec![0u16; length as usize + 1];
    // SAFETY: The buffer holds length + 1 UTF-16 units including the terminator.
    let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    let copied = usize::try_from(copied).ok().filter(|count| *count > 0)?;
    let title = String::from_utf16_lossy(&buffer[..copied.min(buffer.len())]);
    let title = title.trim();
    (!title.is_empty()).then(|| title.to_string())
}
