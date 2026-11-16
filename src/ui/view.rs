//! Surface drawing: strip pills (modules and notices), the calendar and the status panel.

use chrono::Datelike;
use std::sync::LazyLock;

use iced::widget::{Column, Row, button, column, container, mouse_area, row, space, svg, text};
use iced::{Alignment, Border, Color, Element, Length, Padding};

use super::stretch::Stretch;
use super::{Message, Surface};
use crate::calendar::{self, WEEKDAYS_SHORT};
use crate::event::{Audio, Battery, BatteryStatus, Network, System, Workspaces};
use crate::state::{Action, Frame, Hidebar, Module, Notice, NoticeFrame, Severity, severity};

/// Height of the strip surface, in logical pixels.
pub(crate) const STRIP_HEIGHT: u32 = 34;
const PILL_BACKGROUND: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.6);
/// Pills: height, gap from the top edge and from the sides, horizontal padding.
const PILL_HEIGHT: f32 = 26.0;
const PILL_TOP: f32 = 6.0;
const PILL_SIDE: f32 = 12.0;
const PILL_PADDING: f32 = 12.0;
const DOT_SIZE: f32 = 7.0;
/// Notices (ADR-0006): gap between icons, as in `status`, and how far each icon slides in from the right.
const NOTICE_GAP: f32 = 12.0;
const NOTICE_SLIDE: f32 = 6.0;
/// Battery icon: body plus terminal.
const BATTERY_ICON_WIDTH: f32 = 22.0;

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
static ICON_POWER: LazyLock<svg::Handle> =
    LazyLock::new(|| icon(include_bytes!("../../assets/icons/power.svg")));
static ICON_LOGOUT: LazyLock<svg::Handle> =
    LazyLock::new(|| icon(include_bytes!("../../assets/icons/logout.svg")));

fn icon(bytes: &'static [u8]) -> svg::Handle {
    svg::Handle::from_memory(bytes)
}

/// Calendar: cells, padding and gap below the strip.
const CELL: f32 = 28.0;
const CALENDAR_PADDING: f32 = 12.0;
const CALENDAR_HEADER: f32 = 28.0;
pub(crate) const CALENDAR_GAP: i32 = 4;
const CALENDAR_BACKGROUND: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.8);

/// Status panel (ADR-0005): width, padding, gap between sections and the heights of each section.
const PANEL_WIDTH: f32 = 300.0;
const PANEL_PADDING: f32 = 14.0;
const PANEL_GAP: f32 = 14.0;
const METER_LABEL: f32 = 18.0;
const METER_BAR: f32 = 4.0;
const METER_WARNING: u8 = 90;
const INFO_ROW: f32 = 20.0;
const INFO_ROWS: f32 = 3.0;
const INFO_GAP: f32 = 10.0;
const ACTION_BUTTON: f32 = 44.0;
const ACTION_ICON: f32 = 18.0;
/// Gap between the panel and the right screen edge: aligned with the `status` pill.
pub(crate) const PANEL_SIDE: i32 = PILL_SIDE as i32;

pub(crate) fn view(state: &Hidebar, surface: Surface) -> Element<'_, Message> {
    match surface {
        Surface::Strip => strip_view(state),
        Surface::Calendar => calendar_view(state),
        Surface::Panel => panel_view(state),
    }
}

/// Meters, information rows, separator and action buttons.
fn panel_height() -> f32 {
    let meters = METER_LABEL + 6.0 + METER_BAR;
    let info = INFO_ROW * INFO_ROWS + INFO_GAP * (INFO_ROWS - 1.0);
    2.0 * PANEL_PADDING + meters + info + 1.0 + ACTION_BUTTON + 3.0 * PANEL_GAP
}

pub(crate) fn panel_size() -> (u32, u32) {
    (PANEL_WIDTH as u32, panel_height().ceil() as u32)
}

