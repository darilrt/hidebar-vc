//! Battery source: polls `/sys/class/power_supply/BAT*` and emits an event when it changes.
//!
//! `HIDEBAR_BATTERY_PATH` may point to a directory with `capacity` and `status` files
//! to simulate levels in manual tests.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::event::{Battery, BatteryStatus, Event};

const POWER_SUPPLY: &str = "/sys/class/power_supply";
const POLL: Duration = Duration::from_secs(10);

/// Polls the battery on its own thread. Emits the initial state and then only changes.
/// `emit` returns `false` once the receiver is gone. Does nothing without a battery.
pub fn listen(emit: impl Fn(Event) -> bool + Send + 'static) {
    let Some(dir) = battery_dir() else {
        return;
    };
    std::thread::spawn(move || {
        let mut last = None;
        loop {
            match read(&dir) {
                Ok(battery) if last.as_ref() != Some(&battery) => {
                    last = Some(battery.clone());
                    if !emit(Event::Battery(battery)) {
                        return;
                    }
                }
                Ok(_) => {}
                Err(error) => eprintln!("hidebar: battery ({}): {error}", dir.display()),
            }
            std::thread::sleep(POLL);
        }
    });
}

fn battery_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("HIDEBAR_BATTERY_PATH") {
        return Some(PathBuf::from(path));
    }
    let mut batteries: Vec<PathBuf> = fs::read_dir(POWER_SUPPLY)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            fs::read_to_string(path.join("type")).is_ok_and(|kind| kind.trim() == "Battery")
        })
        .collect();
    batteries.sort();
    batteries.into_iter().next()
}

fn read(dir: &Path) -> io::Result<Battery> {
    let capacity = fs::read_to_string(dir.join("capacity"))?;
    let status = fs::read_to_string(dir.join("status")).unwrap_or_default();
    parse(&capacity, &status).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid capacity: {capacity:?}"),
        )
    })
}

fn parse(capacity: &str, status: &str) -> Option<Battery> {
    let percent = capacity.trim().parse::<u8>().ok()?.min(100);
    let status = match status.trim() {
        "Charging" => BatteryStatus::Charging,
        "Discharging" => BatteryStatus::Discharging,
        "Full" => BatteryStatus::Full,
        "Not charging" => BatteryStatus::NotCharging,
        _ => BatteryStatus::Unknown,
    };
    Some(Battery { percent, status })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sysfs_values() {
        assert_eq!(
            parse("80\n", "Discharging\n"),
            Some(Battery {
                percent: 80,
                status: BatteryStatus::Discharging
            })
        );
        assert_eq!(
            parse("100", "Not charging").map(|b| b.status),
            Some(BatteryStatus::NotCharging)
        );
        assert_eq!(
            parse("5", "").map(|b| b.status),
            Some(BatteryStatus::Unknown)
        );
        assert_eq!(parse("130", "Full").map(|b| b.percent), Some(100));
        assert_eq!(parse("abc", "Full"), None);
    }

    /// `cargo test -- --ignored`: reads the machine's real battery (read-only).
    #[test]
    #[ignore = "requires a battery in /sys/class/power_supply"]
    fn reads_real_battery() {
        let dir = battery_dir().expect("no battery");
        let battery = read(&dir).unwrap();
        println!("{} → {battery:?}", dir.display());
        assert!(battery.percent <= 100);
    }
}
