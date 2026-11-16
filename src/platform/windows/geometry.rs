//! Geometría del monitor primario y DPI. Todo en píxeles físicos.

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
    MonitorFromPoint,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, MONITORINFOF_PRIMARY, SM_CXSCREEN, SM_CYSCREEN, SPI_GETWORKAREA,
    SystemParametersInfoW,
};

use crate::ui;

#[derive(Debug, Clone, Copy)]
pub(crate) struct WorkArea {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub scale: f32,
}

impl WorkArea {
    pub(crate) fn strip_height(&self) -> u32 {
        (ui::STRIP_HEIGHT as f32 * self.scale).round() as u32
    }

    /// Esquina superior izquierda y tamaño físico del calendario, centrado bajo el reloj.
    pub(crate) fn calendar_rect(&self) -> (i32, i32, u32, u32) {
        let (logical_width, logical_height) = ui::calendar_size();
        let width = (logical_width as f32 * self.scale).round() as u32;
        let height = (logical_height as f32 * self.scale).round() as u32;
        let gap = (ui::CALENDAR_GAP as f32 * self.scale).round() as i32;

        let x = self.left + (self.width.saturating_sub(width) / 2) as i32;
        let y = self.top + self.strip_height() as i32 + gap;

        (x, y, width, height)
    }
}

pub(crate) fn primary() -> WorkArea {
    let monitor = primary_monitor();
    let work = monitor.and_then(work_area).unwrap_or_else(virtual_screen);

    WorkArea {
        left: work.left,
        top: work.top,
        width: (work.right - work.left).max(1) as u32,
        // Valor inicial: `run_daemon` y `relayout` lo sustituyen enseguida por
        // el DPI real de la ventana, que sí sigue los cambios de escala.
        scale: unsafe { GetDpiForSystem() } as f32 / 96.0,
    }
}

/// `MonitorFromPoint(0, 0)` asume que el primario está en el origen; con
/// varios monitores o escritorios virtuales conviene preguntar por el flag.
fn primary_monitor() -> Option<HMONITOR> {
    let mut found: HMONITOR = null_mut();

    // SAFETY: el callback sólo escribe en `found`, que vive durante la llamada.
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(find_primary),
            &mut found as *mut HMONITOR as LPARAM,
        );
    }

    if found.is_null() {
        // SAFETY: sin callback; `MONITOR_DEFAULTTOPRIMARY` garantiza un monitor
        // mientras exista un escritorio.
        found = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    }

    (!found.is_null()).then_some(found)
}

/// Callback de `EnumDisplayMonitors`: guarda el primer monitor `PRIMARY` y
/// detiene la enumeración devolviendo 0.
unsafe extern "system" fn find_primary(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> windows_sys::core::BOOL {
    // SAFETY: `lparam` es el `*mut HMONITOR` que pasa `primary_monitor`.
    let found = unsafe { &mut *(lparam as *mut HMONITOR) };

    // SAFETY: `MONITORINFO` con `cbSize` y un monitor entregado por el sistema.
    let mut info: MONITORINFO = unsafe { zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return 1;
    }

    if info.dwFlags & MONITORINFOF_PRIMARY == 0 {
        return 1;
    }

    *found = monitor;
    0
}

fn work_area(monitor: HMONITOR) -> Option<RECT> {
    // SAFETY: `MONITORINFO` con `cbSize`; el monitor viene del sistema.
    let mut info: MONITORINFO = unsafe { zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;

    // SAFETY: el out es válido y el monitor sigue existiendo durante la llamada.
    (unsafe { GetMonitorInfoW(monitor, &mut info) } != 0).then_some(info.rcWork)
}

fn virtual_screen() -> RECT {
    let mut work: RECT = unsafe { zeroed() };

    // SAFETY: `work` es un out válido; SPI_GETWORKAREA sólo escribe en él.
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut RECT as *mut c_void, 0)
    };
    if ok != 0 {
        return work;
    }

    // SAFETY: métricas globales sin punteros.
    RECT {
        left: 0,
        top: 0,
        right: unsafe { GetSystemMetrics(SM_CXSCREEN) },
        bottom: unsafe { GetSystemMetrics(SM_CYSCREEN) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(scale: f32) -> WorkArea {
        WorkArea {
            left: 0,
            top: 0,
            width: 1920,
            scale,
        }
    }

    #[test]
    fn strip_height_follows_scale() {
        assert_eq!(area(1.0).strip_height(), ui::STRIP_HEIGHT);
        assert_eq!(area(1.25).strip_height(), 43);
        assert_eq!(area(1.5).strip_height(), 51);
        assert_eq!(area(2.0).strip_height(), ui::STRIP_HEIGHT * 2);
    }

    #[test]
    fn calendar_is_centered_and_scaled() {
        let (logical_width, logical_height) = ui::calendar_size();
        let area = area(1.5);
        let (x, y, width, height) = area.calendar_rect();

        assert_eq!(width, (logical_width as f32 * 1.5).round() as u32);
        assert_eq!(height, (logical_height as f32 * 1.5).round() as u32);
        assert_eq!(x, area.left + (area.width - width) as i32 / 2);
        assert_eq!(
            y,
            area.top + area.strip_height() as i32 + (ui::CALENDAR_GAP as f32 * 1.5).round() as i32
        );
    }

    #[test]
    fn calendar_clamps_to_the_left_when_work_area_is_narrow() {
        let (_, _, width, _) = area(1.0).calendar_rect();
        let narrow = WorkArea {
            left: 100,
            top: 7,
            width: width / 2,
            scale: 1.0,
        };
        let (x, y, _, _) = narrow.calendar_rect();
        assert_eq!(x, 100);
        assert_eq!(y, 7 + narrow.strip_height() as i32 + ui::CALENDAR_GAP);
    }
}
