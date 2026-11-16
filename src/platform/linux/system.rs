//! System source: CPU (`/proc/stat`) and RAM (`/proc/meminfo`) usage every few seconds.

use std::fs;
use std::time::Duration;

use crate::event::{Event, System};

const POLL: Duration = Duration::from_secs(3);

/// Cumulative CPU counters: total time and idle time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CpuTimes {
    total: u64,
    idle: u64,
}

/// Polls on its own thread and emits only when the percentages change.
/// `emit` returns `false` once the receiver is gone.
pub fn listen(emit: impl Fn(Event) -> bool + Send + 'static) {
    std::thread::spawn(move || {
        let mut previous = read_cpu();
        let mut last = None;
        loop {
            std::thread::sleep(POLL);
            let current = read_cpu();
            let (Some(before), Some(after)) = (previous, current) else {
                previous = current;
                continue;
            };
            previous = current;
            let Some(ram) = read_ram() else { continue };
            let system = System {
                cpu: cpu_percent(before, after),
                ram,
            };
            if last != Some(system) {
                last = Some(system);
                if !emit(Event::System(system)) {
                    return;
                }
            }
        }
    });
}

fn read_cpu() -> Option<CpuTimes> {
    parse_cpu(&fs::read_to_string("/proc/stat").ok()?)
}

fn read_ram() -> Option<u8> {
    parse_ram(&fs::read_to_string("/proc/meminfo").ok()?)
}

/// First line `cpu  user nice system idle iowait irq softirq steal ...`.
/// Idle = idle + iowait. guest/guest_nice are ignored (already included in user/nice).
fn parse_cpu(stat: &str) -> Option<CpuTimes> {
    let line = stat.lines().find(|line| line.starts_with("cpu "))?;
    let fields: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if fields.len() < 5 {
        return None;
    }
    Some(CpuTimes {
        total: fields.iter().sum(),
        idle: fields[3] + fields[4],
    })
}

fn cpu_percent(before: CpuTimes, after: CpuTimes) -> u8 {
    let total = after.total.saturating_sub(before.total);
    let idle = after.idle.saturating_sub(before.idle);
    if total == 0 {
        return 0;
    }
    let busy = total.saturating_sub(idle);
    ((busy * 100 + total / 2) / total).min(100) as u8
}

/// `1 - MemAvailable / MemTotal`, rounded.
fn parse_ram(meminfo: &str) -> Option<u8> {
    let field = |name: &str| -> Option<u64> {
        meminfo
            .lines()
            .find_map(|line| line.strip_prefix(name))?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    };
    let total = field("MemTotal:")?;
    let available = field("MemAvailable:")?;
    if total == 0 {
        return None;
    }
    let used = total.saturating_sub(available);
    Some(((used * 100 + total / 2) / total).min(100) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cpu_line() {
        let stat = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 1 2 3 4 5 6 7 8\n";
        assert_eq!(
            parse_cpu(stat),
            Some(CpuTimes {
                total: 1000,
                idle: 850
            })
        );
        assert_eq!(parse_cpu("intr 1 2 3"), None);
    }

    #[test]
    fn computes_cpu_percent() {
        let before = CpuTimes {
            total: 1000,
            idle: 800,
        };
        let after = CpuTimes {
            total: 1200,
            idle: 900,
        };
        assert_eq!(cpu_percent(before, after), 50);
        assert_eq!(cpu_percent(before, before), 0, "no elapsed time");
    }

    #[test]
    fn parses_meminfo() {
        let meminfo = "MemTotal:       16000000 kB\nMemFree:  1 kB\nMemAvailable:    4000000 kB\n";
        assert_eq!(parse_ram(meminfo), Some(75));
        assert_eq!(parse_ram("MemTotal: 10 kB\n"), None);
    }

    /// `cargo test -- --ignored`: reads the real `/proc`.
    #[test]
    #[ignore = "reads the machine's /proc"]
    fn reads_real_proc() {
        let before = read_cpu().unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let after = read_cpu().unwrap();
        let system = System {
            cpu: cpu_percent(before, after),
            ram: read_ram().unwrap(),
        };
        println!("{system:?}");
        assert!(system.cpu <= 100 && system.ram > 0);
    }
}
