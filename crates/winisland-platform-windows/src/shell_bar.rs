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
    // System broadcasts this top-level window receives become shell events for the app.
    const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
    const WTS_SESSION_UNLOCK: usize = 0x8;
    const WM_DEVICECHANGE: u32 = 0x0219;
    const DBT_DEVICEARRIVAL: usize = 0x8000;
    const DBT_DEVICEREMOVECOMPLETE: usize = 0x8004;
    const DBT_DEVTYP_VOLUME: u32 = 2;
    const WM_DISPLAYCHANGE: u32 = 0x007E;
    const WM_POWERBROADCAST: u32 = 0x0218;
    const PBT_APMRESUMEAUTOMATIC: usize = 0x12;
    #[repr(C)]
    struct BroadcastVolume {
        size: u32,
        device_type: u32,
        reserved: u32,
        unit_mask: u32,
        flags: u16,
    }
    let event = match (msg, wparam.0) {
        (WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK) => Some(ShellEvent::Unlocked),
        (WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC) => Some(ShellEvent::Resumed),
        (WM_DISPLAYCHANGE, _) => Some(ShellEvent::DisplayChanged),
        (WM_DEVICECHANGE, change @ (DBT_DEVICEARRIVAL | DBT_DEVICEREMOVECOMPLETE))
            if lparam.0 != 0 =>
        {
            // SAFETY: for these notifications lparam points to a DEV_BROADCAST header, and
            // the volume layout is only read after checking its type.
            let header = unsafe { &*(lparam.0 as *const BroadcastVolume) };
            (header.device_type == DBT_DEVTYP_VOLUME && header.unit_mask != 0).then(|| {
                let letter = (b'A' + header.unit_mask.trailing_zeros() as u8) as char;
                if change == DBT_DEVICEARRIVAL {
                    ShellEvent::DriveAdded(letter)
                } else {
                    ShellEvent::DriveRemoved(letter)
                }
            })
        }
        _ => None,
    };
    if let Some(event) = event
        && let Ok(mut events) = SHELL_EVENTS.lock()
        && events.len() < 32
    {
        events.push(event);
    }
    // SAFETY: forwarding the unmodified message to the default procedure.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// A system event observed by the top bar's host window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellEvent {
    Unlocked,
    Resumed,
    DisplayChanged,
    DriveAdded(char),
    DriveRemoved(char),
}

static SHELL_EVENTS: std::sync::Mutex<Vec<ShellEvent>> = std::sync::Mutex::new(Vec::new());

/// Events received since the last call, oldest first.
pub fn take_shell_events() -> Vec<ShellEvent> {
    SHELL_EVENTS
        .lock()
        .map(|mut events| {
            let mut taken = std::mem::take(&mut *events);
            // The AppBar host and the listener both receive broadcasts; report each once.
            taken.dedup();
            taken
        })
        .unwrap_or_default()
}

/// Asks Windows to send session lock/unlock notifications to `hwnd`.
fn register_session_notifications(hwnd: HWND) {
    #[link(name = "wtsapi32")]
    unsafe extern "system" {
        fn WTSRegisterSessionNotification(hwnd: HWND, flags: u32) -> i32;
    }
    const NOTIFY_FOR_THIS_SESSION: u32 = 0;
    // SAFETY: registers a window this process owns; failure only loses unlock events.
    unsafe {
        WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION);
    }
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
        .inspect(|hwnd| register_session_notifications(*hwnd))
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

