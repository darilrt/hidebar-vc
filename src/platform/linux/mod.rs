//! Linux shell: `iced_layershell` + Hyprland/Niri. The logic lives in `state` and `ui`.

mod audio;
mod battery;
mod hyprland;
mod ipc;
mod network;
mod niri;
mod notifications;
mod session;
mod system;

use std::io;
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::mpsc::Sender;
use std::time::Instant;

use iced::futures::channel::mpsc::UnboundedReceiver;
use iced::keyboard::{self, key::Named};
use iced::widget::space;
use iced::{Color, Element, Subscription, Task, event, window};
use iced_layershell::reexport::{
    Anchor, KeyboardInteractivity, Layer, NewLayerShellSettings, OutputOption,
};
use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};
use iced_layershell::{daemon, to_layer_message};

use crate::command::Command;
use crate::state::{ANIMATION_FRAME, CLOCK_TICK, Effect, FRAME, Hidebar, Input, Module, TICK};
use crate::ui::{self, Message as UiMessage};
use session::Compositor;

const NAMESPACE: &str = "hidebar";

struct Shell {
    state: Hidebar,
    compositor: Compositor,
    strip: Option<window::Id>,
    calendar_surface: Option<window::Id>,
    panel_surface: Option<window::Id>,
    notification_surface: Option<window::Id>,
    /// Size last given to the notification surface: it follows the stack of cards.
    notification_size: (u32, u32),
    /// Answers for the notification server (ADR-0007).
    effects: Sender<Effect>,
}

impl Shell {
    fn new(compositor: Compositor, effects: Sender<Effect>) -> Self {
        Self {
            state: Hidebar::new(),
            compositor,
            strip: None,
            calendar_surface: None,
            panel_surface: None,
            notification_surface: None,
            notification_size: (0, 0),
            effects,
        }
    }
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
enum Message {
    Input(Input),
    Ui(UiMessage),
    Tick,
    Frame,
    Opened(window::Id),
    Closed(window::Id),
    Focus(window::Id, bool),
    Key(window::Id, Named),
}

fn run(
    inputs: UnboundedReceiver<Input>,
    compositor: Compositor,
    effects: Sender<Effect>,
) -> Result<(), iced_layershell::Error> {
    let inputs = Mutex::new(Some(inputs));
    daemon(
        move || {
            let task = match inputs.lock().ok().and_then(|mut slot| slot.take()) {
                Some(receiver) => Task::run(receiver, Message::Input),
                None => Task::none(),
            };
            (Shell::new(compositor, effects.clone()), task)
        },
        || NAMESPACE.to_owned(),
        update,
        view,
    )
    .subscription(subscription)
    .style(|_, _| iced::theme::Style {
        background_color: Color::TRANSPARENT,
        text_color: Color::WHITE,
    })
    .settings(Settings {
        layer_settings: LayerShellSettings {
            // No initial surface: only drawn when requested.
            start_mode: StartMode::Background,
            layer: Layer::Overlay,
            exclusive_zone: -1,
            keyboard_interactivity: KeyboardInteractivity::None,
            events_transparent: true,
            ..Default::default()
        },
        ..Default::default()
    })
    .run()
}

fn update(shell: &mut Shell, message: Message) -> Task<Message> {
    let task = handle(shell, message);
    for effect in shell.state.take_effects() {
        // Without a server (name taken) nobody listens: the answer is dropped.
        let _ = shell.effects.send(effect);
    }
    Task::batch([task, sync(shell)])
}

fn handle(shell: &mut Shell, message: Message) -> Task<Message> {
    match message {
        Message::Input(Input::Command(Command::Quit)) => return iced::exit(),
        Message::Input(input) => shell.state.handle(input),
        Message::Ui(message) => match message {
            UiMessage::Month(delta) => shell.state.shift_calendar(delta),
            UiMessage::Today => shell.state.calendar_today(),
            UiMessage::Close => shell.state.close_calendar(),
            UiMessage::Press(action) => {
                if let Some(action) = shell.state.press(action, Instant::now()) {
                    session::run(action, shell.compositor);
                }
            }
            UiMessage::Disarm => shell.state.disarm(),
            UiMessage::NotificationClick(id) => shell.state.click_notification(id),
            UiMessage::NotificationDismiss(id) => shell.state.dismiss_notification(id),
            UiMessage::NotificationAction(id, key) => shell.state.notification_action(id, &key),
            UiMessage::NotificationHover(id, hovered) => {
                shell.state.hover_notification(id, hovered, Instant::now());
            }
            UiMessage::NotificationsExpand => shell.state.expand_notifications(),
        },
        Message::Tick => shell.state.tick(Instant::now()),
        Message::Frame => shell.state.frame(Instant::now()),
        // The strip surface takes a moment to appear: notices start animating once it is there.
        Message::Opened(id) if shell.strip == Some(id) => shell.state.strip_opened(Instant::now()),
        Message::Opened(id) if shell.calendar_surface == Some(id) => {
            shell.state.calendar_opened(Instant::now());
        }
        Message::Opened(id) if shell.panel_surface == Some(id) => {
            shell.state.panel_opened(Instant::now());
        }
        Message::Opened(id) if shell.notification_surface == Some(id) => {
            shell.state.notification_opened(Instant::now());
        }
        Message::Closed(id) => {
            // If the compositor closed a surface, `sync` reopens it if it is still needed.
            shell.strip.take_if(|strip| *strip == id);
            shell.notification_surface.take_if(|surface| *surface == id);
            if shell
                .calendar_surface
                .take_if(|surface| *surface == id)
                .is_some()
            {
                shell.state.close_calendar();
            }
            if shell
                .panel_surface
                .take_if(|surface| *surface == id)
                .is_some()
            {
                shell.state.close_panel();
            }
        }
        Message::Focus(id, focused) if shell.calendar_surface == Some(id) => {
            shell.state.mark_calendar_focused(focused);
        }
        Message::Focus(id, focused) if shell.panel_surface == Some(id) => {
            shell.state.mark_panel_focused(focused);
        }
        // Calendar: Esc closes, ←/→ change month, Home returns to the current month.
        Message::Key(id, key) if shell.calendar_surface == Some(id) => {
            let message = match key {
                Named::Escape => UiMessage::Close,
                Named::ArrowLeft => UiMessage::Month(-1),
                Named::ArrowRight => UiMessage::Month(1),
                Named::Home => UiMessage::Today,
                _ => return Task::none(),
            };
            return handle(shell, Message::Ui(message));
        }
        // Panel: Esc disarms an armed action, otherwise closes.
        Message::Key(id, Named::Escape) if shell.panel_surface == Some(id) => {
            if shell
                .state
                .panel
                .as_ref()
                .is_some_and(|panel| panel.armed().is_some())
            {
                shell.state.disarm();
            } else {
                shell.state.close_panel();
            }
        }
        // The layer-shell variants generated by the macro are consumed by the runtime.
        _ => {}
    }
    Task::none()
}

fn view(shell: &Shell, id: window::Id) -> Element<'_, Message> {
    if shell.strip == Some(id) {
        ui::view(&shell.state, ui::Surface::Strip).map(Message::Ui)
    } else if shell.calendar_surface == Some(id) {
        ui::view(&shell.state, ui::Surface::Calendar).map(Message::Ui)
    } else if shell.panel_surface == Some(id) {
        ui::view(&shell.state, ui::Surface::Panel).map(Message::Ui)
    } else if shell.notification_surface == Some(id) {
        ui::view(&shell.state, ui::Surface::Notification).map(Message::Ui)
    } else {
        space().into()
    }
}

