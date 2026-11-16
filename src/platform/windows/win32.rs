//! Win32 helpers for the shell. All unsafe code is confined here or in `render`.

use std::mem::size_of;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HINSTANCE, HWND};
use windows_sys::Win32::System::Console::{GetConsoleProcessList, GetConsoleWindow};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, IDC_ARROW, KillTimer, LoadCursorW, PostMessageW, RegisterClassW, SW_HIDE,
    SetProcessDPIAware, SetTimer, SetWindowTextW, ShowWindow, WNDCLASSW, WNDPROC, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

pub(crate) const WM_APP_EVENT: u32 = 0x8000 + 1;
/// Messages sent by the system (not posted) that the WndProc forwards to the queue.
pub(crate) const WM_APP_FOCUS: u32 = WM_APP_EVENT + 1;
pub(crate) const WM_APP_RELAYOUT: u32 = WM_APP_EVENT + 2;
/// `WM_MOUSELEAVE` lives in `Win32::UI::Controls`; this avoids an extra feature.
pub(crate) const WM_MOUSELEAVE: u32 = 0x02A3;
pub(crate) const TIMER_ID: usize = 1;

/// Window handle that can be sent to the source/IPC threads.
/// `PostMessageW` is safe from any thread while the window exists.
#[derive(Clone, Copy)]
pub(crate) struct SendHwnd(pub HWND);

// SAFETY: the handle is only used for `PostMessageW`; the window lives until
// the process exits.
unsafe impl Send for SendHwnd {}
unsafe impl Sync for SendHwnd {}

pub(crate) const STRIP_EX_STYLE: u32 =
    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST;

pub(crate) const CALENDAR_EX_STYLE: u32 = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;

pub(crate) fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Declares Per-Monitor V2 before querying DPI or creating windows. Without it
/// Windows treats the process as DPI-unaware: `GetDpiForSystem` returns 96 and
/// DWM stretches the windows, with blurry text at 125–150 %.
pub(crate) fn enable_dpi_awareness() {
    // SAFETY: called only once at process start, before creating
    // windows or querying DPI.
    let ok = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if ok == 0 {
        // SAFETY: fallback for the same DPI initialization; creates no resources
        // and runs before any window or DPI query.
        unsafe {
            SetProcessDPIAware();
        }
    }
}

/// Effective DPI of a live window; `None` if Windows cannot determine it.
pub(crate) fn dpi_for_window(hwnd: HWND) -> Option<u32> {
    // SAFETY: `hwnd` is a live window of this process.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    (dpi != 0).then_some(dpi)
}

/// Asks Windows for a `WM_MOUSELEAVE` when the cursor leaves the window.
pub(crate) fn track_mouse_leave(hwnd: HWND) {
    let mut event = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };

    // SAFETY: `hwnd` is a live window of this thread; the struct is valid.
    unsafe {
        TrackMouseEvent(&mut event);
    }
}

pub(crate) fn instance() -> HINSTANCE {
    unsafe { GetModuleHandleW(null()) }
}

pub(crate) fn register_class(name: &[u16], procedure: WNDPROC) -> bool {
    let class = WNDCLASSW {
        style: 0,
        lpfnWndProc: procedure,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance(),
        hIcon: null_mut(),
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: name.as_ptr(),
    };

    unsafe { RegisterClassW(&class) != 0 }
}

pub(crate) fn create_window(
    class: &[u16],
    title: &[u16],
    extended_style: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> HWND {
    let hwnd = unsafe {
        CreateWindowExW(
            extended_style,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            x,
            y,
            width as i32,
            height as i32,
            null_mut(),
            null_mut(),
            instance(),
            null(),
        )
    };
    assert!(!hwnd.is_null(), "CreateWindowExW");

    // CreateWindowExW did not keep the title in the PoC; set it explicitly.
    unsafe {
        SetWindowTextW(hwnd, title.as_ptr());
    }

    hwnd
}

pub(crate) fn set_timer(hwnd: HWND, interval: Option<u32>) {
    unsafe {
        match interval {
            Some(milliseconds) => {
                SetTimer(hwnd, TIMER_ID, milliseconds, None);
            }
            None => {
                KillTimer(hwnd, TIMER_ID);
            }
        }
    }
}

pub(crate) fn post_event(hwnd: SendHwnd) {
    unsafe {
        PostMessageW(hwnd.0, WM_APP_EVENT, 0, 0);
    }
}

/// If the process owns its console (autostart, shortcut),
/// hide it. When launched from a terminal, `GetConsoleProcessList` includes the
/// shell and it is left alone. `HIDEBAR_FOREGROUND=1` keeps it for debugging.
pub(crate) fn hide_console_if_own() {
    if std::env::var_os("HIDEBAR_FOREGROUND").is_some() {
        return;
    }

    let mut processes = [0u32; 2];
    let count = unsafe { GetConsoleProcessList(processes.as_mut_ptr(), processes.len() as u32) };
    if count > 1 {
        return;
    }

    let console = unsafe { GetConsoleWindow() };
    if !console.is_null() {
        unsafe {
            ShowWindow(console, SW_HIDE);
        }
    }
}
