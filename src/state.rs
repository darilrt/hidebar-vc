//! Estado del daemon: visibilidad, avisos, calendario y pulso.
//!
//! No conoce iced ni ventanas: recibe `Input` del shell y expone consultas
//! (`strip_wanted`, `calendar_wanted`, `active_notices`, ...) para que cada
//! plataforma decida cómo presentarlas.

use std::time::{Duration, Instant};

use chrono::NaiveDate;

use crate::calendar;
use crate::command::{Command, Target};
use crate::event::{Audio, Battery, Event, Network, System, Workspaces};

/// Tiempo visible de cada módulo en modo momentáneo; volver a pedirlo lo reinicia.
const WORKSPACES_DURATION: Duration = Duration::from_millis(1500);
const CLOCK_DURATION: Duration = Duration::from_secs(3);
const STATUS_DURATION: Duration = Duration::from_secs(3);
/// Tiempo visible de un aviso; repetir el mismo aviso lo reinicia.
const NOTICE_DURATION: Duration = Duration::from_secs(3);
/// Revisión de vencimientos mientras hay algo momentáneo.
pub(crate) const TICK: Duration = Duration::from_millis(250);
/// Refresco de la hora con el reloj fijado y nada momentáneo.
pub(crate) const CLOCK_TICK: Duration = Duration::from_secs(1);
/// Intervalo de cuadros mientras algo pulsa (20 fps: suficiente para un fade lento).
pub(crate) const FRAME: Duration = Duration::from_millis(50);

/// Umbrales de batería (descargando), en porcentaje.
const BATTERY_LOW: u8 = 25;
const BATTERY_CRITICAL: u8 = 10;
/// Pulso de batería baja: la opacidad baja hasta `1 - PULSE_DIM` y vuelve, cada `PULSE_PERIOD`.
const PULSE_PERIOD: Duration = Duration::from_secs(2);
const PULSE_DIM: f32 = 0.7;

/// Lo que llega al daemon desde fuera del shell: comandos y eventos de las fuentes.
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

/// Aviso: un elemento de `status` mostrado solo, porque cambió (ADR-0004). En el orden de `status`.
/// Para agregar uno: una variante aquí y en `NOTICES`, su regla en `on_event` y su dibujo en `view`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Notice {
    /// Cambió la conectividad.
    Network,
    /// Pasó de sonido a silencio o al revés.
    Audio,
    /// Se conectó o desconectó el cargador; también, sin vencimiento, la batería baja.
    Battery,
}

const NOTICES: [Notice; 3] = [Notice::Network, Notice::Audio, Notice::Battery];

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

/// Calendario abierto.
pub(crate) struct Calendar {
    /// Primer día del mes mostrado.
    pub(crate) month: NaiveDate,
    /// El reloj ya estaba fijado al abrir: al cerrar el calendario se respeta.
    pub(crate) clock_was_pinned: bool,
    /// Recibió el foco alguna vez; sólo entonces perderlo lo cierra.
    pub(crate) focused: bool,
}

pub(crate) struct Hidebar {
    visibility: [Visibility; 3],
    /// Vencimiento de cada aviso momentáneo, por `Notice`.
    notices: [Option<Instant>; 3],
    /// Últimos datos conocidos; `None` hasta el primer evento de cada fuente.
    pub(crate) workspaces: Option<Workspaces>,
    pub(crate) battery: Option<Battery>,
    pub(crate) system: Option<System>,
    pub(crate) network: Option<Network>,
    pub(crate) audio: Option<Audio>,
    pub(crate) calendar: Option<Calendar>,
    /// Origen del pulso de batería baja y último cuadro.
    pulse_origin: Instant,
    frame: Instant,
    pub(crate) now: Now,
}

