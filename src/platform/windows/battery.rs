//! Battery: `GetSystemPowerStatus` every 10 s.
//!
//! `BatteryFlag` does not tell "full" from "plugged in, not charging"; it is inferred from
//! the percentage. `255` means unknown and no level is made up.

use std::mem::zeroed;
use std::time::Duration;

use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

use crate::event::{Battery, BatteryStatus, Event};

const POLL: Duration = Duration::from_secs(10);
const NO_BATTERY: u8 = 128;
const CHARGING: u8 = 8;
const UNKNOWN: u8 = 255;

pub(crate) fn listen(emit: impl Fn(Event) -> bool + Send + 'static) {
    std::thread::spawn(move || {
        let mut last = None;

        loop {
            if let Some(battery) = read()
                && last.as_ref() != Some(&battery)
            {
                last = Some(battery.clone());
                if !emit(Event::Battery(battery)) {
                    return;
                }
            }

            std::thread::sleep(POLL);
        }
    });
}

fn read() -> Option<Battery> {
    // SAFETY: zeroed out-struct; the API fills it or returns 0.
    let mut status: SYSTEM_POWER_STATUS = unsafe { zeroed() };

    if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
        return None;
    }

    map(
        status.ACLineStatus,
        status.BatteryFlag,
        status.BatteryLifePercent,
    )
}

fn map(ac_line: u8, battery_flag: u8, percent: u8) -> Option<Battery> {
    if battery_flag == UNKNOWN || battery_flag & NO_BATTERY != 0 || percent > 100 {
        return None;
    }

    let status = match ac_line {
        0 => BatteryStatus::Discharging,
        1 if battery_flag & CHARGING != 0 => BatteryStatus::Charging,
        1 if percent == 100 => BatteryStatus::Full,
        1 => BatteryStatus::NotCharging,
        _ => BatteryStatus::Unknown,
    };

    Some(Battery { percent, status })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_power_status() {
        assert_eq!(map(0, 0, 80).unwrap().status, BatteryStatus::Discharging);
        assert_eq!(map(1, 8, 80).unwrap().status, BatteryStatus::Charging);
        assert_eq!(map(1, 0, 100).unwrap().status, BatteryStatus::Full);
        assert_eq!(map(1, 0, 80).unwrap().status, BatteryStatus::NotCharging);
        assert_eq!(map(255, 0, 80).unwrap().status, BatteryStatus::Unknown);
    }

    #[test]
    fn ignores_unknown_and_missing_battery() {
        assert_eq!(map(0, NO_BATTERY, 80), None);
        assert_eq!(map(0, UNKNOWN, 80), None);
        assert_eq!(map(0, 0, 255), None);
    }

    #[test]
    fn keeps_low_levels() {
        let battery = map(0, 0, 5).unwrap();
        assert_eq!(battery.percent, 5);
        assert!(!battery.plugged());
    }
}
