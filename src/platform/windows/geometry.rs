//! Primary monitor geometry and DPI. Everything in physical pixels.

use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

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

    /// Top-left corner and physical size of the calendar, centered below the clock.
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
    // SAFETY: `MONITORINFO` with `cbSize`; the monitor belongs to the system and the out is valid.
    let mut info: MONITORINFO = unsafe { zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;

    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    let work = if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
        info.rcWork
    } else {
        RECT {
            left: 0,
            top: 0,
            right: unsafe { GetSystemMetrics(SM_CXSCREEN) },
            bottom: unsafe { GetSystemMetrics(SM_CYSCREEN) },
        }
    };

    WorkArea {
        left: work.left,
        top: work.top,
        width: (work.right - work.left).max(1) as u32,
        scale: unsafe { GetDpiForSystem() } as f32 / 96.0,
    }
}