/// Hora y fecha ya formateadas para mostrar, y el día de hoy.
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
            workspaces: None,
            battery: None,
            system: None,
            network: None,
            audio: None,
            calendar: None,
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

    /// Sin una lectura de la fuente no hay nada que mostrar.
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

    /// Momentáneo, salvo que esté fijado.
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
        let next = match self.visibility(module) {
            Visibility::Pinned => Visibility::Hidden,
            _ if self.available(module) => Visibility::Pinned,
            current => current,
        };
        self.set_visibility(module, next);
    }

    /// ADR-0002 Q2: si algo está fijado, desfija todo; si no, fija los tres (el reloj, sin calendario).
    fn toggle_all(&mut self) {
        let any_pinned = MODULES
            .iter()
            .any(|module| self.visibility(*module) == Visibility::Pinned);
        if any_pinned {
            self.calendar = None;
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
        if let Some(calendar) = self.calendar.take()
            && !calendar.clock_was_pinned
        {
            self.set_visibility(Module::Clock, Visibility::Hidden);
        }
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

    fn expire(&mut self, instant: Instant) {
        for visibility in &mut self.visibility {
            if matches!(visibility, Visibility::Until(until) if *until <= instant) {
                *visibility = Visibility::Hidden;
            }
        }
        for notice in &mut self.notices {
            notice.take_if(|until| *until <= instant);
        }
    }

    /// Muestra un aviso un momento (o reinicia su tiempo si ya estaba). Con `status` visible el cambio ya
    /// se ve ahí: no se registra, para que no aparezca tarde al ocultarse `status`.
    fn notify(&mut self, notice: Notice) {
        if !self.visible(Module::Status) {
            self.notices[notice as usize] = Some(Instant::now() + NOTICE_DURATION);
        }
    }

    /// Avisos a dibujar, en orden. Ninguno con `status` visible: la pastilla completa ya los contiene.
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

    /// Opacidad actual del pulso.
    pub(crate) fn pulse(&self) -> f32 {
        pulse_alpha(self.frame.saturating_duration_since(self.pulse_origin))
    }

    /// Hay algo pulsando en pantalla: con batería baja, su icono está en `status` o en los avisos.
    pub(crate) fn pulsing(&self) -> bool {
        self.strip_wanted() && self.severity() != Severity::Normal
    }

    /// La franja debe estar abierta: algún módulo visible o algún aviso activo.
    pub(crate) fn strip_wanted(&self) -> bool {
        MODULES.iter().any(|module| self.visible(*module)) || !self.active_notices().is_empty()
    }

    pub(crate) fn calendar_wanted(&self) -> bool {
        self.calendar.is_some()
    }

    /// Hay algo con vencimiento (módulo momentáneo o aviso); el shell debe revisar el reloj.
    pub(crate) fn momentary(&self) -> bool {
        self.visibility
            .iter()
            .any(|visibility| matches!(visibility, Visibility::Until(_)))
            || self.notices.iter().any(Option::is_some)
    }

    pub(crate) fn handle(&mut self, input: Input) {
        match input {
            // Salir es decisión del shell, no del estado.
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
    }

    /// Avanza la hora y vence lo momentáneo.
    pub(crate) fn tick(&mut self, instant: Instant) {
        self.now = now();
        self.frame = instant;
        self.expire(instant);
    }

    /// Nuevo cuadro del pulso sin reformatear la hora (20 fps).
    pub(crate) fn frame(&mut self, instant: Instant) {
        self.frame = instant;
    }

    /// La presentación decide qué eventos merecen aparecer.
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
                // Conectar o desconectar el cargador se avisa; el nivel bajo se avisa mientras dure.
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
                // Sólo interesa si se oye o no; los cambios de volumen no aparecen.
                let changed = self.audio.is_some_and(|old| old.silent() != audio.silent());
                self.audio = Some(audio);
                if changed {
                    self.notify(Notice::Audio);
                }
            }
        }
    }
}

/// Módulos a los que se dirige un comando.
fn modules(target: Target) -> &'static [Module] {
    match target {
        Target::Workspaces => &[Module::Workspaces],
        Target::Clock => &[Module::Clock],
        Target::Status => &[Module::Status],
        Target::All => &MODULES,
    }
}

