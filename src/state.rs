//! Daemon state: visibility, notices, calendar, status panel and pulse.
//!
//! Knows nothing about iced or windows: it receives `Input` from the shell and exposes queries
//! (`strip_wanted`, `calendar_wanted`, `panel_wanted`, `active_notices`, ...) so that each
//! platform decides how to present them.

use std::time::{Duration, Instant};

use chrono::NaiveDate;

use crate::calendar;
use crate::command::{Command, Target};
use crate::event::{Audio, Battery, Event, Network, System, Workspaces};

/// How long each module stays visible in momentary mode; asking again restarts it.
const WORKSPACES_DURATION: Duration = Duration::from_millis(1500);
const CLOCK_DURATION: Duration = Duration::from_secs(3);
const STATUS_DURATION: Duration = Duration::from_secs(3);
/// How long a notice stays visible; repeating the same notice restarts it.
const NOTICE_DURATION: Duration = Duration::from_secs(3);
/// Expiry check interval while something is momentary.
pub(crate) const TICK: Duration = Duration::from_millis(250);
/// Time refresh with the clock pinned and nothing momentary.
pub(crate) const CLOCK_TICK: Duration = Duration::from_secs(1);
/// Frame interval while something pulses (20 fps: enough for a slow fade).
pub(crate) const FRAME: Duration = Duration::from_millis(50);

/// Battery thresholds (discharging), in percent.
const BATTERY_LOW: u8 = 25;
const BATTERY_CRITICAL: u8 = 10;
/// Low-battery pulse: opacity drops to `1 - PULSE_DIM` and back, every `PULSE_PERIOD`.
const PULSE_PERIOD: Duration = Duration::from_secs(2);
const PULSE_DIM: f32 = 0.7;
/// How long an armed session action waits for the confirming click (ADR-0005 Q2).
const ARM_DURATION: Duration = Duration::from_secs(3);

/// Notice animation (ADR-0006). Entering: the pill stretches leftwards while its background fades in;
/// only once it is fully stretched does the icon appear. Leaving: the icon fades out first, then the pill
/// shrinks back to the right.
const NOTICE_STRETCH: Duration = Duration::from_millis(260);
const NOTICE_FADE_IN: Duration = Duration::from_millis(80);
const NOTICE_REVEAL_DELAY: Duration = NOTICE_STRETCH;
const NOTICE_REVEAL: Duration = Duration::from_millis(220);
/// Notices entering together start one after another, right to left.
const NOTICE_STAGGER: Duration = Duration::from_millis(40);
const NOTICE_HIDE: Duration = Duration::from_millis(120);
const NOTICE_SHRINK_DELAY: Duration = NOTICE_HIDE;
const NOTICE_SHRINK: Duration = Duration::from_millis(200);
/// Frame interval while a notice animates (60 fps).
pub(crate) const ANIMATION_FRAME: Duration = Duration::from_millis(16);

/// What reaches the daemon from outside the shell: commands and source events.
#[derive(Debug, Clone)]
pub(crate) enum Input {
    Command(Command),
    Event(Event),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Module {
    Workspaces,
    Clock,
    Status,
}

pub(crate) const MODULES: [Module; 3] = [Module::Workspaces, Module::Clock, Module::Status];

impl Module {
    fn duration(self) -> Duration {
        match self {
            Self::Workspaces => WORKSPACES_DURATION,
            Self::Clock => CLOCK_DURATION,
            Self::Status => STATUS_DURATION,
        }
    }
}

/// Notice: a single `status` item shown on its own because it changed (ADR-0004). In `status` order.
/// To add one: a variant here and in `NOTICES`, its rule in `on_event` and its drawing in `view`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Notice {
    /// Connectivity changed.
    Network,
    /// Went from audible to silent or back.
    Audio,
    /// The charger was plugged or unplugged; also, without expiry, low battery.
    Battery,
}

const NOTICES: [Notice; 3] = [Notice::Network, Notice::Audio, Notice::Battery];

/// Animation of one notice or module pill: entering (and then settled) or leaving, since `since`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Motion {
    since: Instant,
    leaving: bool,
}

/// How a pill or notice is drawn at a given instant, every value from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Frame {
    /// Share of its width.
    pub(crate) size: f32,
    /// Content opacity (and slide, for notices).
    pub(crate) reveal: f32,
    /// Pill background opacity.
    pub(crate) background: f32,
}

/// A notice and how it is drawn now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NoticeFrame {
    pub(crate) notice: Notice,
    pub(crate) size: f32,
    pub(crate) reveal: f32,
    pub(crate) background: f32,
}

impl Motion {
    fn entering(since: Instant) -> Self {
        Self {
            since,
            leaving: false,
        }
    }

    fn frame(&self, instant: Instant) -> Frame {
        let elapsed = instant.saturating_duration_since(self.since);
        if self.leaving {
            let shrink = ease_in(progress(elapsed, NOTICE_SHRINK_DELAY, NOTICE_SHRINK));
            Frame {
                size: 1.0 - shrink,
                reveal: 1.0 - progress(elapsed, Duration::ZERO, NOTICE_HIDE),
                background: 1.0 - shrink,
            }
        } else {
            Frame {
                size: ease_out(progress(elapsed, Duration::ZERO, NOTICE_STRETCH)),
                reveal: ease_out(progress(elapsed, NOTICE_REVEAL_DELAY, NOTICE_REVEAL)),
                background: progress(elapsed, Duration::ZERO, NOTICE_FADE_IN),
            }
        }
    }

    /// Follows `wanted`: starts entering at `start`, reverses from the current width when it comes back while
    /// leaving, starts leaving, or ends (`None`) once it is gone. Returns whether it started entering.
    fn follow(slot: &mut Option<Self>, wanted: bool, start: Instant, instant: Instant) -> bool {
        match (*slot, wanted) {
            (None, true) => {
                *slot = Some(Self::entering(start));
                return true;
            }
            (Some(motion), true) if motion.leaving => {
                let size = motion.frame(instant).size;
                let elapsed = NOTICE_STRETCH.mul_f32(1.0 - (1.0 - size).cbrt());
                *slot = Some(Self::entering(
                    instant.checked_sub(elapsed).unwrap_or(instant),
                ));
            }
            (Some(motion), false) if !motion.leaving => {
                *slot = Some(Self {
                    since: instant,
                    leaving: true,
                });
            }
            (Some(motion), false) if motion.done(instant) => *slot = None,
            _ => {}
        }
        false
    }