pub(crate) fn calendar_size() -> (u32, u32) {
    let width = 7.0 * CELL + 2.0 * CALENDAR_PADDING;
    // Title, weekday row and the weeks.
    let height = CALENDAR_HEADER + CELL * (1 + calendar::WEEKS) as f32 + 2.0 * CALENDAR_PADDING;
    (width as u32, height as u32)
}

fn strip_view(state: &Hidebar) -> Element<'_, Message> {
    // Module pills stretch in and out (ADR-0006); the content shows once stretched.
    let module =
        |module: Module, content: fn(&Hidebar, f32) -> Option<Element<'static, Message>>| {
            state
                .module_frame(module)
                .and_then(|frame| Some(stretched(content(state, frame.reveal)?, frame)))
        };
    let left = module(Module::Workspaces, |state, alpha| {
        state
            .workspaces
            .as_ref()
            .map(|workspaces| workspaces_view(workspaces, alpha))
    })
    .unwrap_or_else(|| space().into());
    let center = module(Module::Clock, |state, alpha| Some(clock_view(state, alpha)))
        .unwrap_or_else(|| space().into());
    // On the right, the full `status` or, if hidden, the notices on screen.
    let notices = state.notice_frames();
    let status = module(Module::Status, |state, alpha| {
        Some(status_view(state, alpha))
    });
    let right = if let Some(status) = status {
        status
    } else if !notices.is_empty() {
        notices_pill(state, &notices)
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

/// Pill-shaped background around an element, at the width and opacity of `frame`.
fn stretched(content: Element<'static, Message>, frame: Frame) -> Element<'static, Message> {
    Stretch::new(
        content,
        frame.size,
        fade(PILL_BACKGROUND, frame.background),
        PILL_HEIGHT,
        Padding::from([0.0, PILL_PADDING]),
    )
    .into()
}

fn clock_view(state: &Hidebar, alpha: f32) -> Element<'static, Message> {
    row![
        text(state.now.time.clone())
            .size(16)
            .color(fade(Color::WHITE, alpha)),
        text(state.now.date.clone())
            .size(11)
            .color(fade(DIM, alpha)),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// One dot per workspace; the active one filled.
fn workspaces_view(workspaces: &Workspaces, alpha: f32) -> Element<'static, Message> {
    let color = fade(Color::WHITE, alpha);
    let dots = workspaces
        .ids
        .iter()
        .map(|id| circle(DOT_SIZE, color, *id == workspaces.active));
    Row::with_children(dots).spacing(6).into()
}

/// network · cpu · ram · mute · battery. Anything without a reading yet is not shown;
/// mute, only when nothing is audible.
fn status_view(state: &Hidebar, alpha: f32) -> Element<'static, Message> {
    let mut line = Row::new().spacing(12).align_y(Alignment::Center);
    if let Some(network) = state.network {
        line = line.push(network_view(network, alpha));
    }
    if let Some(System { cpu, ram }) = state.system {
        line = line
            .push(metric(&ICON_CPU, cpu, alpha))
            .push(metric(&ICON_RAM, ram, alpha));
    }
    if state.audio.is_some_and(|audio: Audio| audio.silent()) {
        line = line.push(faded(&ICON_MUTED, Color::WHITE, alpha));
    }
    if let Some(battery) = &state.battery {
        line = line.push(battery_view(battery, state.pulse(), alpha));
    }
    line.into()
}

/// Notices together in one pill, looking the same as in `status` (battery: icon only), animated (ADR-0006).
///
/// The width is computed, not measured: each notice takes `size` of its box and the pill is anchored on
/// its right edge, so it stretches leftwards. In its box, the icon is aligned right, followed by a spacer
/// that grows to `NOTICE_SLIDE` as it appears: the icon slides in from the right. The right padding gives
/// that spacer back, so a settled pill looks like before.
fn notices_pill(state: &Hidebar, notices: &[NoticeFrame]) -> Element<'static, Message> {
    let background = notices
        .iter()
        .map(|frame| frame.background)
        .fold(0.0, f32::max);
    let mut content = 0.0;
    let mut items = Row::new().align_y(Alignment::Center);
    for (index, frame) in notices.iter().enumerate() {
        let Some((icon, width)) = notice_icon(state, frame.notice, frame.reveal) else {
            continue;
        };
        let lead = if index == 0 {
            0.0
        } else {
            NOTICE_GAP - NOTICE_SLIDE
        };
        let slot = frame.size * (lead + width + NOTICE_SLIDE);
        content += slot;
        // Not drawn until it starts appearing: a narrower box would squeeze it.
        let icon = if frame.reveal > 0.0 {
            icon
        } else {
            space().into()
        };
        items = items.push(
            container(row![icon, space().width(NOTICE_SLIDE * frame.reveal)])
                .width(slot)
                .align_right(slot)
                .clip(true),
        );
    }

    let padding = Padding {
        top: 0.0,
        right: PILL_PADDING - NOTICE_SLIDE,
        bottom: 0.0,
        left: PILL_PADDING,
    };
    let width = (padding.left + padding.right + content).max(PILL_HEIGHT);
    container(items)
        .width(width)
        .center_y(PILL_HEIGHT)
        .align_right(width)
        .padding(padding)
        .clip(true)
        .style(move |_| container::Style {
            background: Some(fade(PILL_BACKGROUND, background).into()),
            border: Border::default().rounded(PILL_HEIGHT / 2.0),
            ..Default::default()
        })
        .into()
}

