//! Surface drawing: strip pills (modules and notices) and the calendar.

use chrono::Datelike;
use std::sync::LazyLock;

use iced::widget::{Column, Row, button, column, container, row, space, svg, text};
use iced::{Alignment, Border, Color, Element, Length, Padding};

use super::{Message, Surface};
use crate::calendar::{self, WEEKDAYS_SHORT};
use crate::event::{Audio, Battery, BatteryStatus, Network, System, Workspaces};
use crate::state::{Hidebar, Module, Notice, Severity, severity};

/// Height of the strip surface, in logical pixels.
pub(crate) const STRIP_HEIGHT: u32 = 34;
const PILL_BACKGROUND: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.6);
/// Pills: height, gap from the top edge and from the sides, horizontal padding.
const PILL_HEIGHT: f32 = 26.0;
const PILL_TOP: f32 = 6.0;
const PILL_SIDE: f32 = 12.0;
const PILL_PADDING: f32 = 12.0;
const DOT_SIZE: f32 = 7.0;

const DIM: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.6);
const WARNING_COLOR: Color = Color::from_rgb(1.0, 0.65, 0.25);
const CRITICAL_COLOR: Color = Color::from_rgb(1.0, 0.42, 0.21);
const CHARGING_COLOR: Color = Color::from_rgb(1.0, 0.82, 0.25);

/// Own icons (`assets/icons/`), embedded in the binary and tinted when drawn.
const ICON_SIZE: f32 = 14.0;
static ICON_NET: LazyLock<svg::Handle> =
    LazyLock::new(|| icon(include_bytes!("../../assets/icons/net.svg")));
static ICON_NET_OFF: LazyLock<svg::Handle> =
    LazyLock::new(|| icon(include_bytes!("../../assets/icons/net-off.svg")));
static ICON_CPU: LazyLock<svg::Handle> =
    LazyLock::new(|| icon(include_bytes!("../../assets/icons/cpu.svg")));
static ICON_RAM: LazyLock<svg::Handle> =
    LazyLock::new(|| icon(include_bytes!("../../assets/icons/ram.svg")));
static ICON_MUTED: LazyLock<svg::Handle> =
    LazyLock::new(|| icon(include_bytes!("../../assets/icons/muted.svg")));
static ICON_SPEAKER: LazyLock<svg::Handle> =
    LazyLock::new(|| icon(include_bytes!("../../assets/icons/speaker.svg")));

fn icon(bytes: &'static [u8]) -> svg::Handle {
    svg::Handle::from_memory(bytes)
}

/// Calendar: cells, padding and gap below the strip.
const CELL: f32 = 28.0;
const CALENDAR_PADDING: f32 = 12.0;
const CALENDAR_HEADER: f32 = 28.0;
pub(crate) const CALENDAR_GAP: i32 = 4;
const CALENDAR_BACKGROUND: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.8);

pub(crate) fn view(state: &Hidebar, surface: Surface) -> Element<'_, Message> {
    match surface {
        Surface::Strip => strip_view(state),
        Surface::Calendar => calendar_view(state),
    }
}

pub(crate) fn calendar_size() -> (u32, u32) {
    let width = 7.0 * CELL + 2.0 * CALENDAR_PADDING;
    // Title, weekday row and the weeks.
    let height = CALENDAR_HEADER + CELL * (1 + calendar::WEEKS) as f32 + 2.0 * CALENDAR_PADDING;
    (width as u32, height as u32)
}

fn strip_view(state: &Hidebar) -> Element<'_, Message> {
    let module = |module: Module, content: Option<Element<'static, Message>>| match content {
        Some(content) if state.visible(module) => pill(content),
        _ => space().into(),
    };
    let left = module(
        Module::Workspaces,
        state.workspaces.as_ref().map(workspaces_view),
    );
    let center = module(Module::Clock, Some(clock_view(state)));
    // On the right, the full `status` or, if hidden, the active notices.
    let notices = state.active_notices();
    let right = if state.visible(Module::Status) {
        pill(status_view(state))
    } else if !notices.is_empty() {
        pill(notices_view(state, &notices))
    } else {
        space().into()
    };

    // Both sides take the same width, so the center stays centered.
    // Only the pills have a background; the rest of the surface is transparent.
    let line = row![
        container(left).width(Length::Fill),
        center,
        container(right).align_right(Length::Fill),
    ]
    .align_y(Alignment::Center);

    container(line)
        .width(Length::Fill)
        .align_top(Length::Fill)
        .padding(Padding {
            top: PILL_TOP,
            right: PILL_SIDE,
            bottom: 0.0,
            left: PILL_SIDE,
        })
        .into()
}

