//! Windows shell: Win32 + `UpdateLayeredWindow` + `iced_tiny_skia`.
//!
//! Uses neither wgpu, winit nor `iced::daemon`. The message loop owns the
//! state; the WndProc only validates painting and hit-testing.

mod audio;
mod battery;
mod fullscreen;
mod geometry;
mod hotkeys;
mod ipc;
mod logging;
mod network;
mod render;
mod session;
mod startup;
mod system;
mod win32;

pub(crate) use startup::{install, uninstall};

use std::io;
use std::mem::zeroed;
use std::process::ExitCode;
use std::ptr::null_mut;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_HOME, VK_LEFT, VK_RIGHT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DispatchMessageW, GetMessageW, HTTRANSPARENT, MSG, PostMessageW,
    PostQuitMessage, TranslateMessage, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ERASEBKGND,
    WM_HOTKEY, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCHITTEST,
    WM_PAINT, WM_SETFOCUS, WM_TIMER,
};

use crate::command::Command;
use crate::event::Event;
use crate::state::{CLOCK_TICK, FRAME, Hidebar, Input, Module, TICK};
use crate::ui;
use geometry::WorkArea;
use render::LayeredSurface;

/// Interactive window (calendar or status panel): receives cursor, keyboard and focus while open.
struct Interactive {
    layer: LayeredSurface,
    surface: ui::Surface,
    visible: bool,
    cursor: iced::mouse::Cursor,
}

impl Interactive {
    fn new(layer: LayeredSurface, surface: ui::Surface) -> Self {
        Self {
            layer,
            surface,
            visible: false,
            cursor: iced::mouse::Cursor::Unavailable,
        }
    }

    fn window(&self) -> HWND {
        self.layer.window()
    }

    fn wanted(&self, state: &Hidebar) -> bool {
        match self.surface {
            ui::Surface::Calendar => state.calendar_wanted(),
            ui::Surface::Panel => state.panel_wanted(),
            ui::Surface::Strip => false,
        }
    }

    fn render(&mut self, state: &Hidebar) {
        self.layer.render(state, self.surface, self.cursor);
    }

    fn sync(&mut self, state: &Hidebar) {
        if self.wanted(state) {
            self.render(state);
            if !self.visible {
                self.layer.show(true);
                self.visible = true;
            }
        } else {
            self.hide();
        }
    }

    fn hide(&mut self) {
        if self.visible {
            self.layer.hide();
            self.visible = false;
        }
        // A stale position would draw a hover the next time it opens.
        self.cursor = iced::mouse::Cursor::Unavailable;
    }

    /// Runs `event` through the view and applies the resulting messages to the state.
    fn dispatch(&mut self, state: &mut Hidebar, event: iced::Event) {
        let messages = self.layer.update(state, self.surface, event, self.cursor);
        for message in messages {
            apply_ui(state, message);
        }
    }
}

struct Shell {
    state: Hidebar,
    area: WorkArea,
    strip: LayeredSurface,
    strip_visible: bool,
    calendar: Interactive,
    panel: Interactive,
    timer: Option<u32>,
    inputs: Receiver<Input>,
    fullscreen_rx: Receiver<bool>,
    fullscreen: bool,
    hotkeys: Vec<i32>,
    quit: bool,
}

impl Shell {
    fn new(
        area: WorkArea,
        windows: Windows,
        inputs: Receiver<Input>,
        fullscreen_rx: Receiver<bool>,
    ) -> Self {
        let strip = LayeredSurface::new(
            windows.strip,
            POINT {
                x: area.left,
                y: area.top,
            },
            area.width,
            area.strip_height(),
            area.scale,
        );

        let (x, y, width, height) = area.calendar_rect();
        let calendar =
            LayeredSurface::new(windows.calendar, POINT { x, y }, width, height, area.scale);

        let (x, y, width, height) = area.panel_rect();
        let panel = LayeredSurface::new(windows.panel, POINT { x, y }, width, height, area.scale);

        Self {
            state: Hidebar::new(),
            area,
            strip,
            strip_visible: false,
            calendar: Interactive::new(calendar, ui::Surface::Calendar),
            panel: Interactive::new(panel, ui::Surface::Panel),
            timer: None,
            inputs,
            fullscreen_rx,
            fullscreen: false,
            hotkeys: hotkeys::register(windows.strip),
            quit: false,
        }
    }

