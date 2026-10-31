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

/// A top-level window that belongs in a taskbar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppWindow {
    /// Raw HWND value; stable for the window's lifetime.
    pub handle: isize,
    pub title: String,
    /// Executable file name, such as `Code.exe`, used to group and label windows.
    pub executable: String,
    pub foreground: bool,
    pub minimized: bool,
}

/// Windows a taskbar would show: visible, unowned, not tool windows and not cloaked
/// (cloaked windows are UWP frames on other virtual desktops or suspended apps).
pub fn app_windows() -> Vec<AppWindow> {
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GW_OWNER, GWL_EXSTYLE, GetForegroundWindow, GetWindow, GetWindowLongW,
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, WS_EX_APPWINDOW,
    };
    use windows::core::BOOL;

    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam is the Vec passed by app_windows for the duration of EnumWindows.
        let found = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        found.push(hwnd);
        true.into()
    }

    let mut handles: Vec<HWND> = Vec::new();
    // SAFETY: the callback only pushes into `handles`, which outlives the call.
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut handles as *mut _ as isize));
    }
    // SAFETY: read-only queries on windows returned by EnumWindows.
    let foreground = unsafe { GetForegroundWindow() };
    let mut windows = Vec::new();
    for hwnd in handles {
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() {
                continue;
            }
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            let owned = GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.is_invalid());
            let app_window = ex_style & WS_EX_APPWINDOW.0 != 0;
            if (ex_style & WS_EX_TOOLWINDOW.0 != 0 || owned) && !app_window {
                continue;
            }
            let mut cloaked = 0u32;
            if DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut cloaked as *mut u32 as *mut _,
                std::mem::size_of::<u32>() as u32,
            )
            .is_ok()
                && cloaked != 0
            {
                continue;
            }
            let length = GetWindowTextLengthW(hwnd);
            if length <= 0 {
                continue;
            }
            let mut buffer = vec![0u16; length as usize + 1];
            let copied = GetWindowTextW(hwnd, &mut buffer);
            let title = String::from_utf16_lossy(&buffer[..copied.max(0) as usize]);
            let mut process_id = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut process_id));
            let executable = crate::process::open(process_id)
                .and_then(|process| crate::process::executable_name(*process))
                .unwrap_or_default();
            // Our own windows are not apps on the bar.
            if process_id == std::process::id()
                || executable.eq_ignore_ascii_case("explorer.exe") && title == "Program Manager"
            {
                continue;
            }
            windows.push(AppWindow {
                handle: hwnd.0 as isize,
                title,
                executable,
                foreground: hwnd == foreground,
                minimized: IsIconic(hwnd).as_bool(),
            });
        }
    }
    windows
}

/// Brings a window to the front, restoring it if minimised. Clicking the window that already
/// has focus minimises it, like the taskbar does.
pub fn activate_window(handle: isize) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_MENU,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, IsIconic, SW_MINIMIZE, SW_RESTORE, SetForegroundWindow,
    };
    let hwnd = HWND(handle as *mut _);
    // SAFETY: window queries and focus calls on a handle from app_windows; a stale handle
    // simply makes the calls fail.
    unsafe {
        if GetForegroundWindow() == hwnd && !IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
            return;
        }
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        // Windows only lets the foreground process change focus; a synthetic Alt press marks
        // this process as having received input so SetForegroundWindow is honoured.
        let key = |flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_MENU,
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        };
        let inputs = [key(Default::default()), key(KEYEVENTF_KEYUP)];
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Opens the real Start menu by sending the Windows key.
pub fn open_start_menu() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_LWIN,
    };
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_LWIN,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    // SAFETY: two well-formed keyboard inputs.
    unsafe {
        SendInput(
            &[key(Default::default()), key(KEYEVENTF_KEYUP)],
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}

/// Presses `keys` together and releases them in reverse order, for shell shortcuts such as
/// Win+A (Quick Settings) and Win+N (notifications).
pub fn press_shortcut(keys: &[u16]) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY,
    };
    let key = |vk: u16, flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let mut inputs: Vec<INPUT> = keys.iter().map(|vk| key(*vk, Default::default())).collect();
    inputs.extend(keys.iter().rev().map(|vk| key(*vk, KEYEVENTF_KEYUP)));
    // SAFETY: well-formed keyboard inputs.
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Virtual-key codes used with [`press_shortcut`].
pub const VK_WIN: u16 = 0x5B;
pub const VK_A: u16 = 0x41;
pub const VK_N: u16 = 0x4E;