/// The notice's icon at opacity `alpha`, and its width; `None` without a reading.
fn notice_icon(
    state: &Hidebar,
    notice: Notice,
    alpha: f32,
) -> Option<(Element<'static, Message>, f32)> {
    match notice {
        Notice::Network => state
            .network
            .map(|network| (network_view(network, alpha), ICON_SIZE)),
        Notice::Audio => state
            .audio
            .map(|audio| (audio_view(audio, alpha), ICON_SIZE)),
        Notice::Battery => state.battery.as_ref().map(|battery| {
            (
                battery_icon(battery, state.pulse(), alpha),
                BATTERY_ICON_WIDTH,
            )
        }),
    }
}

/// Crossed-out speaker when silent; regular speaker with sound.
fn audio_view(audio: Audio, alpha: f32) -> Element<'static, Message> {
    let icon = if audio.silent() {
        &ICON_MUTED
    } else {
        &ICON_SPEAKER
    };
    faded(icon, Color::WHITE, alpha)
}

/// White wifi with internet, orange with no way out, crossed-out orange-red with no network.
fn network_view(network: Network, alpha: f32) -> Element<'static, Message> {
    let (icon, color) = match network {
        Network::Online => (&ICON_NET, Color::WHITE),
        Network::NoInternet => (&ICON_NET, WARNING_COLOR),
        Network::Offline => (&ICON_NET_OFF, CRITICAL_COLOR),
    };
    faded(icon, color, alpha)
}

/// Dim icon + "12 %" in white.
fn metric(icon: &svg::Handle, percent: u8, alpha: f32) -> Element<'static, Message> {
    row![
        faded(icon, DIM, alpha),
        text(format!("{percent} %"))
            .size(12)
            .color(fade(Color::WHITE, alpha)),
    ]
    .spacing(4)
    .align_y(Alignment::Center)
    .into()
}

fn tinted(icon: &svg::Handle, color: Color) -> Element<'static, Message> {
    faded(icon, color, 1.0)
}

/// Tinted icon at opacity `alpha`. The renderers ignore the tint's alpha, so it goes through `opacity`.
fn faded(icon: &svg::Handle, color: Color, alpha: f32) -> Element<'static, Message> {
    svg(icon.clone())
        .width(ICON_SIZE)
        .height(ICON_SIZE)
        .opacity(alpha)
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
fn battery_view(battery: &Battery, pulse: f32, alpha: f32) -> Element<'static, Message> {
    let color = fade(battery_shade(battery, pulse), alpha);
    let mut line = Row::new().spacing(6).align_y(Alignment::Center);
    if battery.status == BatteryStatus::Charging {
        line = line.push(text("charging").size(11).color(fade(CHARGING_COLOR, alpha)));
    }
    line.push(text(format!("{} %", battery.percent)).size(12).color(color))
        .push(battery_icon(battery, pulse, alpha))
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

