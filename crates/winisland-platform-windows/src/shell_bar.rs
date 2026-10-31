//! Top bar support: reserving a strip at the top of a monitor as an AppBar, and hiding the
//! classic Windows taskbar while the top bar replaces it.
//!
//! Hiding the taskbar is not an official API, so every path that hides it has a matching
//! restore: dropping the [`TaskbarHider`], calling [`restore_taskbar`], or the next launch
//! after a crash (the caller keeps a marker file and calls [`restore_taskbar`] at startup).

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    ABE_TOP, ABM_GETSTATE, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, ABM_SETSTATE,
    ABS_AUTOHIDE, APPBARDATA, SHAppBarMessage,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW, FindWindowW, IsWindowVisible,
    RegisterClassW, RegisterWindowMessageW, SW_HIDE, SW_SHOWNA, ShowWindow, WINDOW_EX_STYLE,
    WM_USER, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, w};

const CALLBACK_MESSAGE: u32 = WM_USER + 0x4B;

/// Screen rectangle in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScreenRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// A registered top AppBar. Windows keeps maximised windows below the reserved strip for as
/// long as this value lives; dropping it releases the space.
pub struct AppBarReservation {
    hwnd: HWND,
    reserved: ScreenRect,
}

impl AppBarReservation {
    /// Reserves `height` physical pixels along the top of `monitor`. Returns the strip Windows
    /// actually granted, which can differ when another AppBar already sits there.
    pub fn new(monitor: ScreenRect, height: i32) -> Option<Self> {
        let hwnd = create_host_window()?;
        let mut data = app_bar_data(hwnd);
        data.uCallbackMessage = CALLBACK_MESSAGE;
        // SAFETY: `data` is a valid, sized APPBARDATA for a window this process owns.
        if unsafe { SHAppBarMessage(ABM_NEW, &mut data) } == 0 {
            // SAFETY: the window was created above and is not used afterwards.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return None;
        }
        let mut reservation = Self {
            hwnd,
            reserved: monitor,
        };
        reservation.set_position(monitor, height);
        Some(reservation)
    }

    /// Moves or resizes the reserved strip, for example after a display change.
    pub fn set_position(&mut self, monitor: ScreenRect, height: i32) {
        let mut data = app_bar_data(self.hwnd);
        data.uEdge = ABE_TOP;
        data.rc = RECT {
            left: monitor.left,
            top: monitor.top,
            right: monitor.right,
            bottom: monitor.top + height,
        };
        // SAFETY: `data` is valid for both calls; Windows adjusts `rc` in place.
        unsafe {
            SHAppBarMessage(ABM_QUERYPOS, &mut data);
            data.rc.bottom = data.rc.top + height;
            SHAppBarMessage(ABM_SETPOS, &mut data);
        }
        self.reserved = ScreenRect {
            left: data.rc.left,
            top: data.rc.top,
            right: data.rc.right,
            bottom: data.rc.bottom,
        };
    }

    pub fn reserved(&self) -> ScreenRect {
        self.reserved
    }
}

impl Drop for AppBarReservation {
    fn drop(&mut self) {
        let mut data = app_bar_data(self.hwnd);
        // SAFETY: unregisters the AppBar registered in `new` and destroys its host window.
        unsafe {
            SHAppBarMessage(ABM_REMOVE, &mut data);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn app_bar_data(hwnd: HWND) -> APPBARDATA {
    APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: hwnd,
        ..Default::default()
    }
}

unsafe extern "system" fn host_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // AppBar notifications (position changes, full-screen apps) arrive as CALLBACK_MESSAGE;
    // the app polls state instead, so they only need the default handling here.
    // SAFETY: forwarding the unmodified message to the default procedure.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// An invisible, never-activated top-level window that owns the AppBar registration.
fn create_host_window() -> Option<HWND> {
    let class = w!("WinIslandTopBarHost");
    // SAFETY: plain Win32 class registration and window creation with static strings.
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(host_proc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        // Registering twice fails harmlessly; the class from the first call is reused.
        RegisterClassW(&wc);
        CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0),
            class,
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .ok()
    }
}

/// Every taskbar window: the primary `Shell_TrayWnd` and one `Shell_SecondaryTrayWnd` per
/// extra monitor.
fn taskbar_windows() -> Vec<HWND> {
    let mut windows = Vec::new();
    // SAFETY: FindWindow/FindWindowEx only read window class names.
    unsafe {
        if let Ok(primary) = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) {
            windows.push(primary);
        }
        let mut after: Option<HWND> = None;
        while let Ok(next) =
            FindWindowExW(None, after, w!("Shell_SecondaryTrayWnd"), PCWSTR::null())
        {
            if next.is_invalid() {
                break;
            }
            windows.push(next);
            after = Some(next);
        }
    }
    windows
}

/// The taskbar's AppBar state before we changed it, so restore can put it back exactly.
static PREVIOUS_STATE: AtomicIsize = AtomicIsize::new(-1);

/// Hides the classic taskbar on every monitor and switches it to auto-hide so Windows releases
/// its reserved space. Returns the previous AppBar state, for the caller's crash marker.
pub fn hide_taskbar() -> isize {
    let Some(&primary) = taskbar_windows().first() else {
        return -1;
    };
    let mut data = app_bar_data(primary);
    // SAFETY: querying and setting the taskbar's AppBar state with a valid APPBARDATA.
    let previous = unsafe { SHAppBarMessage(ABM_GETSTATE, &mut data) } as isize;
    if PREVIOUS_STATE
        .compare_exchange(-1, previous, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        data.lParam = LPARAM(ABS_AUTOHIDE as isize);
        // SAFETY: as above.
        unsafe {
            SHAppBarMessage(ABM_SETSTATE, &mut data);
        }
    }
    for hwnd in taskbar_windows() {
        // SAFETY: hiding windows owned by Explorer; restore_taskbar shows them again.
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
    PREVIOUS_STATE.load(Ordering::SeqCst)
}

/// Hides any taskbar window that came back, for example after Explorer restarted. Cheap
/// enough to call every couple of seconds while the top bar is active.
pub fn keep_taskbar_hidden() {
    for hwnd in taskbar_windows() {
        // SAFETY: visibility query and hide on Explorer's taskbar windows.
        unsafe {
            if IsWindowVisible(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }
}

/// Shows the taskbar again and restores its AppBar state. `previous_state` comes from
/// [`hide_taskbar`] or from the crash marker; `None` uses the state remembered in-process,
/// falling back to "always visible".
pub fn restore_taskbar(previous_state: Option<isize>) {
    let remembered = PREVIOUS_STATE.swap(-1, Ordering::SeqCst);
    let state = previous_state
        .filter(|state| *state >= 0)
        .or((remembered >= 0).then_some(remembered))
        .unwrap_or(0);
    let windows = taskbar_windows();
    if let Some(&primary) = windows.first() {
        let mut data = app_bar_data(primary);
        data.lParam = LPARAM(state);
        // SAFETY: setting the taskbar's AppBar state with a valid APPBARDATA.
        unsafe {
            SHAppBarMessage(ABM_SETSTATE, &mut data);
        }
    }
    for hwnd in windows {
        // SAFETY: showing Explorer's taskbar windows without activating them.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
        }
    }
}

/// The message Explorer broadcasts after (re)creating the taskbar.
pub fn taskbar_created_message() -> u32 {
    // SAFETY: registering a well-known message name.
    unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }
}