    /// Nothing left to animate: fully shown, or gone.
    fn done(&self, instant: Instant) -> bool {
        let end = if self.leaving {
            NOTICE_SHRINK_DELAY + NOTICE_SHRINK
        } else {
            NOTICE_REVEAL_DELAY + NOTICE_REVEAL
        };
        instant >= self.since && instant.duration_since(self.since) >= end
    }
}

/// An entrance that started before its surface existed starts over from `instant`.
#[cfg(target_os = "linux")]
fn restart(slot: &mut Option<Motion>, instant: Instant) {
    if let Some(motion) = slot
        && !motion.leaving
        && motion.since < instant
    {
        motion.since = instant;
    }
}

/// Share of `duration` elapsed after `delay`, from 0 to 1.
fn progress(elapsed: Duration, delay: Duration, duration: Duration) -> f32 {
    (elapsed.saturating_sub(delay).as_secs_f32() / duration.as_secs_f32()).min(1.0)
}

fn ease_out(progress: f32) -> f32 {
    1.0 - (1.0 - progress).powi(3)
}

fn ease_in(progress: f32) -> f32 {
    progress.powi(3)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visibility {
    Hidden,
    Until(Instant),
    Pinned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Severity {
    Normal,
    Low,
    Critical,
}

/// Open calendar.
pub(crate) struct Calendar {
    /// First day of the displayed month.
    pub(crate) month: NaiveDate,
    /// The clock was already pinned when opening: closing the calendar keeps it pinned.
    pub(crate) clock_was_pinned: bool,
    /// Has received focus at least once; only then does losing it close the calendar.
    pub(crate) focused: bool,
}

/// Session actions of the status panel. Running them is up to the platform; the state only arms them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Logout,
    Poweroff,
}

/// Open status panel (ADR-0005).
pub(crate) struct Panel {
    /// `status` was already pinned when opening: closing the panel keeps it pinned.
    status_was_pinned: bool,
    /// Has received focus at least once; only then does losing it close the panel.
    focused: bool,
    /// Action waiting for its confirming click, and until when.
    armed: Option<(Action, Instant)>,
}

impl Panel {
    pub(crate) fn armed(&self) -> Option<Action> {
        self.armed.map(|(action, _)| action)
    }
}

pub(crate) struct Hidebar {
    visibility: [Visibility; 3],
    /// Expiry of each momentary notice, indexed by `Notice`.
    notices: [Option<Instant>; 3],
    /// Animation of each notice on screen (entering, shown or leaving), indexed by `Notice`.
    motions: [Option<Motion>; 3],
    /// Animation of each module pill, indexed by `Module`.
    module_motions: [Option<Motion>; 3],
    /// Animation of the calendar panel: it stretches down from below the clock, then its content shows.
    calendar_motion: Option<Motion>,
    /// Month still drawn while the calendar folds back after closing.
    closing_month: Option<NaiveDate>,
    /// Animation of the status panel, like the calendar's.
    panel_motion: Option<Motion>,
    /// Last known data; `None` until each source's first event.
    pub(crate) workspaces: Option<Workspaces>,
    pub(crate) battery: Option<Battery>,
    pub(crate) system: Option<System>,
    pub(crate) network: Option<Network>,
    pub(crate) audio: Option<Audio>,
    pub(crate) calendar: Option<Calendar>,
    pub(crate) panel: Option<Panel>,
    /// Low-battery pulse origin and last frame.
    pulse_origin: Instant,
    frame: Instant,
    pub(crate) now: Now,
}

/// Time and date already formatted for display, and today's date.
pub(crate) struct Now {
    pub(crate) time: String,
    pub(crate) date: String,
    pub(crate) today: NaiveDate,
}

impl Hidebar {
    pub(crate) fn new() -> Self {
        Self {
            visibility: [Visibility::Hidden; 3],
            notices: [None; 3],
            motions: [None; 3],
            module_motions: [None; 3],
            calendar_motion: None,
            closing_month: None,
            panel_motion: None,
            workspaces: None,
            battery: None,
            system: None,
            network: None,
            audio: None,
            calendar: None,
            panel: None,
            pulse_origin: Instant::now(),
            frame: Instant::now(),
            now: now(),
        }
    }

    fn visibility(&self, module: Module) -> Visibility {
        self.visibility[module as usize]
    }

    fn set_visibility(&mut self, module: Module, visibility: Visibility) {
        self.visibility[module as usize] = visibility;
    }

    pub(crate) fn visible(&self, module: Module) -> bool {
        self.visibility(module) != Visibility::Hidden
    }

    /// Without a reading from the source there is nothing to show.
    fn available(&self, module: Module) -> bool {
        match module {
            Module::Workspaces => self.workspaces.is_some(),
            Module::Clock => true,
            Module::Status => {
                self.battery.is_some()
                    || self.system.is_some()
                    || self.network.is_some()
                    || self.audio.is_some()
            }
        }
    }

    /// Momentary, unless pinned.
    fn show(&mut self, module: Module) {
        if self.available(module) && self.visibility(module) != Visibility::Pinned {
            if module == Module::Clock {
                self.now = now();
            }
            self.set_visibility(
                module,
                Visibility::Until(Instant::now() + module.duration()),
            );
        }
    }

    fn toggle(&mut self, module: Module) {
        if module == Module::Clock {
            return self.toggle_calendar();
        }
        if module == Module::Status {
            return self.toggle_panel();
        }
        let next = match self.visibility(module) {
            Visibility::Pinned => Visibility::Hidden,
            _ if self.available(module) => Visibility::Pinned,
            current => current,
        };
        self.set_visibility(module, next);
    }

    /// ADR-0002 Q2: if anything is pinned, unpin everything; otherwise pin all three (the clock, without calendar).
    fn toggle_all(&mut self) {
        let any_pinned = MODULES
            .iter()
            .any(|module| self.visibility(*module) == Visibility::Pinned);
        if any_pinned {
            if let Some(calendar) = self.calendar.take() {
                self.closing_month = Some(calendar.month);
            }
            self.panel = None;
            for module in MODULES {
                if self.visibility(module) == Visibility::Pinned {
                    self.set_visibility(module, Visibility::Hidden);
                }
            }
        } else {
            self.now = now();
            for module in MODULES {
                if self.available(module) {
                    self.set_visibility(module, Visibility::Pinned);
                }
            }
        }
    }

    fn toggle_calendar(&mut self) {
        if self.calendar.is_some() {
            return self.close_calendar();
        }
        self.now = now();
        self.calendar = Some(Calendar {
            month: calendar::first_of_month(self.now.today),
            clock_was_pinned: self.visibility(Module::Clock) == Visibility::Pinned,
            focused: false,
        });
        self.set_visibility(Module::Clock, Visibility::Pinned);
    }

    pub(crate) fn close_calendar(&mut self) {
        if let Some(calendar) = self.calendar.take() {
            self.closing_month = Some(calendar.month);
            if !calendar.clock_was_pinned {
                self.set_visibility(Module::Clock, Visibility::Hidden);
            }
        }
        // Closing also comes from the view and from focus, outside `handle`: start folding back now.
        self.settle();
    }

    pub(crate) fn shift_calendar(&mut self, delta: i32) {
        if let Some(calendar) = &mut self.calendar {
            calendar.month = calendar::shift_month(calendar.month, delta);
        }
    }

    pub(crate) fn calendar_today(&mut self) {
        let today = calendar::first_of_month(self.now.today);
        if let Some(calendar) = &mut self.calendar {
            calendar.month = today;
        }
    }

    pub(crate) fn mark_calendar_focused(&mut self, focused: bool) {
        if focused {
            if let Some(calendar) = &mut self.calendar {
                calendar.focused = true;
            }
        } else if self
            .calendar
            .as_ref()
            .is_some_and(|calendar| calendar.focused)
        {
            self.close_calendar();
        }
    }

    fn toggle_panel(&mut self) {
        if self.panel.is_some() {
            return self.close_panel();
        }
        self.panel = Some(Panel {
            status_was_pinned: self.visibility(Module::Status) == Visibility::Pinned,
            focused: false,
            armed: None,
        });
        if self.available(Module::Status) {
            self.set_visibility(Module::Status, Visibility::Pinned);
        }
    }

    pub(crate) fn close_panel(&mut self) {
        if let Some(panel) = self.panel.take()
            && !panel.status_was_pinned
        {
            self.set_visibility(Module::Status, Visibility::Hidden);
        }
        self.settle();
    }

    pub(crate) fn mark_panel_focused(&mut self, focused: bool) {
        if focused {
            if let Some(panel) = &mut self.panel {
                panel.focused = true;
            }
        } else if self.panel.as_ref().is_some_and(|panel| panel.focused) {
            self.close_panel();
        }
    }

    /// First click arms `action`; a second click on the same action while armed confirms it: the panel
    /// closes and the action is returned for the platform to run.
    pub(crate) fn press(&mut self, action: Action, instant: Instant) -> Option<Action> {
        let panel = self.panel.as_mut()?;
        if matches!(panel.armed, Some((armed, until)) if armed == action && instant < until) {
            self.close_panel();
            return Some(action);
        }
        panel.armed = Some((action, instant + ARM_DURATION));
        None
    }

    pub(crate) fn disarm(&mut self) {
        if let Some(panel) = &mut self.panel {
            panel.armed = None;
        }
    }

    fn expire(&mut self, instant: Instant) {
        if let Some(panel) = &mut self.panel {
            panel.armed.take_if(|(_, until)| *until <= instant);
        }
        for visibility in &mut self.visibility {
            if matches!(visibility, Visibility::Until(until) if *until <= instant) {
                *visibility = Visibility::Hidden;
            }
        }
        for notice in &mut self.notices {
            notice.take_if(|until| *until <= instant);
        }
    }

    /// Shows a notice briefly (or restarts its timer if already shown). With `status` visible the change is
    /// already visible there: it is not recorded, so it does not show up late once `status` hides.
    fn notify(&mut self, notice: Notice) {
        if !self.visible(Module::Status) {
            self.notices[notice as usize] = Some(Instant::now() + NOTICE_DURATION);
        }
    }

    /// Starts, reverses or ends the notice animations so that they follow `active_notices`.
    fn animate(&mut self, instant: Instant) {
        Motion::follow(
            &mut self.calendar_motion,
            self.calendar.is_some(),
            instant,
            instant,
        );
        if self.calendar_motion.is_none() {
            self.closing_month = None;
        }
        Motion::follow(
            &mut self.panel_motion,
            self.panel.is_some(),
            instant,
            instant,
        );
        for module in MODULES {
            let wanted = self.visible(module);
            Motion::follow(
                &mut self.module_motions[module as usize],
                wanted,
                instant,
                instant,
            );
        }
        // The full `status` pill replaces the notices at once.
        if self.visible(Module::Status) {
            self.motions = [None; 3];
            return;
        }
        let wanted = self.active_notices();
        let mut entering = 0;
        // Right to left, so the stagger starts from the edge the pill grows from.
        for notice in NOTICES.into_iter().rev() {
            let start = instant + NOTICE_STAGGER * entering;
            let slot = &mut self.motions[notice as usize];
            if Motion::follow(slot, wanted.contains(&notice), start, instant) {
                entering += 1;
            }
        }
    }

    /// Month shown by the calendar and how the panel is drawn now; `None` when it is not on screen.
    pub(crate) fn calendar_frame(&self) -> Option<(NaiveDate, Frame)> {
        let month = self
            .calendar
            .as_ref()
            .map(|calendar| calendar.month)
            .or(self.closing_month)?;
        Some((month, self.calendar_motion?.frame(self.frame)))
    }

    /// How the status panel is drawn now; `None` when it is not on screen.
    pub(crate) fn panel_frame(&self) -> Option<Frame> {
        self.panel_motion.map(|motion| motion.frame(self.frame))
    }

    /// The calendar surface has just appeared (Linux): its entrance starts over from `instant`.
    #[cfg(target_os = "linux")]
    pub(crate) fn calendar_opened(&mut self, instant: Instant) {
        restart(&mut self.calendar_motion, instant);
        self.frame = instant;
    }

    /// The status panel surface has just appeared (Linux): its entrance starts over from `instant`.
    #[cfg(target_os = "linux")]
    pub(crate) fn panel_opened(&mut self, instant: Instant) {
        restart(&mut self.panel_motion, instant);
        self.frame = instant;
    }

    /// How a module pill is drawn now; `None` when it is not on screen.
    pub(crate) fn module_frame(&self, module: Module) -> Option<Frame> {
        self.module_motions[module as usize].map(|motion| motion.frame(self.frame))
    }

    /// The strip surface has just appeared (Linux creates it on demand, which takes a moment): notices
    /// entering before then start over from `instant`, keeping their stagger, so the stretch is seen whole.
    #[cfg(target_os = "linux")]
    pub(crate) fn strip_opened(&mut self, instant: Instant) {
        let Some(first) = self
            .motions
            .iter()
            .chain(&self.module_motions)
            .flatten()
            .filter(|motion| !motion.leaving)
            .map(|motion| motion.since)
            .min()
        else {
            return;
        };
        let Some(delay) = instant.checked_duration_since(first) else {
            return;
        };
        for motion in self
            .motions
            .iter_mut()
            .chain(&mut self.module_motions)
            .flatten()
        {
            if !motion.leaving {
                motion.since += delay;
            }
        }
        self.frame = instant;
    }

    /// Notices on screen, in order, as they look at the current frame.
    pub(crate) fn notice_frames(&self) -> Vec<NoticeFrame> {
        NOTICES
            .into_iter()
            .filter_map(|notice| {
                self.motions[notice as usize].map(|motion| {
                    let Frame {
                        size,
                        reveal,
                        background,
                    } = motion.frame(self.frame);
                    NoticeFrame {
                        notice,
                        size,
                        reveal,
                        background,
                    }
                })
            })
            .collect()
    }

    /// A notice or module pill is entering or leaving: the shell must send `ANIMATION_FRAME` frames.
    pub(crate) fn animating(&self) -> bool {
        self.motions
            .iter()
            .chain(&self.module_motions)
            .chain([&self.calendar_motion, &self.panel_motion])
            .flatten()
            .any(|motion| motion.leaving || !motion.done(self.frame))
    }

    /// Notices that should be on screen, in order. None while `status` is visible: the full pill already
    /// contains them. Entering and leaving is up to `animate`.
    pub(crate) fn active_notices(&self) -> Vec<Notice> {
        if self.visible(Module::Status) {
            return Vec::new();
        }
        NOTICES
            .into_iter()
            .filter(|notice| {
                self.notices[*notice as usize].is_some()
                    || (*notice == Notice::Battery && self.severity() != Severity::Normal)
            })
            .filter(|notice| self.has_reading(*notice))
            .collect()
    }

    fn has_reading(&self, notice: Notice) -> bool {
        match notice {
            Notice::Network => self.network.is_some(),
            Notice::Audio => self.audio.is_some(),
            Notice::Battery => self.battery.is_some(),
        }
    }

    pub(crate) fn severity(&self) -> Severity {
        self.battery.as_ref().map_or(Severity::Normal, severity)
    }

    /// Current pulse opacity.
    pub(crate) fn pulse(&self) -> f32 {
        pulse_alpha(self.frame.saturating_duration_since(self.pulse_origin))
    }

    /// Something is pulsing on screen: with low battery, its icon is in `status` or in the notices.
    pub(crate) fn pulsing(&self) -> bool {
        self.strip_wanted() && self.severity() != Severity::Normal
    }

    /// The strip must be open: some module visible or some notice on screen (even while leaving).
    pub(crate) fn strip_wanted(&self) -> bool {
        MODULES.iter().any(|module| self.visible(*module))
            || self
                .motions
                .iter()
                .chain(&self.module_motions)
                .any(Option::is_some)
    }

    /// Open, or folding back after closing.
    pub(crate) fn calendar_wanted(&self) -> bool {
        self.calendar.is_some() || self.calendar_motion.is_some()
    }

    /// Open, or folding back after closing.
    pub(crate) fn panel_wanted(&self) -> bool {
        self.panel.is_some() || self.panel_motion.is_some()
    }

    /// Something has an expiry (momentary module or notice); the shell must keep checking the time.
    pub(crate) fn momentary(&self) -> bool {
        self.visibility
            .iter()
            .any(|visibility| matches!(visibility, Visibility::Until(_)))
            || self.notices.iter().any(Option::is_some)
            || self
                .panel
                .as_ref()
                .is_some_and(|panel| panel.armed.is_some())
    }

    pub(crate) fn handle(&mut self, input: Input) {
        match input {
            // Quitting is up to the shell, not the state.
            Input::Command(Command::Quit) => {}
            Input::Command(Command::Show(target)) => {
                for module in modules(target) {
                    self.show(*module);
                }
            }
            Input::Command(Command::Toggle(Target::All)) => self.toggle_all(),
            Input::Command(Command::Toggle(target)) => {
                for module in modules(target) {
                    self.toggle(*module);
                }
            }
            Input::Event(event) => self.on_event(event),
        }
        self.settle();
    }

    /// Brings the animations up to date after a change outside the frame clock.
    fn settle(&mut self) {
        let instant = Instant::now();
        self.frame = instant;
        self.animate(instant);
    }

    /// Advances the time and expires momentary items.
    pub(crate) fn tick(&mut self, instant: Instant) {
        self.now = now();
        self.frame = instant;
        self.expire(instant);
        self.animate(instant);
    }

    /// New pulse or animation frame without reformatting the time (Linux).
    #[cfg(target_os = "linux")]
    pub(crate) fn frame(&mut self, instant: Instant) {
        self.frame = instant;
        self.animate(instant);
    }

    /// Presentation decides which events deserve to appear.
    fn on_event(&mut self, event: Event) {
        match event {
            Event::Workspaces(workspaces) => {
                let switched = self
                    .workspaces
                    .as_ref()
                    .is_some_and(|old| old.active != workspaces.active);
                self.workspaces = Some(workspaces);
                if switched {
                    self.show(Module::Workspaces);
                }
            }
            Event::Battery(battery) => {
                // Plugging or unplugging the charger is notified; low level is notified while it lasts.
                let plug_changed = self
                    .battery
                    .as_ref()
                    .is_some_and(|old| old.plugged() != battery.plugged());
                self.battery = Some(battery);
                if plug_changed {
                    self.notify(Notice::Battery);
                }
            }
            Event::System(system) => self.system = Some(system),
            Event::Network(network) => {
                let changed = self.network.is_some_and(|old| old != network);
                self.network = Some(network);
                if changed {
                    self.notify(Notice::Network);
                }
            }
            Event::Audio(audio) => {
                // Only audible vs. silent matters; volume changes do not appear.
                let changed = self.audio.is_some_and(|old| old.silent() != audio.silent());
                self.audio = Some(audio);
                if changed {
                    self.notify(Notice::Audio);
                }
            }
        }
    }
}

/// Modules a command is aimed at.
fn modules(target: Target) -> &'static [Module] {
    match target {
        Target::Workspaces => &[Module::Workspaces],
        Target::Clock => &[Module::Clock],
        Target::Status => &[Module::Status],
        Target::All => &MODULES,
    }
}