    fn sync(&mut self) {
        if self.fullscreen {
            if self.strip_visible {
                self.strip.hide();
                self.strip_visible = false;
            }
            self.calendar.hide();
            self.panel.hide();
            if self.timer.is_some() {
                win32::set_timer(self.strip.window(), None);
                self.timer = None;
            }
            return;
        }

        let strip = self.state.strip_wanted();
        if strip {
            self.strip.render(
                &self.state,
                ui::Surface::Strip,
                iced::mouse::Cursor::Unavailable,
            );
            if !self.strip_visible {
                self.strip.show(false);
                self.strip_visible = true;
            }
        } else if self.strip_visible {
            self.strip.hide();
            self.strip_visible = false;
        }

        self.calendar.sync(&self.state);
        self.panel.sync(&self.state);

        self.update_timer();
    }

    fn update_timer(&mut self) {
        let interval = if self.state.pulsing() {
            Some(FRAME.as_millis() as u32)
        } else if self.state.momentary() {
            Some(TICK.as_millis() as u32)
        } else if self.state.visible(Module::Clock) {
            Some(CLOCK_TICK.as_millis() as u32)
        } else {
            None
        };

        if interval != self.timer {
            win32::set_timer(self.strip.window(), interval);
            self.timer = interval;
        }
    }

    fn on_timer(&mut self) {
        self.state.tick(Instant::now());
        self.sync();
    }

    fn on_events(&mut self) {
        while let Ok(active) = self.fullscreen_rx.try_recv() {
            self.fullscreen = active;
            if active {
                self.state.close_calendar();
                self.state.close_panel();
            }
        }

        while let Ok(input) = self.inputs.try_recv() {
            if matches!(&input, Input::Command(Command::Quit)) {
                self.quit = true;
                return;
            }
            self.state.handle(input);
        }
        self.sync();
    }

    fn on_hotkey(&mut self, id: i32) {
        let Some(command) = hotkeys::command(id) else {
            return;
        };
        self.state.handle(Input::Command(command));
        self.sync();
    }

    /// The interactive window `hwnd` belongs to, together with the state it acts on.
    fn target(&mut self, hwnd: HWND) -> Option<(&mut Hidebar, &mut Interactive)> {
        let target = if hwnd == self.calendar.window() {
            &mut self.calendar
        } else if hwnd == self.panel.window() {
            &mut self.panel
        } else {
            return None;
        };
        Some((&mut self.state, target))
    }

    /// After input on `hwnd`: repaint only that window while it stays open, otherwise resync everything.
    fn after_input(&mut self, hwnd: HWND) {
        let Some((state, target)) = self.target(hwnd) else {
            return;
        };
        if target.wanted(state) {
            target.render(state);
            // Arming a session action needs ticks for its expiry.
            self.update_timer();
        } else {
            self.sync();
        }
    }