/// Hides the classic taskbar on `monitor` (the one the top bar replaces) and switches the
/// taskbar to auto-hide so Windows releases its reserved space; taskbars on other monitors stay
/// and simply auto-hide. Returns the previous AppBar state, for the caller's crash marker.
pub fn hide_taskbar(monitor: ScreenRect) -> isize {
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
    for hwnd in taskbar_windows_on(monitor) {
        // SAFETY: hiding windows owned by Explorer; restore_taskbar shows them again.
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
    PREVIOUS_STATE.load(Ordering::SeqCst)
}

/// Hides any taskbar window that came back, for example after Explorer restarted. Cheap
/// enough to call every couple of seconds while the top bar is active.
pub fn keep_taskbar_hidden(monitor: ScreenRect) {
    for hwnd in taskbar_windows_on(monitor) {
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
    pub process_id: u32,
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
                process_id,
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

/// Changes the volume of every audio session owned by `process_id` on the default output by
/// `delta` (-1.0..1.0) and returns the new level, or `None` when the app is not playing audio.
/// Blocking COM work: call it off the UI thread.
pub fn adjust_app_volume(process_id: u32, delta: f32) -> Option<f32> {
    use windows::Win32::Media::Audio::{
        IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume,
        MMDeviceEnumerator, eConsole, eRender,
    };
    use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
    use windows::core::Interface;

    let _com = crate::com::ComGuard::mta().ok()?;
    // SAFETY: COM calls on interfaces created here, with the apartment held by `_com`.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).ok()?;
        let sessions = manager.GetSessionEnumerator().ok()?;
        let mut level = None;
        for index in 0..sessions.GetCount().ok()? {
            let Ok(control) = sessions.GetSession(index) else {
                continue;
            };
            let Ok(control) = control.cast::<IAudioSessionControl2>() else {
                continue;
            };
            if control.GetProcessId().ok() != Some(process_id) {
                continue;
            }
            let Ok(volume) = control.cast::<ISimpleAudioVolume>() else {
                continue;
            };
            let current = volume.GetMasterVolume().unwrap_or(1.0);
            let next = (current + delta).clamp(0.0, 1.0);
            if volume.SetMasterVolume(next, std::ptr::null()).is_ok() {
                level = Some(next);
            }
        }
        level
    }
}

/// The window's large icon as straight RGBA pixels `(width, height, pixels)`, or `None` when
/// the window has no icon. Falls back to the window class icon.
pub fn window_icon_rgba(handle: isize) -> Option<(i32, i32, Vec<u8>)> {
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
        DeleteObject, GetDIBits, GetObjectW, HGDIOBJ,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GCLP_HICON, GetClassLongPtrW, GetIconInfo, HICON, ICON_BIG, ICONINFO, SMTO_ABORTIFHUNG,
        SendMessageTimeoutW, WM_GETICON,
    };
    let hwnd = HWND(handle as *mut _);
    // SAFETY: icon and bitmap queries on handles owned by the target window or created and
    // released here; a stale window just yields no icon.
    unsafe {
        let mut result = 0usize;
        let _ = SendMessageTimeoutW(
            hwnd,
            WM_GETICON,
            WPARAM(ICON_BIG as usize),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            50,
            Some(&mut result),
        );
        if result == 0 {
            result = GetClassLongPtrW(hwnd, GCLP_HICON);
        }
        if result == 0 {
            return None;
        }
        let icon = HICON(result as *mut _);
        let mut info = ICONINFO::default();
        GetIconInfo(icon, &mut info).ok()?;
        let color = info.hbmColor;
        let mask = info.hbmMask;
        let release = || {
            if !color.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(color.0));
            }
            if !mask.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(mask.0));
            }
        };
        if color.is_invalid() {
            release();
            return None;
        }
        let mut bitmap = BITMAP::default();
        if GetObjectW(
            HGDIOBJ(color.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bitmap as *mut BITMAP as *mut _),
        ) == 0
        {
            release();
            return None;
        }
        let (width, height) = (bitmap.bmWidth, bitmap.bmHeight);
        let mut header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (width * height * 4).max(0) as usize];
        let dc = CreateCompatibleDC(None);
        let rows = GetDIBits(
            dc,
            color,
            0,
            height as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut header,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        release();
        if rows == 0 {
            return None;
        }
        // BGRA → RGBA. Old icons carry no alpha at all; treat them as opaque.
        let has_alpha = pixels.chunks_exact(4).any(|px| px[3] != 0);
        for px in pixels.chunks_exact_mut(4) {
            px.swap(0, 2);
            if !has_alpha {
                px[3] = 255;
            }
        }
        Some((width, height, pixels))
    }
}

/// A live DWM preview of one window, composited by Windows into a rectangle of another
/// window (ours). Unregistered on drop.
pub struct WindowThumbnail {
    thumbnail: isize,
}

impl WindowThumbnail {
    /// Registers a preview of `source` inside `destination` (both raw HWNDs).
    pub fn new(destination: isize, source: isize) -> Option<Self> {
        use windows::Win32::Graphics::Dwm::DwmRegisterThumbnail;
        // SAFETY: both handles are top-level windows; failure just means no preview.
        let thumbnail = unsafe {
            DwmRegisterThumbnail(HWND(destination as *mut _), HWND(source as *mut _)).ok()?
        };
        Some(Self { thumbnail })
    }