/// Opens and closes surfaces according to the state.
fn sync(shell: &mut Shell) -> Task<Message> {
    Task::batch([
        surface(&mut shell.strip, shell.state.strip_wanted(), strip_settings),
        surface(
            &mut shell.calendar_surface,
            shell.state.calendar_wanted(),
            calendar_settings,
        ),
        surface(
            &mut shell.panel_surface,
            shell.state.panel_wanted(),
            panel_settings,
        ),
        notification_surface(shell),
    ])
}

/// Like `surface`, but its height follows the stack of cards: opened at the current size, resized when it changes.
fn notification_surface(shell: &mut Shell) -> Task<Message> {
    let size = ui::notifications_size(&shell.state);
    match (
        shell.state.notification_wanted(),
        shell.notification_surface,
    ) {
        (true, None) => {
            let id = window::Id::unique();
            shell.notification_surface = Some(id);
            shell.notification_size = size;
            Task::done(Message::NewLayerShell {
                settings: notification_settings(size),
                id,
            })
        }
        (true, Some(id)) if shell.notification_size != size => {
            shell.notification_size = size;
            Task::done(Message::SizeChange { id, size })
        }
        (false, Some(id)) => {
            shell.notification_surface = None;
            window::close(id)
        }
        _ => Task::none(),
    }
}

fn surface(
    slot: &mut Option<window::Id>,
    wanted: bool,
    settings: fn() -> NewLayerShellSettings,
) -> Task<Message> {
    match (wanted, *slot) {
        (true, None) => {
            let id = window::Id::unique();
            *slot = Some(id);
            Task::done(Message::NewLayerShell {
                settings: settings(),
                id,
            })
        }
        (false, Some(id)) => {
            *slot = None;
            window::close(id)
        }
        _ => Task::none(),
    }
}

fn subscription(shell: &Shell) -> Subscription<Message> {
    let mut subscriptions = vec![
        window::open_events().map(Message::Opened),
        window::close_events().map(Message::Closed),
    ];
    if shell.state.momentary() {
        subscriptions.push(iced::time::every(TICK).map(|_| Message::Tick));
    } else if shell.state.visible(Module::Clock) {
        subscriptions.push(iced::time::every(CLOCK_TICK).map(|_| Message::Tick));
    }
    // Animation at 60 fps; otherwise the low-battery pulse or a notification time bar at 20 fps.
    if shell.state.animating() {
        subscriptions.push(iced::time::every(ANIMATION_FRAME).map(|_| Message::Frame));
    } else if shell.state.pulsing() || shell.state.counting_down() {
        subscriptions.push(iced::time::every(FRAME).map(|_| Message::Frame));
    }
    if shell.calendar_surface.is_some() || shell.panel_surface.is_some() {
        subscriptions.push(event::listen_with(surface_input));
    }
    // At rest only the close listener remains: zero work.
    Subscription::batch(subscriptions)
}