/// Opacidad del pulso: 1 al inicio de cada periodo, `1 - PULSE_DIM` a la mitad, y de vuelta a 1.
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
        date: calendar::spanish_date(now.date_naive()),
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

    /// Estado con lecturas de todas las fuentes.
    fn ready() -> Hidebar {
        let mut state = Hidebar::new();
        apply(&mut state, workspaces(1));
        apply(&mut state, battery_event(80, BatteryStatus::Discharging));
        apply(&mut state, input(Event::System(System { cpu: 5, ram: 40 })));
        apply(&mut state, input(Event::Network(Network::Online)));
        state
    }

    /// Hace vencer todo lo momentáneo.
    fn expire_all(state: &mut Hidebar) {
        state.expire(Instant::now() + Duration::from_secs(60));
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
        assert!(!state.visible(Module::Workspaces), "sin cambio de activo");
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
        assert!(state.strip_wanted(), "fijado: no vence");
        apply(&mut state, show(Target::Status));
        assert_eq!(
            state.visibility(Module::Status),
            Visibility::Pinned,
            "show no lo desfija"
        );
        apply(&mut state, input(Event::Network(Network::NoInternet)));
        assert_eq!(state.visibility(Module::Status), Visibility::Pinned);
        assert!(state.active_notices().is_empty(), "fijado: sin avisos");
        apply(&mut state, toggle(Target::Status));
        assert!(!state.visible(Module::Status) && !state.strip_wanted());
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
        assert!(state.calendar.is_none(), "el reloj se fija sin calendario");
        apply(&mut state, toggle(Target::All));
        assert!(MODULES.iter().all(|module| !state.visible(*module)));

        // Con un solo módulo fijado, toggle all desfija en vez de fijar.
        apply(&mut state, toggle(Target::Workspaces));
        apply(&mut state, toggle(Target::All));
        assert!(MODULES.iter().all(|module| !state.visible(*module)));
    }

    #[test]
    fn network_and_charger_changes_notify_without_status() {
        let mut state = ready();
        apply(&mut state, input(Event::Network(Network::Online)));
        assert!(state.active_notices().is_empty(), "sin cambio");
        apply(&mut state, input(Event::Network(Network::Offline)));
        assert_eq!(state.active_notices(), [Notice::Network]);
        assert!(!state.visible(Module::Status) && state.strip_wanted());
        expire_all(&mut state);
        assert!(!state.strip_wanted());
        apply(&mut state, battery_event(79, BatteryStatus::Discharging));
        assert!(state.active_notices().is_empty(), "cambio de nivel");
        apply(&mut state, battery_event(79, BatteryStatus::Charging));
        assert_eq!(state.active_notices(), [Notice::Battery]);
    }

    #[test]
    fn muting_notifies_but_volume_changes_do_not() {
        let audio = |muted, volume| input(Event::Audio(Audio { muted, volume }));
        let mut state = ready();
        apply(&mut state, audio(false, 50));
        assert!(state.active_notices().is_empty(), "primera lectura");
        apply(&mut state, audio(false, 70));
        assert!(state.active_notices().is_empty(), "cambio de volumen");
        apply(&mut state, audio(true, 70));
        assert_eq!(state.active_notices(), [Notice::Audio], "silenciado");
        expire_all(&mut state);
        apply(&mut state, audio(true, 0));
        assert!(state.active_notices().is_empty(), "sigue sin oírse");
        apply(&mut state, audio(false, 30));
        assert_eq!(state.active_notices(), [Notice::Audio], "vuelve el sonido");
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
        assert_eq!(state.active_notices().len(), 3, "repetir no duplica");

        apply(&mut state, show(Target::Status));
        assert!(
            state.active_notices().is_empty(),
            "status completo los cubre"
        );
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
        assert_eq!(state.active_notices(), [Notice::Battery], "no vence");
        apply(&mut state, toggle(Target::Workspaces));
        assert_eq!(
            state.active_notices(),
            [Notice::Battery],
            "otro módulo no lo oculta"
        );
        apply(&mut state, toggle(Target::Workspaces));
        apply(&mut state, show(Target::Status));
        assert!(
            state.active_notices().is_empty() && state.pulsing(),
            "status pulsa en su lugar"
        );
        expire_all(&mut state);
        assert_eq!(state.active_notices(), [Notice::Battery]);
        apply(&mut state, battery_event(20, BatteryStatus::Charging));
        assert_eq!(
            state.active_notices(),
            [Notice::Battery],
            "aviso del cargador"
        );
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
        assert!(state.calendar.is_none() && !state.calendar_wanted());
        assert!(!state.visible(Module::Clock), "el reloj vuelve a ocultarse");
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
        assert!(state.calendar.is_some(), "nunca tuvo foco: no se cierra");
        state.mark_calendar_focused(true);
        state.mark_calendar_focused(false);
        assert!(state.calendar.is_none());
    }
}