/// Battery body filled according to the level, plus the terminal. `alpha` fades the outline too;
/// `pulse` only the fill.
fn battery_icon(battery: &Battery, pulse: f32, alpha: f32) -> Element<'static, Message> {
    const BODY_WIDTH: f32 = BATTERY_ICON_WIDTH - 2.0;
    const BODY_HEIGHT: f32 = 10.0;
    const BORDER: f32 = 1.2;
    const PADDING: f32 = 1.5;
    let inner = BODY_WIDTH - 2.0 * (BORDER + PADDING);
    let level = inner * f32::from(battery.percent) / 100.0;
    let color = fade(battery_shade(battery, pulse), alpha);
    let outline = fade(Color::WHITE, alpha);

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
        .style(move |_| container::Style {
            border: Border {
                color: outline,
                width: BORDER,
                radius: 2.5.into(),
            },
            ..Default::default()
        });
    let nub = container(space())
        .width(2.0)
        .height(4.0)
        .style(move |_| container::Style {
            background: Some(outline.into()),
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

/// Status panel: CPU/RAM meters, battery, network and audio rows, and the session actions.
fn panel_view(state: &Hidebar) -> Element<'static, Message> {
    let Some(panel) = &state.panel else {
        return space().into();
    };

    let meters = match state.system {
        Some(System { cpu, ram }) => {
            row![meter(&ICON_CPU, "CPU", cpu), meter(&ICON_RAM, "RAM", ram)]
                .spacing(PANEL_GAP)
                .into()
        }
        None => Element::from(space().height(METER_LABEL + 6.0 + METER_BAR)),
    };

    let mut info = Column::new()
        .spacing(INFO_GAP)
        .height(INFO_ROW * INFO_ROWS + INFO_GAP * (INFO_ROWS - 1.0));
    if let Some(battery) = &state.battery {
        let color = battery_shade(battery, state.pulse());
        let mut value = Row::new()
            .spacing(4)
            .push(text(format!("{} %", battery.percent)).size(13).color(color));
        if let Some((label, label_color)) = battery_label(battery) {
            value = value
                .push(text("·").size(13).color(DIM))
                .push(text(label).size(13).color(label_color));
        }
        info = info.push(info_row(
            battery_icon(battery, state.pulse(), 1.0),
            "Battery",
            value.into(),
        ));
    }
    if let Some(network) = state.network {
        let (label, color) = match network {
            Network::Online => ("Online", Color::WHITE),
            Network::NoInternet => ("No internet", WARNING_COLOR),
            Network::Offline => ("Offline", CRITICAL_COLOR),
        };
        info = info.push(info_row(
            network_view(network, 1.0),
            "Network",
            text(label).size(13).color(color).into(),
        ));
    }
    if let Some(audio) = state.audio {
        let label = if audio.silent() {
            "Muted".to_owned()
        } else {
            format!("{} %", audio.volume)
        };
        info = info.push(info_row(
            audio_view(audio, 1.0),
            "Audio",
            text(label).size(13).color(Color::WHITE).into(),
        ));
    }

    let armed = panel.armed();
    let hint = match armed {
        Some(Action::Poweroff) => "Click again to power off",
        Some(Action::Logout) => "Click again to log out",
        None => "",
    };
    let actions = row![
        text(hint).size(11).color(WARNING_COLOR).width(Length::Fill),
        action_button(&ICON_LOGOUT, Action::Logout, armed),
        action_button(&ICON_POWER, Action::Poweroff, armed),
    ]
    .spacing(2)
    .align_y(Alignment::Center);

    let separator = container(space())
        .width(Length::Fill)
        .height(1)
        .style(|_| container::Style {
            background: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.1).into()),
            ..Default::default()
        });

    container(column![meters, info, separator, actions].spacing(PANEL_GAP))
        .padding(PANEL_PADDING)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style {
            background: Some(CALENDAR_BACKGROUND.into()),
            border: Border::default().rounded(14.0),
            ..Default::default()
        })
        .into()
}

