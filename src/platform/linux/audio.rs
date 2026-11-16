//! Audio source: mute and volume of the default output, via `pactl`
//! (PipeWire with pipewire-pulse, or PulseAudio). Learns about changes with `pactl subscribe`.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::event::{Audio, Event};

const RETRY: Duration = Duration::from_secs(5);

/// Listens on its own thread. Emits the initial state and then only changes.
/// `emit` returns `false` once the receiver is gone. Does nothing without `pactl`.
pub fn listen(emit: impl Fn(Event) -> bool + Send + 'static) {
    std::thread::spawn(move || {
        let mut last = None;
        // Reads and emits if changed; returns `false` if the receiver is gone.
        let refresh = |last: &mut Option<Audio>| match read() {
            Some(audio) if *last != Some(audio) => {
                *last = Some(audio);
                emit(Event::Audio(audio))
            }
            _ => true,
        };
        loop {
            if !refresh(&mut last) {
                return;
            }
            let child = Command::new("pactl")
                .arg("subscribe")
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            let Ok(mut child) = child else {
                // No pactl: no audio module.
                return;
            };
            if let Some(stdout) = child.stdout.take() {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if is_relevant(&line) && !refresh(&mut last) {
                        let _ = child.kill();
                        return;
                    }
                }
            }
            // The sound server restarted or pactl exited: retry.
            let _ = child.wait();
            std::thread::sleep(RETRY);
        }
    });
}

/// Changes to a sink (volume, mute) or to the server (default sink).
fn is_relevant(line: &str) -> bool {
    line.starts_with("Event 'change' on sink #")
        || line.starts_with("Event 'new' on sink #")
        || line.starts_with("Event 'change' on server")
}

fn read() -> Option<Audio> {
    let mute = pactl(&["get-sink-mute", "@DEFAULT_SINK@"])?;
    let volume = pactl(&["get-sink-volume", "@DEFAULT_SINK@"])?;
    Some(Audio {
        muted: parse_mute(&mute)?,
        volume: parse_volume(&volume)?,
    })
}

fn pactl(args: &[&str]) -> Option<String> {
    let output = Command::new("pactl")
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `Mute: yes` / `Mute: no`.
fn parse_mute(output: &str) -> Option<bool> {
    match output.trim().strip_prefix("Mute:")?.trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

/// `Volume: front-left: 65536 / 100% / 0.00 dB, ...` → the first percentage.
fn parse_volume(output: &str) -> Option<u16> {
    output
        .split('/')
        .find_map(|part| part.trim().strip_suffix('%')?.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mute() {
        assert_eq!(parse_mute("Mute: yes\n"), Some(true));
        assert_eq!(parse_mute("Mute: no"), Some(false));
        assert_eq!(parse_mute("Connection failure"), None);
    }

    #[test]
    fn parses_volume() {
        let output = "Volume: front-left: 65536 / 100% / 0.00 dB,   front-right: 65536 / 100% / 0.00 dB\n        balance 0.00\n";
        assert_eq!(parse_volume(output), Some(100));
        assert_eq!(parse_volume("Volume: mono: 0 /   0% / -inf dB"), Some(0));
        assert_eq!(
            parse_volume("Volume: front-left: 98304 / 150% / 10.57 dB"),
            Some(150)
        );
        assert_eq!(parse_volume("no data"), None);
    }

    #[test]
    fn filters_events() {
        assert!(is_relevant("Event 'change' on sink #56"));
        assert!(is_relevant("Event 'change' on server #4294967295"));
        assert!(!is_relevant("Event 'change' on source-output #12"));
        assert!(!is_relevant("Event 'change' on sink-input #90"));
        assert!(!is_relevant("Event 'remove' on client #3"));
    }

    #[test]
    fn silent_when_muted_or_zero() {
        assert!(
            Audio {
                muted: true,
                volume: 80
            }
            .silent()
        );
        assert!(
            Audio {
                muted: false,
                volume: 0
            }
            .silent()
        );
        assert!(
            !Audio {
                muted: false,
                volume: 1
            }
            .silent()
        );
    }

    /// `cargo test -- --ignored`: reads the real audio output.
    #[test]
    #[ignore = "requires pactl and a sound server"]
    fn reads_real_audio() {
        let audio = read().expect("pactl");
        println!("{audio:?}");
    }
}