    /// The source window's size in pixels.
    pub fn source_size(&self) -> Option<(i32, i32)> {
        use windows::Win32::Graphics::Dwm::DwmQueryThumbnailSourceSize;
        // SAFETY: querying a thumbnail registered by this value.
        unsafe { DwmQueryThumbnailSourceSize(self.thumbnail).ok() }.map(|size| (size.cx, size.cy))
    }

    /// Shows the preview in `rect`, given in the destination window's client pixels.
    pub fn place(&self, rect: ScreenRect, opacity: u8) {
        use windows::Win32::Graphics::Dwm::{
            DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
            DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE, DwmUpdateThumbnailProperties,
        };
        let properties = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION
                | DWM_TNP_VISIBLE
                | DWM_TNP_OPACITY
                | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: RECT {
                left: rect.left,
                top: rect.top,
                right: rect.right,
                bottom: rect.bottom,
            },
            opacity,
            fVisible: true.into(),
            fSourceClientAreaOnly: false.into(),
            ..Default::default()
        };
        // SAFETY: updating a thumbnail registered by this value with valid properties.
        unsafe {
            let _ = DwmUpdateThumbnailProperties(self.thumbnail, &properties);
        }
    }
}

impl Drop for WindowThumbnail {
    fn drop(&mut self) {
        use windows::Win32::Graphics::Dwm::DwmUnregisterThumbnail;
        // SAFETY: unregisters the thumbnail registered in `new`.
        unsafe {
            let _ = DwmUnregisterThumbnail(self.thumbnail);
        }
    }
}

/// Toggles the default microphone's mute and returns the new state (`true` = muted).
/// Blocking COM work: call it off the UI thread.
pub fn toggle_microphone_mute() -> Option<bool> {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        IMMDeviceEnumerator, MMDeviceEnumerator, eCapture, eCommunications,
    };
    use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
    let _com = crate::com::ComGuard::mta().ok()?;
    // SAFETY: COM calls on interfaces created here, with the apartment held by `_com`.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eCapture, eCommunications)
            .ok()?;
        let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None).ok()?;
        let muted = !volume.GetMute().ok()?.as_bool();
        volume.SetMute(muted, std::ptr::null()).ok()?;
        Some(muted)
    }
}

/// Executables of calling apps; a call is assumed while one runs and the mic is in use.
pub const CALL_APPS: [&str; 7] = [
    "ms-teams.exe",
    "teams.exe",
    "zoom.exe",
    "slack.exe",
    "discord.exe",
    "skype.exe",
    "webex.exe",
];

/// Opens the notification-area overflow ("show hidden icons"): Win+B focuses the tray
/// chevron and Enter opens it. With the taskbar hidden Windows may not show it, so this
/// falls back to the taskbar settings page that lists the tray icons.
pub fn open_tray_overflow() {
    const VK_B: u16 = 0x42;
    const VK_RETURN: u16 = 0x0D;
    press_shortcut(&[VK_WIN, VK_B]);
    std::thread::sleep(std::time::Duration::from_millis(80));
    press_shortcut(&[VK_RETURN]);
}

/// Shows the taskbar windows without changing their AppBar state, for a moment such as
/// opening the tray overflow; the top bar's watchdog hides them again afterwards.
pub fn show_taskbar_windows() {
    for hwnd in taskbar_windows() {
        // SAFETY: showing Explorer's taskbar windows without activating them.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNA);
        }
    }
}

/// Taskbar windows whose centre lies on `monitor`.
fn taskbar_windows_on(monitor: ScreenRect) -> Vec<HWND> {
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
    taskbar_windows()
        .into_iter()
        .filter(|hwnd| {
            let mut rect = RECT::default();
            // SAFETY: reading the bounds of an Explorer taskbar window.
            if unsafe { GetWindowRect(*hwnd, &mut rect) }.is_err() {
                return true;
            }
            let (x, y) = ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2);
            x >= monitor.left && x < monitor.right && y >= monitor.top && y < monitor.bottom
        })
        .collect()
}

/// A hidden window that listens for system broadcasts (unlock, resume, display and USB
/// changes) for the app's whole lifetime, independent of the top bar. See
/// [`take_shell_events`].
pub struct ShellEventListener {
    hwnd: HWND,
}

impl ShellEventListener {
    pub fn new() -> Option<Self> {
        create_host_window().map(|hwnd| Self { hwnd })
    }
}

impl Drop for ShellEventListener {
    fn drop(&mut self) {
        // SAFETY: destroys the window created in `new`.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