/// Pill-shaped background around an element.
fn pill(content: Element<'static, Message>) -> Element<'static, Message> {
    container(content)
        .center_y(PILL_HEIGHT)
        .padding([0.0, PILL_PADDING])
        .style(|_| container::Style {
            background: Some(PILL_BACKGROUND.into()),
            border: Border::default().rounded(PILL_HEIGHT / 2.0),
            ..Default::default()
        })
        .into()
}

fn clock_view(state: &Hidebar) -> Element<'static, Message> {
    row![
        text(state.now.time.clone()).size(16).color(Color::WHITE),
        text(state.now.date.clone()).size(11).color(DIM),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// One dot per workspace; the active one filled.
fn workspaces_view(workspaces: &Workspaces) -> Element<'static, Message> {
    let dots = workspaces
        .ids
        .iter()
        .map(|id| circle(DOT_SIZE, Color::WHITE, *id == workspaces.active));
    Row::with_children(dots).spacing(6).into()
}

/// network · cpu · ram · mute · battery. Anything without a reading yet is not shown;
/// mute, only when nothing is audible.
fn status_view(state: &Hidebar) -> Element<'static, Message> {
    let mut line = Row::new().spacing(12).align_y(Alignment::Center);
    if let Some(network) = state.network {
        line = line.push(network_view(network));
    }
    if let Some(System { cpu, ram }) = state.system {
        line = line
            .push(metric(&ICON_CPU, cpu))
            .push(metric(&ICON_RAM, ram));
    }
    if state.audio.is_some_and(|audio: Audio| audio.silent()) {
        line = line.push(tinted(&ICON_MUTED, Color::WHITE));
    }
    if let Some(battery) = &state.battery {
        line = line.push(battery_view(battery, state.pulse()));
    }
    line.into()
}

/// Notices together in one pill, looking the same as in `status`. Battery: icon only.
fn notices_view(state: &Hidebar, notices: &[Notice]) -> Element<'static, Message> {
    let items = notices.iter().filter_map(|notice| match notice {
        Notice::Network => state.network.map(network_view),
        Notice::Audio => state.audio.map(audio_view),
        Notice::Battery => state
            .battery
            .as_ref()
            .map(|battery| battery_icon(battery, state.pulse())),
    });
    Row::with_children(items)
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
}

/// Crossed-out speaker when silent; regular speaker with sound.
fn audio_view(audio: Audio) -> Element<'static, Message> {
    let icon = if audio.silent() {
        &ICON_MUTED
    } else {
        &ICON_SPEAKER
    };
    tinted(icon, Color::WHITE)
}

/// White wifi with internet, orange with no way out, crossed-out orange-red with no network.
fn network_view(network: Network) -> Element<'static, Message> {
    let (icon, color) = match network {
        Network::Online => (&ICON_NET, Color::WHITE),
        Network::NoInternet => (&ICON_NET, WARNING_COLOR),
        Network::Offline => (&ICON_NET_OFF, CRITICAL_COLOR),
    };
    tinted(icon, color)
}

/// Dim icon + "12 %" in white.
fn metric(icon: &svg::Handle, percent: u8) -> Element<'static, Message> {
    row![
        tinted(icon, DIM),
        text(format!("{percent} %")).size(12).color(Color::WHITE),
    ]
    .spacing(4)
    .align_y(Alignment::Center)
    .into()
}

fn tinted(icon: &svg::Handle, color: Color) -> Element<'static, Message> {
    svg(icon.clone())
        .width(ICON_SIZE)
        .height(ICON_SIZE)
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}

/// Filled circle or outline only.
fn circle(size: f32, color: Color, filled: bool) -> Element<'static, Message> {
    container(space())
        .width(size)
        .height(size)
        .style(move |_| container::Style {
            background: filled.then_some(color.into()),
            border: Border {
                color,
                width: 1.5,
                radius: (size / 2.0).into(),
            },
            ..Default::default()
        })
        .into()
}

/// "charging 80 %" + icon. Same states as the battery notice: yellow while charging,
/// white, or pulsing orange-red with low or critical battery.
fn battery_view(battery: &Battery, pulse: f32) -> Element<'static, Message> {
    let color = battery_shade(battery, pulse);
    let mut line = Row::new().spacing(6).align_y(Alignment::Center);
    if battery.status == BatteryStatus::Charging {
        line = line.push(text("charging").size(11).color(CHARGING_COLOR));
    }
    line.push(text(format!("{} %", battery.percent)).size(12).color(color))
        .push(battery_icon(battery, pulse))
        .into()
}

/// Battery color with the pulse applied when low.
fn battery_shade(battery: &Battery, pulse: f32) -> Color {
    let color = battery_color(battery);
    if severity(battery) == Severity::Normal {
        color
    } else {
        fade(color, pulse)
    }
}

