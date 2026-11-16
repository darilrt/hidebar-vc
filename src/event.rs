//! Events emitted by the sources. They carry facts, not presentation:
//! the UI decides whether to show them, how and for how long.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// No workspaces source on Windows v1; kept for Linux.
    #[cfg_attr(windows, allow(dead_code))]
    Workspaces(Workspaces),
    Battery(Battery),
    System(System),
    Network(Network),
    Audio(Audio),
}

/// Default audio output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Audio {
    pub muted: bool,
    /// Volume of the first channel, in percent (may exceed 100).
    pub volume: u16,
}

impl Audio {
    /// Nothing is audible: muted or at 0 %.
    pub fn silent(&self) -> bool {
        self.muted || self.volume == 0
    }
}

/// CPU and RAM usage, in percent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct System {
    pub cpu: u8,
    pub ram: u8,
}

/// Connectivity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    /// Internet is reachable.
    Online,
    /// There is a default route (local network), but the reachability probe fails.
    NoInternet,
    /// No default route.
    Offline,
}

/// Existing regular workspaces, sorted by id, and the active one on the focused monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspaces {
    pub ids: Vec<i32>,
    pub active: i32,
}

/// Battery state as reported by the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub status: BatteryStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryStatus {
    Charging,
    Discharging,
    Full,
    /// Plugged in but not charging (e.g. due to a charge limit).
    NotCharging,
    Unknown,
}

impl Battery {
    /// Plugged into the charger. `Unknown` counts as plugged so we don't alarm without data.
    pub fn plugged(&self) -> bool {
        self.status != BatteryStatus::Discharging
    }
}