/// Focus and named keys of the interactive surfaces; `handle` routes them by window.
fn surface_input(event: iced::Event, _: event::Status, id: window::Id) -> Option<Message> {
    match event {
        iced::Event::Window(window::Event::Focused) => Some(Message::Focus(id, true)),
        iced::Event::Window(window::Event::Unfocused) => Some(Message::Focus(id, false)),
        iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(key),
            ..
        }) => Some(Message::Key(id, key)),
        _ => None,
    }
}

fn strip_settings() -> NewLayerShellSettings {
    NewLayerShellSettings {
        size: Some((0, ui::STRIP_HEIGHT)),
        layer: Layer::Overlay,
        anchor: Anchor::Top | Anchor::Left | Anchor::Right,
        // -1: reserves no space and is not pushed by other bars' exclusive zones.
        exclusive_zone: Some(-1),
        margin: None,
        keyboard_interactivity: KeyboardInteractivity::None,
        output_option: OutputOption::Active,
        events_transparent: true,
        namespace: Some(NAMESPACE.to_owned()),
    }
}

/// Below the clock, centered (anchored to the top only). Receives pointer and keyboard.
fn calendar_settings() -> NewLayerShellSettings {
    let (width, height) = ui::calendar_size();
    NewLayerShellSettings {
        size: Some((width, height)),
        anchor: Anchor::Top,
        margin: Some((ui::STRIP_HEIGHT as i32 + ui::CALENDAR_GAP, 0, 0, 0)),
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
        events_transparent: false,
        ..strip_settings()
    }
}

/// Notification cards (ADR-0007): below the strip, aligned to the right like the panel. Receives the pointer
/// only while there are cards; never the keyboard.
fn notification_settings(size: (u32, u32)) -> NewLayerShellSettings {
    NewLayerShellSettings {
        size: Some(size),
        anchor: Anchor::Top | Anchor::Right,
        margin: Some((
            ui::STRIP_HEIGHT as i32 + ui::CALENDAR_GAP,
            ui::PANEL_SIDE,
            0,
            0,
        )),
        events_transparent: false,
        ..strip_settings()
    }
}

/// Below the `status` pill, aligned to its right edge. Receives pointer and keyboard.
fn panel_settings() -> NewLayerShellSettings {
    let (width, height) = ui::panel_size();
    NewLayerShellSettings {
        size: Some((width, height)),
        anchor: Anchor::Top | Anchor::Right,
        margin: Some((
            ui::STRIP_HEIGHT as i32 + ui::CALENDAR_GAP,
            ui::PANEL_SIDE,
            0,
            0,
        )),
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
        events_transparent: false,
        ..strip_settings()
    }
}

pub(crate) fn run_daemon() -> ExitCode {
    use_vulkan_by_default();

    let listener = match ipc::bind() {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("hidebar: {error}");
            return ExitCode::FAILURE;
        }
    };

    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let commands = sender.clone();
    ipc::serve(listener, move |command| {
        commands.unbounded_send(Input::Command(command)).is_ok()
    });

    // Each source gets its own sender into the same channel.
    let emitter = || {
        let events = sender.clone();
        move |event| events.unbounded_send(Input::Event(event)).is_ok()
    };

    // Niri must answer: a stale `NIRI_SOCKET` must not shadow Hyprland.
    let compositor = if niri::available() {
        niri::listen(emitter());
        Compositor::Niri
    } else if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        hyprland::listen(emitter());
        Compositor::Hyprland
    } else {
        eprintln!(
            "hidebar: neither NIRI_SOCKET nor HYPRLAND_INSTANCE_SIGNATURE is set; no workspace events"
        );
        Compositor::Unknown
    };

    battery::listen(emitter());
    system::listen(emitter());
    network::listen(emitter());
    audio::listen(emitter());
    let effects = notifications::listen(emitter());

    let result = run(receiver, compositor, effects);
    let _ = std::fs::remove_file(ipc::socket_path());

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("hidebar: {error}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn send(command: Command) -> io::Result<String> {
    ipc::send(command)
}

pub(crate) fn install() -> ExitCode {
    eprintln!("hidebar: install is only implemented on Windows");
    ExitCode::FAILURE
}

pub(crate) fn uninstall() -> ExitCode {
    eprintln!("hidebar: uninstall is only implemented on Windows");
    ExitCode::FAILURE
}

/// Vulkan by default (ADR-0003): without this, wgpu also enumerates GL and EGL loads every
/// installed driver. An explicit `WGPU_BACKEND` is respected.
fn use_vulkan_by_default() {
    if std::env::var_os("WGPU_BACKEND").is_none() {
        // SAFETY: called at the start of `run_daemon`, before any thread is spawned.
        unsafe { std::env::set_var("WGPU_BACKEND", "vulkan") };
    }
}