    fn on_mouse(&mut self, hwnd: HWND, message: u32, lparam: LPARAM) {
        let scale = self.area.scale;
        let Some((state, target)) = self.target(hwnd) else {
            return;
        };
        if !target.visible {
            return;
        }

        win32::track_mouse_leave(hwnd);
        let position = iced::Point::new(
            low_word(lparam) as f32 / scale,
            high_word(lparam) as f32 / scale,
        );
        target.cursor = iced::mouse::Cursor::Available(position);

        let event = match message {
            WM_MOUSEMOVE => iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }),
            WM_LBUTTONDOWN => {
                iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))
            }
            WM_LBUTTONUP => iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                iced::mouse::Button::Left,
            )),
            _ => return,
        };

        target.dispatch(state, event);
        self.after_input(hwnd);
    }

    fn on_mouse_leave(&mut self, hwnd: HWND) {
        let Some((state, target)) = self.target(hwnd) else {
            return;
        };
        target.cursor = iced::mouse::Cursor::Unavailable;
        if !target.visible {
            return;
        }
        // Lets `mouse_area::on_exit` fire: leaving a session button disarms it.
        target.dispatch(state, iced::Event::Mouse(iced::mouse::Event::CursorLeft));
        self.after_input(hwnd);
    }

    fn on_key(&mut self, hwnd: HWND, key: u32) {
        let message = if hwnd == self.calendar.window() {
            match key {
                k if k == VK_ESCAPE as u32 => ui::Message::Close,
                k if k == VK_LEFT as u32 => ui::Message::Month(-1),
                k if k == VK_RIGHT as u32 => ui::Message::Month(1),
                k if k == VK_HOME as u32 => ui::Message::Today,
                _ => return,
            }
        } else if hwnd == self.panel.window() && key == VK_ESCAPE as u32 {
            // Esc disarms an armed action, otherwise closes.
            if self
                .state
                .panel
                .as_ref()
                .is_some_and(|panel| panel.armed().is_some())
            {
                ui::Message::Disarm
            } else {
                self.state.close_panel();
                return self.sync();
            }
        } else {
            return;
        };

        apply_ui(&mut self.state, message);
        self.after_input(hwnd);
    }

    fn on_focus(&mut self, hwnd: HWND, focused: bool) {
        if hwnd == self.calendar.window() {
            self.state.mark_calendar_focused(focused);
        } else if hwnd == self.panel.window() {
            self.state.mark_panel_focused(focused);
        }
        self.sync();
    }

    fn relayout(&mut self, dpi: Option<u32>) {
        self.area = geometry::primary();

        // `GetDpiForSystem` is the DPI at startup; after a scale change
        // the WM_DPICHANGED wParam or the window's DPI must be used.
        if let Some(dpi) = dpi.or_else(|| win32::dpi_for_window(self.strip.window())) {
            self.area.scale = dpi as f32 / 96.0;
        }

        self.strip.move_resize(
            POINT {
                x: self.area.left,
                y: self.area.top,
            },
            self.area.width,
            self.area.strip_height(),
            self.area.scale,
        );

        let (x, y, width, height) = self.area.calendar_rect();
        self.calendar
            .layer
            .move_resize(POINT { x, y }, width, height, self.area.scale);

        let (x, y, width, height) = self.area.panel_rect();
        self.panel
            .layer
            .move_resize(POINT { x, y }, width, height, self.area.scale);

        self.sync();
    }

    fn interactive(&self, hwnd: HWND) -> bool {
        hwnd == self.calendar.window() || hwnd == self.panel.window()
    }

    fn run(&mut self) {
        // SAFETY: standard message loop of the UI thread; `message` is a valid out.
        let mut message: MSG = unsafe { zeroed() };

        while unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } > 0 {
            match message.message {
                WM_TIMER if message.hwnd == self.strip.window() => self.on_timer(),
                win32::WM_APP_EVENT => self.on_events(),
                WM_HOTKEY => self.on_hotkey(message.wParam as i32),
                WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP if self.interactive(message.hwnd) => {
                    self.on_mouse(message.hwnd, message.message, message.lParam);
                }
                WM_KEYDOWN if self.interactive(message.hwnd) => {
                    self.on_key(message.hwnd, message.wParam as u32);
                }
                // WM_SETFOCUS/KILLFOCUS and WM_DPICHANGED/DISPLAYCHANGE are sent,
                // not posted: the WndProc forwards them as these messages.
                win32::WM_APP_FOCUS => self.on_focus(message.hwnd, message.wParam != 0),
                win32::WM_APP_RELAYOUT => {
                    self.relayout((message.wParam != 0).then_some(message.wParam as u32));
                }
                win32::WM_MOUSELEAVE if self.interactive(message.hwnd) => {
                    self.on_mouse_leave(message.hwnd);
                }
                // SAFETY: standard dispatch of messages the shell does not handle.
                _ => unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                },
            }

            if self.quit {
                break;
            }
        }
    }
}

/// Applies a view message to the state; confirmed session actions run here.
fn apply_ui(state: &mut Hidebar, message: ui::Message) {
    match message {
        ui::Message::Month(delta) => state.shift_calendar(delta),
        ui::Message::Today => state.calendar_today(),
        ui::Message::Close => state.close_calendar(),
        ui::Message::Press(action) => {
            if let Some(action) = state.press(action, Instant::now()) {
                session::run(action);
            }
        }
        ui::Message::Disarm => state.disarm(),
    }
}

/// The daemon's windows, created before the shell.
struct Windows {
    strip: HWND,
    calendar: HWND,
    panel: HWND,
}

