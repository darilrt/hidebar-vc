//! Shared views: drawing of the strip, the calendar and the status panel with iced widgets.
//!
//! The shell decides which surface to draw; there are no Wayland, Win32 or
//! window types here. The same view is used with wgpu (Linux) and tiny-skia (Windows).

mod stretch;
mod view;

use iced::Element;

use crate::state::{Action, Hidebar};

/// Messages produced by interacting with the view (calendar and status panel).
#[derive(Debug, Clone)]
pub(crate) enum Message {
    Month(i32),
    Today,
    Close,
    /// Session action button pressed: arms it, or confirms it if already armed.
    Press(Action),
    /// The cursor left a session action button.
    Disarm,
    /// Click on notification card `id`: its default action.
    #[cfg_attr(windows, allow(dead_code))]
    NotificationClick(u32),
    /// The X of card `id`.
    #[cfg_attr(windows, allow(dead_code))]
    NotificationDismiss(u32),
    /// Action button of card `id`, by key.
    #[cfg_attr(windows, allow(dead_code))]
    NotificationAction(u32, String),
    /// The cursor entered or left card `id`.
    #[cfg_attr(windows, allow(dead_code))]
    NotificationHover(u32, bool),
    /// The "N more" pill.
    #[cfg_attr(windows, allow(dead_code))]
    NotificationsExpand,
}

/// Surface being drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Surface {
    Strip,
    Calendar,
    Panel,
    /// Desktop notification card (ADR-0007, Linux only).
    #[cfg_attr(windows, allow(dead_code))]
    Notification,
}

pub(crate) fn view(state: &Hidebar, surface: Surface) -> Element<'_, Message> {
    view::view(state, surface)
}

#[cfg(target_os = "linux")]
pub(crate) use view::notifications_size;
pub(crate) use view::{CALENDAR_GAP, STRIP_HEIGHT, calendar_size};
pub(crate) use view::{PANEL_SIDE, panel_size};
