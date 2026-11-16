//! Shared views: drawing of the strip, the calendar and the status panel with iced widgets.
//!
//! The shell decides which surface to draw; there are no Wayland, Win32 or
//! window types here. The same view is used with wgpu (Linux) and tiny-skia (Windows).

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
}

/// Surface being drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Surface {
    Strip,
    Calendar,
    Panel,
}

pub(crate) fn view(state: &Hidebar, surface: Surface) -> Element<'_, Message> {
    view::view(state, surface)
}

pub(crate) use view::{CALENDAR_GAP, STRIP_HEIGHT, calendar_size};
pub(crate) use view::{PANEL_SIDE, panel_size};