pub(crate) fn run_daemon() -> ExitCode {
    win32::enable_dpi_awareness();
    logging::init();
    win32::hide_console_if_own();
    let area = geometry::primary();

    let strip_class = win32::wide("hidebar_strip");
    let calendar_class = win32::wide("hidebar_calendar");
    let panel_class = win32::wide("hidebar_panel");
    if !win32::register_class(&strip_class, Some(strip_proc))
        || !win32::register_class(&calendar_class, Some(interactive_proc))
        || !win32::register_class(&panel_class, Some(interactive_proc))
    {
        logging::error(format!(
            "hidebar: could not register the window class: {}",
            io::Error::last_os_error()
        ));
        return ExitCode::FAILURE;
    }

    let title = win32::wide("hidebar");
    let strip_hwnd = win32::create_window(
        &strip_class,
        &title,
        win32::STRIP_EX_STYLE,
        area.left,
        area.top,
        area.width,
        area.strip_height(),
    );

    let (x, y, width, height) = area.calendar_rect();
    let calendar_hwnd = win32::create_window(
        &calendar_class,
        &title,
        win32::INTERACTIVE_EX_STYLE,
        x,
        y,
        width,
        height,
    );

    let (x, y, width, height) = area.panel_rect();
    let panel_hwnd = win32::create_window(
        &panel_class,
        &title,
        win32::INTERACTIVE_EX_STYLE,
        x,
        y,
        width,
        height,
    );

    let (sender, receiver) = mpsc::channel();
    if let Err(error) = ipc::listen(sender.clone(), win32::SendHwnd(strip_hwnd)) {
        logging::error(format!("hidebar: {error}"));
        return ExitCode::FAILURE;
    }

    let hwnd = win32::SendHwnd(strip_hwnd);
    battery::listen(emitter(sender.clone(), hwnd));
    system::listen(emitter(sender.clone(), hwnd));
    network::listen(emitter(sender.clone(), hwnd));
    audio::listen(emitter(sender.clone(), hwnd));

    let (fullscreen_tx, fullscreen_rx) = mpsc::channel();
    fullscreen::listen(fullscreen_tx, hwnd);

    let windows = Windows {
        strip: strip_hwnd,
        calendar: calendar_hwnd,
        panel: panel_hwnd,
    };
    let mut shell = Shell::new(area, windows, receiver, fullscreen_rx);
    shell.sync();
    shell.run();

    hotkeys::unregister(strip_hwnd, &shell.hotkeys);

    ExitCode::SUCCESS
}

pub(crate) fn send(command: Command) -> io::Result<String> {
    ipc::send(command)
}

/// Shared source emitter: queues the event and wakes the message loop.
fn emitter(
    sender: mpsc::Sender<Input>,
    hwnd: win32::SendHwnd,
) -> impl Fn(Event) -> bool + Send + 'static {
    move |event| {
        if sender.send(Input::Event(event)).is_ok() {
            win32::post_event(hwnd);
            true
        } else {
            false
        }
    }
}

fn low_word(value: LPARAM) -> i32 {
    (value as u16) as i16 as i32
}

fn high_word(value: LPARAM) -> i32 {
    ((value >> 16) as u16) as i16 as i32
}

unsafe extern "system" fn strip_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { window_proc(true, hwnd, message, wparam, lparam) }
}

unsafe extern "system" fn interactive_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { window_proc(false, hwnd, message, wparam, lparam) }
}

/// WndProc of all windows. It does not touch the `Shell`: the main loop handles
/// timers, input, IPC and hotkeys outside of system reentrancy.
unsafe fn window_proc(
    is_strip: bool,
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST if is_strip => HTTRANSPARENT as LRESULT,
        WM_SETFOCUS | WM_KILLFOCUS if !is_strip => {
            let focused = usize::from(message == WM_SETFOCUS);
            // SAFETY: focus is forwarded to the UI thread's queue; the Shell
            // consumes it in its loop. DefWindowProc keeps the default behavior.
            unsafe {
                PostMessageW(hwnd, win32::WM_APP_FOCUS, focused, 0);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
        }
        WM_DPICHANGED => {
            // SAFETY: the wParam carries the new DPI in the low word; it is
            // forwarded so `relayout` does not depend on GetDpiForSystem.
            let dpi = wparam & 0xFFFF;
            unsafe {
                PostMessageW(hwnd, win32::WM_APP_RELAYOUT, dpi, 0);
            }
            0
        }
        WM_DISPLAYCHANGE => {
            // SAFETY: the notification is forwarded; `relayout` queries the window's
            // DPI if the message carries none.
            unsafe {
                PostMessageW(hwnd, win32::WM_APP_RELAYOUT, 0, 0);
            }
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            // SAFETY: paired BeginPaint/EndPaint on a valid HWND.
            let mut paint: PAINTSTRUCT = unsafe { zeroed() };
            unsafe {
                BeginPaint(hwnd, &mut paint);
                EndPaint(hwnd, &paint);
            }
            0
        }
        WM_DESTROY => {
            // SAFETY: ends this thread's message loop.
            unsafe { PostQuitMessage(0) };
            0
        }
        // SAFETY: the system guarantees a valid HWND and parameters for the message.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}