/// Dim icon, name and "12 %" over a thin bar; orange from `METER_WARNING`.
fn meter(icon: &svg::Handle, name: &'static str, percent: u8) -> Element<'static, Message> {
    let percent = percent.min(100);
    let color = if percent >= METER_WARNING {
        WARNING_COLOR
    } else {
        Color::WHITE
    };
    let label = row![
        tinted(icon, DIM),
        text(name).size(13).color(Color::WHITE).width(Length::Fill),
        text(format!("{percent} %")).size(13).color(color),
    ]
    .spacing(8)
    .height(METER_LABEL)
    .align_y(Alignment::Center);

    let filled = container(space())
        .width(Length::FillPortion(u16::from(percent)))
        .height(Length::Fill)
        .style(move |_| container::Style {
            background: Some(color.into()),
            ..Default::default()
        });
    let bar = container(row![
        filled,
        space().width(Length::FillPortion(u16::from(100 - percent)))
    ])
    .width(Length::Fill)
    .height(METER_BAR)
    .style(|_| container::Style {
        background: Some(Color::from_rgba(1.0, 1.0, 1.0, 0.14).into()),
        border: Border::default().rounded(METER_BAR / 2.0),
        ..Default::default()
    });

    column![label, bar].spacing(6).width(Length::Fill).into()
}

/// Icon, name and value aligned to the right.
fn info_row(
    icon: Element<'static, Message>,
    name: &'static str,
    value: Element<'static, Message>,
) -> Element<'static, Message> {
    row![
        container(icon).center_x(24),
        text(name).size(13).color(Color::WHITE).width(Length::Fill),
        value,
    ]
    .spacing(8)
    .height(INFO_ROW)
    .align_y(Alignment::Center)
    .into()
}

/// Battery state in words, with the charging color; nothing when unknown.
fn battery_label(battery: &Battery) -> Option<(&'static str, Color)> {
    match battery.status {
        BatteryStatus::Charging => Some(("charging", CHARGING_COLOR)),
        BatteryStatus::Discharging => Some(("on battery", DIM)),
        BatteryStatus::Full => Some(("full", DIM)),
        BatteryStatus::NotCharging => Some(("plugged in", DIM)),
        BatteryStatus::Unknown => None,
    }
}

/// 44×44 icon button; orange while armed. Leaving it with the cursor disarms.
fn action_button(
    icon: &svg::Handle,
    action: Action,
    armed: Option<Action>,
) -> Element<'static, Message> {
    let armed = armed == Some(action);
    let color = if armed { WARNING_COLOR } else { Color::WHITE };
    let glyph = svg(icon.clone())
        .width(ACTION_ICON)
        .height(ACTION_ICON)
        .style(move |_, _| svg::Style { color: Some(color) });
    let button = button(container(glyph).center(Length::Fill))
        .on_press(Message::Press(action))
        .width(ACTION_BUTTON)
        .height(ACTION_BUTTON)
        .padding(0)
        .style(move |theme, status| {
            if armed {
                button::Style {
                    background: Some(fade(WARNING_COLOR, 0.18).into()),
                    border: Border::default().rounded(10.0),
                    ..Default::default()
                }
            } else {
                button::Style {
                    border: Border::default().rounded(10.0),
                    ..flat_button(theme, status)
                }
            }
        });
    mouse_area(button).on_exit(Message::Disarm).into()
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
    fn panel_size_fits_its_sections() {
        let (width, height) = panel_size();
        assert_eq!(width, 300);
        assert_eq!(height, 28 + 28 + 80 + 1 + 44 + 42);
    }

    #[test]
    fn calendar_fits_six_weeks() {
        let (width, height) = calendar_size();
        assert_eq!(width, 220);
        assert_eq!(height, 28 + 28 * 7 + 24);
    }
}