/// Battery body filled according to the level, plus the terminal.
fn battery_icon(battery: &Battery, pulse: f32) -> Element<'static, Message> {
    const BODY_WIDTH: f32 = 20.0;
    const BODY_HEIGHT: f32 = 10.0;
    const BORDER: f32 = 1.2;
    const PADDING: f32 = 1.5;
    let inner = BODY_WIDTH - 2.0 * (BORDER + PADDING);
    let level = inner * f32::from(battery.percent) / 100.0;
    let color = battery_shade(battery, pulse);

    let fill = container(space())
        .width(level)
        .height(Length::Fill)
        .style(move |_| container::Style {
            background: Some(color.into()),
            border: Border::default().rounded(1.0),
            ..Default::default()
        });
    let body = container(fill)
        .width(BODY_WIDTH)
        .height(BODY_HEIGHT)
        .padding(PADDING)
        .style(|_| container::Style {
            border: Border {
                color: Color::WHITE,
                width: BORDER,
                radius: 2.5.into(),
            },
            ..Default::default()
        });
    let nub = container(space())
        .width(2.0)
        .height(4.0)
        .style(|_| container::Style {
            background: Some(Color::WHITE.into()),
            border: Border::default().rounded(1.0),
            ..Default::default()
        });

    row![body, nub].align_y(Alignment::Center).into()
}

/// ‹ month year ›, Mo–Su headers and the grid; today highlighted.
fn calendar_view(state: &Hidebar) -> Element<'static, Message> {
    let Some(open) = &state.calendar else {
        return space().into();
    };
    let month = open.month;
    let today = state.now.today;
    let is_current_month = today.year() == month.year() && today.month() == month.month();

    let title = button(
        text(calendar::month_title(month))
            .size(13)
            .width(Length::Fill)
            .center(),
    )
    .on_press(Message::Today)
    .width(Length::Fill)
    .style(flat_button);
    let header = row![
        nav_button("‹", Message::Month(-1)),
        title,
        nav_button("›", Message::Month(1)),
    ]
    .height(CALENDAR_HEADER)
    .align_y(Alignment::Center);

    let weekdays = Row::with_children(
        WEEKDAYS_SHORT
            .iter()
            .map(|day| cell(text(*day).size(11).color(DIM).into())),
    );

    let weeks = calendar::month_grid(month).map(|week| {
        Row::with_children(week.iter().map(|day| match day {
            Some(day) if is_current_month && *day == today.day() => cell(today_mark(*day)),
            Some(day) => cell(text(day.to_string()).size(12).color(Color::WHITE).into()),
            None => cell(space().into()),
        }))
        .into()
    });

    container(column![header, weekdays, Column::with_children(weeks)])
        .padding(CALENDAR_PADDING)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style {
            background: Some(CALENDAR_BACKGROUND.into()),
            border: Border::default().rounded(14.0),
            ..Default::default()
        })
        .into()
}

fn cell(content: Element<'static, Message>) -> Element<'static, Message> {
    container(content).center(CELL).into()
}

/// Today: dark number on a white circle.
fn today_mark(day: u32) -> Element<'static, Message> {
    container(text(day.to_string()).size(12).color(Color::BLACK))
        .center(CELL - 4.0)
        .style(|_| container::Style {
            background: Some(Color::WHITE.into()),
            border: Border::default().rounded((CELL - 4.0) / 2.0),
            ..Default::default()
        })
        .into()
}

fn nav_button(label: &'static str, message: Message) -> Element<'static, Message> {
    button(text(label).size(16).center())
        .on_press(message)
        .width(CELL)
        .style(flat_button)
        .into()
}

/// Button without background; a slight highlight on hover.
fn flat_button(_: &iced::Theme, status: button::Status) -> button::Style {
    let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hover.then_some(Color::from_rgba(1.0, 1.0, 1.0, 0.12).into()),
        text_color: Color::WHITE,
        border: Border::default().rounded(8.0),
        ..Default::default()
    }
}

/// Battery color: yellow while charging; otherwise by severity.
fn battery_color(battery: &Battery) -> Color {
    if battery.status == BatteryStatus::Charging {
        CHARGING_COLOR
    } else {
        severity_color(severity(battery))
    }
}

fn severity_color(severity: Severity) -> Color {
    match severity {
        Severity::Critical => CRITICAL_COLOR,
        Severity::Normal | Severity::Low => Color::WHITE,
    }
}

fn fade(color: Color, alpha: f32) -> Color {
    Color {
        a: color.a * alpha,
        ..color
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn battery(percent: u8, status: BatteryStatus) -> Battery {
        Battery { percent, status }
    }

    #[test]
    fn charging_is_yellow() {
        use BatteryStatus::*;
        assert_eq!(battery_color(&battery(50, Charging)), CHARGING_COLOR);
        assert_eq!(battery_color(&battery(5, Charging)), CHARGING_COLOR);
        assert_eq!(battery_color(&battery(50, Full)), Color::WHITE);
        assert_eq!(battery_color(&battery(5, Discharging)), CRITICAL_COLOR);
    }

    #[test]
    fn calendar_fits_six_weeks() {
        let (width, height) = calendar_size();
        assert_eq!(width, 220);
        assert_eq!(height, 28 + 28 * 7 + 24);
    }
}