/// Pulse opacity: 1 at the start of each period, `1 - PULSE_DIM` halfway, and back to 1.
fn pulse_alpha(elapsed: Duration) -> f32 {
    let phase = elapsed.as_secs_f32() % PULSE_PERIOD.as_secs_f32() / PULSE_PERIOD.as_secs_f32();
    1.0 - PULSE_DIM * (1.0 - (std::f32::consts::TAU * phase).cos()) / 2.0
}

pub(crate) fn severity(battery: &Battery) -> Severity {
    match battery.percent {
        _ if battery.plugged() => Severity::Normal,
        percent if percent <= BATTERY_CRITICAL => Severity::Critical,
        percent if percent <= BATTERY_LOW => Severity::Low,
        _ => Severity::Normal,
    }
}

fn now() -> Now {
    let now = chrono::Local::now();
    Now {
        time: now.format("%H:%M").to_string(),
        date: calendar::long_date(now.date_naive()),
        today: now.date_naive(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::BatteryStatus;

    fn input(event: Event) -> Input {
        Input::Event(event)
    }

    fn workspaces(active: i32) -> Input {
        input(Event::Workspaces(Workspaces {
            ids: vec![1, 2, 3],
            active,
        }))
    }

    fn battery(percent: u8, status: BatteryStatus) -> Battery {
        Battery { percent, status }
    }

    fn battery_event(percent: u8, status: BatteryStatus) -> Input {
        input(Event::Battery(battery(percent, status)))
    }

    fn show(target: Target) -> Input {
        Input::Command(Command::Show(target))
    }

    fn toggle(target: Target) -> Input {
        Input::Command(Command::Toggle(target))
    }

    fn apply(state: &mut Hidebar, input: Input) {
        state.handle(input);
    }

    /// State with readings from every source.
    fn ready() -> Hidebar {
        let mut state = Hidebar::new();
        apply(&mut state, workspaces(1));
        apply(&mut state, battery_event(80, BatteryStatus::Discharging));
        apply(&mut state, input(Event::System(System { cpu: 5, ram: 40 })));
        apply(&mut state, input(Event::Network(Network::Online)));
        state
    }

    /// Expires everything momentary and lets the notices finish leaving.
    fn expire_all(state: &mut Hidebar) {
        let later = Instant::now() + Duration::from_secs(60);
        state.expire(later);
        state.animate(later);
        state.animate(later + Duration::from_secs(1));
        state.frame = Instant::now();
    }

    #[test]
    fn first_readings_do_not_show() {
        let state = ready();
        assert!(!state.strip_wanted());
        assert!(MODULES.iter().all(|module| !state.visible(*module)));
    }

    #[test]
    fn switching_workspace_shows_it() {
        let mut state = ready();
        apply(&mut state, workspaces(1));
        assert!(!state.visible(Module::Workspaces), "active unchanged");
        apply(&mut state, workspaces(2));
        assert!(state.visible(Module::Workspaces) && state.strip_wanted());
        assert!(!state.visible(Module::Clock));
    }

    #[test]
    fn show_all_is_momentary() {
        let mut state = ready();
        apply(&mut state, show(Target::All));
        assert!(
            MODULES
                .iter()
                .all(|module| matches!(state.visibility(*module), Visibility::Until(_)))
        );
        expire_all(&mut state);
        assert!(!state.strip_wanted());
    }

    #[test]
    fn show_needs_a_reading() {
        let mut state = Hidebar::new();
        apply(&mut state, show(Target::Status));
        apply(&mut state, show(Target::Workspaces));
        assert!(!state.strip_wanted());
        apply(&mut state, show(Target::Clock));
        assert!(state.visible(Module::Clock));
    }

    #[test]
    fn toggle_pins_and_unpins() {
        let mut state = ready();
        apply(&mut state, toggle(Target::Status));
        assert_eq!(state.visibility(Module::Status), Visibility::Pinned);
        expire_all(&mut state);
        assert!(state.strip_wanted(), "pinned: does not expire");
        apply(&mut state, show(Target::Status));
        assert_eq!(
            state.visibility(Module::Status),
            Visibility::Pinned,
            "show does not unpin it"
        );
        apply(&mut state, input(Event::Network(Network::NoInternet)));
        assert_eq!(state.visibility(Module::Status), Visibility::Pinned);
        assert!(state.active_notices().is_empty(), "pinned: no notices");
        apply(&mut state, toggle(Target::Status));
        assert!(!state.visible(Module::Status));
        assert!(state.strip_wanted(), "the pill folds back first");
        state.animate(Instant::now() + Duration::from_secs(1));
        assert!(!state.strip_wanted());
    }

    #[test]
    fn toggle_all_pins_everything_then_unpins() {
        let mut state = ready();
        apply(&mut state, toggle(Target::All));
        assert!(
            MODULES
                .iter()
                .all(|module| state.visibility(*module) == Visibility::Pinned)
        );
        assert!(
            state.calendar.is_none(),
            "the clock is pinned without calendar"
        );
        apply(&mut state, toggle(Target::All));
        assert!(MODULES.iter().all(|module| !state.visible(*module)));

        // With a single module pinned, toggle all unpins instead of pinning.
        apply(&mut state, toggle(Target::Workspaces));
        apply(&mut state, toggle(Target::All));
        assert!(MODULES.iter().all(|module| !state.visible(*module)));
    }

    #[test]
    fn network_and_charger_changes_notify_without_status() {
        let mut state = ready();
        apply(&mut state, input(Event::Network(Network::Online)));
        assert!(state.active_notices().is_empty(), "no change");
        apply(&mut state, input(Event::Network(Network::Offline)));
        assert_eq!(state.active_notices(), [Notice::Network]);
        assert!(!state.visible(Module::Status) && state.strip_wanted());
        expire_all(&mut state);
        assert!(!state.strip_wanted());
        apply(&mut state, battery_event(79, BatteryStatus::Discharging));
        assert!(state.active_notices().is_empty(), "level change");
        apply(&mut state, battery_event(79, BatteryStatus::Charging));
        assert_eq!(state.active_notices(), [Notice::Battery]);
    }

    #[test]
    fn muting_notifies_but_volume_changes_do_not() {
        let audio = |muted, volume| input(Event::Audio(Audio { muted, volume }));
        let mut state = ready();
        apply(&mut state, audio(false, 50));
        assert!(state.active_notices().is_empty(), "first reading");
        apply(&mut state, audio(false, 70));
        assert!(state.active_notices().is_empty(), "volume change");
        apply(&mut state, audio(true, 70));
        assert_eq!(state.active_notices(), [Notice::Audio], "muted");
        expire_all(&mut state);
        apply(&mut state, audio(true, 0));
        assert!(state.active_notices().is_empty(), "still silent");
        apply(&mut state, audio(false, 30));
        assert_eq!(state.active_notices(), [Notice::Audio], "sound returns");
    }

    #[test]
    fn notices_accumulate_in_status_order_and_yield_to_status() {
        let mut state = ready();
        apply(
            &mut state,
            input(Event::Audio(Audio {
                muted: false,
                volume: 50,
            })),
        );
        apply(&mut state, battery_event(80, BatteryStatus::Charging));
        apply(
            &mut state,
            input(Event::Audio(Audio {
                muted: true,
                volume: 50,
            })),
        );
        apply(&mut state, input(Event::Network(Network::NoInternet)));
        assert_eq!(
            state.active_notices(),
            [Notice::Network, Notice::Audio, Notice::Battery]
        );
        apply(&mut state, input(Event::Network(Network::Online)));
        assert_eq!(
            state.active_notices().len(),
            3,
            "repeating does not duplicate"
        );

        apply(&mut state, show(Target::Status));
        assert!(state.active_notices().is_empty(), "full status covers them");
        expire_all(&mut state);
        assert!(state.active_notices().is_empty() && !state.strip_wanted());
    }

    #[test]
    fn low_battery_notice_persists_and_yields_only_to_status() {
        let mut state = ready();
        apply(&mut state, battery_event(20, BatteryStatus::Discharging));
        assert_eq!(state.active_notices(), [Notice::Battery]);
        assert!(state.strip_wanted() && state.pulsing());
        expire_all(&mut state);
        assert_eq!(state.active_notices(), [Notice::Battery], "does not expire");
        apply(&mut state, toggle(Target::Workspaces));
        assert_eq!(
            state.active_notices(),
            [Notice::Battery],
            "another module does not hide it"
        );
        apply(&mut state, toggle(Target::Workspaces));
        apply(&mut state, show(Target::Status));
        assert!(
            state.active_notices().is_empty() && state.pulsing(),
            "status pulses instead"
        );
        expire_all(&mut state);
        assert_eq!(state.active_notices(), [Notice::Battery]);
        apply(&mut state, battery_event(20, BatteryStatus::Charging));
        assert_eq!(state.active_notices(), [Notice::Battery], "charger notice");
        expire_all(&mut state);
        assert!(!state.strip_wanted() && !state.pulsing());
    }

    #[test]
    fn battery_severity_thresholds() {
        use BatteryStatus::*;
        assert_eq!(severity(&battery(26, Discharging)), Severity::Normal);
        assert_eq!(severity(&battery(25, Discharging)), Severity::Low);
        assert_eq!(severity(&battery(10, Discharging)), Severity::Critical);
        assert_eq!(severity(&battery(5, Charging)), Severity::Normal);
        assert_eq!(severity(&battery(5, NotCharging)), Severity::Normal);
    }

    #[test]
    fn pulse_fades_and_returns() {
        let at = |fraction: f32| pulse_alpha(PULSE_PERIOD.mul_f32(fraction));
        assert!((at(0.0) - 1.0).abs() < 1e-3);
        assert!((at(0.5) - (1.0 - PULSE_DIM)).abs() < 1e-3);
        assert!(at(0.25) < 1.0 && at(0.25) > 1.0 - PULSE_DIM);
        assert!((at(1.0) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn toggle_clock_opens_and_closes_calendar() {
        let mut state = ready();
        apply(&mut state, toggle(Target::Clock));
        assert!(state.calendar.is_some() && state.calendar_wanted());
        assert_eq!(state.visibility(Module::Clock), Visibility::Pinned);
        state.shift_calendar(1);
        let shown = state.calendar.as_ref().unwrap().month;
        assert_eq!(shown, calendar::shift_month(state.now.today, 1));
        state.calendar_today();
        assert_eq!(
            state.calendar.as_ref().unwrap().month,
            calendar::first_of_month(state.now.today)
        );
        apply(&mut state, toggle(Target::Clock));
        assert!(
            state.calendar.is_none() && state.calendar_wanted(),
            "folding back"
        );
        assert!(!state.visible(Module::Clock), "the clock hides again");
        state.tick(state.frame + Duration::from_secs(1));
        assert!(!state.calendar_wanted());
    }

    #[test]
    fn calendar_stretches_down_then_folds_back() {
        let mut state = ready();
        apply(&mut state, toggle(Target::Clock));
        let start = state.frame;
        let at = |state: &mut Hidebar, ms| {
            state.tick(start + Duration::from_millis(ms));
            state.calendar_frame().map(|(_, frame)| frame)
        };
        let frame = at(&mut state, 0).unwrap();
        assert_eq!((frame.size, frame.reveal), (0.0, 0.0));
        let frame = at(&mut state, 250).unwrap();
        assert!(frame.size > 0.9 && frame.reveal == 0.0, "stretch first");
        let frame = at(&mut state, 480).unwrap();
        assert_eq!((frame.size, frame.reveal), (1.0, 1.0));
        assert!(!state.animating());

        let month = state.calendar.as_ref().unwrap().month;
        state.close_calendar();
        let close = state.frame;
        let (shown, frame) = state.calendar_frame().unwrap();
        assert_eq!((shown, frame.size), (month, 1.0), "leaving starts full");
        assert!(state.calendar_wanted() && state.animating());
        state.tick(close + Duration::from_millis(320));
        assert!(state.calendar_frame().is_none() && !state.calendar_wanted());
    }

    #[test]
    fn toggle_all_folds_the_calendar_back() {
        let mut state = ready();
        apply(&mut state, toggle(Target::Clock));
        apply(&mut state, toggle(Target::All));
        assert!(state.calendar.is_none() && state.calendar_frame().is_some());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn calendar_waits_for_its_surface() {
        let mut state = ready();
        apply(&mut state, toggle(Target::Clock));
        let opened = state.frame + Duration::from_millis(250);
        state.calendar_opened(opened);
        assert_eq!(state.calendar_frame().unwrap().1.size, 0.0);
    }

    #[test]
    fn panel_stretches_down_then_folds_back() {
        let mut state = ready();
        apply(&mut state, toggle(Target::Status));
        let start = state.frame;
        let at = |state: &mut Hidebar, ms| {
            state.tick(start + Duration::from_millis(ms));
            state.panel_frame()
        };
        let frame = at(&mut state, 0).unwrap();
        assert_eq!((frame.size, frame.reveal), (0.0, 0.0));
        let frame = at(&mut state, 250).unwrap();
        assert!(frame.size > 0.9 && frame.reveal == 0.0, "stretch first");
        let frame = at(&mut state, 480).unwrap();
        assert_eq!((frame.size, frame.reveal), (1.0, 1.0));

        state.close_panel();
        let close = state.frame;
        assert_eq!(
            state.panel_frame().unwrap().size,
            1.0,
            "leaving starts full"
        );
        assert!(state.panel.is_none() && state.panel_wanted() && state.animating());
        state.tick(close + Duration::from_millis(320));
        assert!(state.panel_frame().is_none() && !state.panel_wanted());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn panel_waits_for_its_surface() {
        let mut state = ready();
        apply(&mut state, toggle(Target::Status));
        let opened = state.frame + Duration::from_millis(250);
        state.panel_opened(opened);
        assert_eq!(state.panel_frame().unwrap().size, 0.0);
    }

    #[test]
    fn calendar_respects_previously_pinned_clock() {
        let mut state = ready();
        apply(&mut state, toggle(Target::All));
        apply(&mut state, toggle(Target::Clock));
        state.close_calendar();
        assert_eq!(state.visibility(Module::Clock), Visibility::Pinned);
    }

    #[test]
    fn calendar_closes_on_focus_loss_only_after_focus() {
        let mut state = ready();
        apply(&mut state, toggle(Target::Clock));
        state.mark_calendar_focused(false);
        assert!(state.calendar.is_some(), "never focused: stays open");
        state.mark_calendar_focused(true);
        state.mark_calendar_focused(false);
        assert!(state.calendar.is_none());
    }

    #[test]
    fn toggle_status_opens_and_closes_panel() {
        let mut state = ready();
        apply(&mut state, toggle(Target::Status));
        assert!(state.panel.is_some());
        assert_eq!(state.visibility(Module::Status), Visibility::Pinned);
        apply(&mut state, toggle(Target::Status));
        assert!(state.panel.is_none() && !state.visible(Module::Status));
    }

    #[test]
    fn panel_respects_previously_pinned_status() {
        let mut state = ready();
        apply(&mut state, toggle(Target::All));
        apply(&mut state, toggle(Target::Status));
        state.close_panel();
        assert_eq!(state.visibility(Module::Status), Visibility::Pinned);
        apply(&mut state, toggle(Target::Status));
        apply(&mut state, toggle(Target::All));
        assert!(
            state.panel.is_none(),
            "toggle all unpins and closes the panel"
        );
    }

    #[test]
    fn panel_closes_on_focus_loss_only_after_focus() {
        let mut state = ready();
        apply(&mut state, toggle(Target::Status));
        state.mark_panel_focused(false);
        assert!(state.panel.is_some(), "never focused: stays open");
        state.mark_panel_focused(true);
        state.mark_panel_focused(false);
        assert!(state.panel.is_none());
    }

    #[test]
    fn session_actions_need_a_second_press_in_time() {
        let mut state = ready();
        let start = Instant::now();
        assert_eq!(state.press(Action::Poweroff, start), None, "panel closed");

        apply(&mut state, toggle(Target::Status));
        assert_eq!(
            state.press(Action::Poweroff, start),
            None,
            "first press arms"
        );
        assert!(state.momentary(), "the arm expiry needs ticks");
        assert_eq!(
            state.press(Action::Logout, start),
            None,
            "another action re-arms"
        );
        assert_eq!(state.panel.as_ref().unwrap().armed(), Some(Action::Logout));
        assert_eq!(state.press(Action::Logout, start), Some(Action::Logout));
        assert!(state.panel.is_none(), "confirming closes the panel");

        apply(&mut state, toggle(Target::Status));
        state.press(Action::Poweroff, start);
        state.disarm();
        assert_eq!(state.press(Action::Poweroff, start), None, "disarmed");
        state.expire(start + ARM_DURATION);
        assert_eq!(state.panel.as_ref().unwrap().armed(), None, "expired");
        assert_eq!(
            state.press(Action::Poweroff, start + ARM_DURATION),
            None,
            "late press arms again"
        );
    }

    /// The single notice frame at `instant`.
    fn frame_at(state: &mut Hidebar, instant: Instant) -> Vec<NoticeFrame> {
        state.frame = instant;
        state.animate(instant);
        state.notice_frames()
    }

    #[test]
    fn notice_stretches_then_reveals() {
        let mut state = ready();
        apply(&mut state, input(Event::Network(Network::Offline)));
        let start = state.frame;
        let frames = frame_at(&mut state, start);
        assert_eq!(frames.len(), 1);
        assert_eq!(
            (frames[0].size, frames[0].reveal),
            (0.0, 0.0),
            "starts as a circle"
        );
        assert!(state.animating());

        let frames = frame_at(&mut state, start + Duration::from_millis(150));
        assert!(
            frames[0].size > 0.8 && frames[0].reveal == 0.0,
            "stretched, icon not yet"
        );
        assert_eq!(frames[0].background, 1.0);

        let frames = frame_at(&mut state, start + Duration::from_millis(250));
        assert_eq!(frames[0].reveal, 0.0, "no icon until fully stretched");

        let frames = frame_at(&mut state, start + Duration::from_millis(300));
        assert!(frames[0].size == 1.0 && frames[0].reveal > 0.0 && frames[0].reveal < 1.0);

        let frames = frame_at(&mut state, start + Duration::from_millis(480));
        assert_eq!((frames[0].size, frames[0].reveal), (1.0, 1.0));
        assert!(!state.animating(), "settled: no frames");
    }

    #[test]
    fn expired_notice_leaves_before_closing_the_strip() {
        let mut state = ready();
        apply(&mut state, input(Event::Network(Network::Offline)));
        let end = state.frame + NOTICE_DURATION;
        state.expire(end);
        assert!(state.active_notices().is_empty());

        let frames = frame_at(&mut state, end);
        assert_eq!(
            (frames[0].size, frames[0].reveal),
            (1.0, 1.0),
            "leaving starts full"
        );
        assert!(state.strip_wanted() && state.animating());

        let frames = frame_at(&mut state, end + Duration::from_millis(120));
        assert_eq!(frames[0].reveal, 0.0, "icon gone first");
        assert_eq!(
            frames[0].size, 1.0,
            "the pill shrinks only after the icon is gone"
        );

        assert!(frame_at(&mut state, end + Duration::from_millis(320)).is_empty());
        assert!(!state.strip_wanted() && !state.animating());
    }

    #[test]
    fn notice_back_while_leaving_reverses_from_its_width() {
        let mut state = ready();
        apply(&mut state, input(Event::Network(Network::Offline)));
        let end = state.frame + NOTICE_DURATION;
        state.expire(end);
        frame_at(&mut state, end);
        let middle = end + Duration::from_millis(200);
        let leaving = frame_at(&mut state, middle)[0].size;
        assert!(leaving > 0.0 && leaving < 1.0);

        state.notify(Notice::Network);
        let back = frame_at(&mut state, middle)[0].size;
        assert!((back - leaving).abs() < 0.01, "{back} vs {leaving}");
        assert!(state.animating());
    }

    #[test]
    fn simultaneous_notices_stagger_right_to_left() {
        let mut state = ready();
        let start = Instant::now();
        state.notify(Notice::Network);
        state.notify(Notice::Battery);
        frame_at(&mut state, start);
        let frames = frame_at(&mut state, start + Duration::from_millis(20));
        let size = |notice| frames.iter().find(|f| f.notice == notice).unwrap().size;
        assert!(size(Notice::Battery) > 0.0, "rightmost first");
        assert_eq!(size(Notice::Network), 0.0, "next one 40 ms later");
    }

    #[test]
    fn full_status_replaces_notices_at_once() {
        let mut state = ready();
        apply(&mut state, input(Event::Network(Network::Offline)));
        assert!(!state.notice_frames().is_empty());
        apply(&mut state, show(Target::Status));
        assert!(state.notice_frames().is_empty(), "notices go at once");
        assert!(state.module_frame(Module::Status).is_some() && state.animating());
    }

    #[test]
    fn module_pills_stretch_in_and_fold_back() {
        let mut state = ready();
        apply(&mut state, show(Target::Clock));
        let start = state.frame;
        let at = |state: &mut Hidebar, ms| {
            let instant = start + Duration::from_millis(ms);
            state.frame = instant;
            state.tick(instant);
            state.module_frame(Module::Clock)
        };
        let frame = at(&mut state, 0).unwrap();
        assert_eq!((frame.size, frame.reveal), (0.0, 0.0));
        let frame = at(&mut state, 250).unwrap();
        assert!(frame.size > 0.9 && frame.reveal == 0.0, "stretch first");
        let frame = at(&mut state, 480).unwrap();
        assert_eq!((frame.size, frame.reveal), (1.0, 1.0));

        let end = 3000;
        let frame = at(&mut state, end).unwrap();
        assert_eq!(frame.size, 1.0, "leaving starts full");
        assert!(!state.visible(Module::Clock) && state.strip_wanted());
        assert!(at(&mut state, end + 320).is_none());
        assert!(!state.strip_wanted());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn notices_wait_for_the_strip_surface() {
        let mut state = ready();
        apply(&mut state, input(Event::Network(Network::Offline)));
        let opened = state.frame + Duration::from_millis(250);
        state.strip_opened(opened);
        let frames = frame_at(&mut state, opened);
        assert_eq!(
            frames[0].size, 0.0,
            "the stretch starts when the strip appears"
        );
    }
}
