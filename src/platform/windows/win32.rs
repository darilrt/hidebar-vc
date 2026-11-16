//! Helpers Win32 del shell. Todo unsafe está confinado aquí o en `render`.

use std::mem::size_of;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HINSTANCE, HWND};
use windows_sys::Win32::System::Console::{GetConsoleProcessList, GetConsoleWindow};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, IDC_ARROW, KillTimer, LoadCursorW, PostMessageW, RegisterClassW, SW_HIDE,
    SetProcessDPIAware, SetTimer, SetWindowTextW, ShowWindow, WNDCLASSW, WNDPROC, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

pub(crate) const WM_APP_EVENT: u32 = 0x8000 + 1;
/// Mensajes enviados por el sistema (no posteados) que el WndProc reenvía a la cola.
pub(crate) const WM_APP_FOCUS: u32 = WM_APP_EVENT + 1;
pub(crate) const WM_APP_RELAYOUT: u32 = WM_APP_EVENT + 2;
pub(crate) const WM_APP_CAPTURE_LOST: u32 = WM_APP_EVENT + 3;
/// `WM_MOUSELEAVE` vive en `Win32::UI::Controls`; evitamos una feature extra.
pub(crate) const WM_MOUSELEAVE: u32 = 0x02A3;
pub(crate) const TIMER_ID: usize = 1;

/// Handle de ventana que puede enviarse a los hilos de fuentes/IPC.
/// `PostMessageW` es seguro desde cualquier hilo mientras la ventana exista.
#[derive(Clone, Copy)]
pub(crate) struct SendHwnd(pub HWND);

// SAFETY: el handle solo se usa para `PostMessageW`; la ventana vive hasta que
// termina el proceso.
unsafe impl Send for SendHwnd {}
unsafe impl Sync for SendHwnd {}

pub(crate) const STRIP_EX_STYLE: u32 =
    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST;

pub(crate) const CALENDAR_EX_STYLE: u32 = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;

pub(crate) fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Declara Per-Monitor V2 antes de consultar DPI o crear ventanas. Sin esto
/// Windows trata el proceso como DPI-unaware: `GetDpiForSystem` devuelve 96 y
/// DWM estira las ventanas, con textos borrosos a 125–150 %.
pub(crate) fn enable_dpi_awareness() {
    // SAFETY: se llama una sola vez al principio del proceso, antes de crear
    // ventanas o consultar DPI.
    let ok = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if ok == 0 {
        // SAFETY: fallback de la misma inicialización de DPI; no crea recursos
        // y se ejecuta antes de cualquier ventana o consulta de DPI.
        unsafe {
            SetProcessDPIAware();
        }
    }
}

/// DPI efectivo de una ventana viva; `None` si Windows no lo puede determinar.
pub(crate) fn dpi_for_window(hwnd: HWND) -> Option<u32> {
    // SAFETY: `hwnd` es una ventana viva de este proceso.
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    (dpi != 0).then_some(dpi)
}

/// Captura el ratón mientras hay un botón pulsado: así el `WM_LBUTTONUP` llega
/// aunque el cursor salga del calendario y la vista no se queda "pegada".
pub(crate) fn capture_mouse(hwnd: HWND) {
    // SAFETY: `hwnd` es una ventana viva de este hilo.
    unsafe {
        SetCapture(hwnd);
    }
}

/// Suelta la captura si la tiene este hilo; es idempotente.
pub(crate) fn release_capture() {
    // SAFETY: sin punteros; sólo libera la captura del hilo actual.
    unsafe {
        ReleaseCapture();
    }
}

/// Pide a Windows un `WM_MOUSELEAVE` cuando el cursor deja la ventana.
pub(crate) fn track_mouse_leave(hwnd: HWND) {
    let mut event = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };

    // SAFETY: `hwnd` es una ventana viva de este hilo; el struct es válido.
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

    // CreateWindowExW no dejó el título en el PoC; fijarlo explícitamente.
    unsafe {
        SetWindowTextW(hwnd, title.as_ptr());
    }

    hwnd
}

pub(crate) fn set_timer(hwnd: HWND, interval: Option<u32>) -> bool {
    // SAFETY: `hwnd` es una ventana viva de este hilo; el id es propio y no se
    // usa para nada más. `KillTimer` sobre un timer inexistente es inocuo.
    unsafe {
        match interval {
            Some(milliseconds) => SetTimer(hwnd, TIMER_ID, milliseconds, None) != 0,
            None => {
                KillTimer(hwnd, TIMER_ID);
                true
            }
        }
    }
}

pub(crate) fn post_event(hwnd: SendHwnd) {
    unsafe {
        PostMessageW(hwnd.0, WM_APP_EVENT, 0, 0);
    }
}

/// Si el proceso posee su propia consola (autoarranque, acceso directo),
/// ocultarla. Lanzado desde una terminal, `GetConsoleProcessList` incluye al
/// shell y no se toca. `HIDEBAR_FOREGROUND=1` la mantiene para depurar.
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
