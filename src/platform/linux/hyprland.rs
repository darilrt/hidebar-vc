//! Hyprland event source: queries `.socket.sock` and listens on `.socket2.sock`.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::event::{Event, Workspaces};

/// `.socket2.sock` events that may change the workspaces or the active one.
const RELEVANT: [&str; 5] = [
    "workspacev2",
    "focusedmonv2",
    "createworkspacev2",
    "destroyworkspacev2",
    "moveworkspacev2",
];
const RETRY: Duration = Duration::from_secs(2);

#[derive(Deserialize)]
struct Workspace {
    id: i32,
}

/// Listens to Hyprland on its own thread and emits the workspace state: an initial one and another after each
/// relevant event. `emit` returns `false` once the receiver is gone. Does nothing without Hyprland.
pub fn listen(emit: impl Fn(Event) -> bool + Send + 'static) {
    let Some(dir) = socket_dir() else {
        eprintln!("hidebar: HYPRLAND_INSTANCE_SIGNATURE is not set; no Hyprland events");
        return;
    };
    std::thread::spawn(move || {
        loop {
            match run(&dir, &emit) {
                Ok(Stop::ReceiverGone) => return,
                Ok(Stop::Disconnected) => {}
                Err(error) => eprintln!("hidebar: Hyprland connection: {error}"),
            }
            std::thread::sleep(RETRY);
        }
    });
}

enum Stop {
    ReceiverGone,
    Disconnected,
}

fn run(dir: &Path, emit: &impl Fn(Event) -> bool) -> io::Result<Stop> {
    let events = UnixStream::connect(dir.join(".socket2.sock"))?;
    if !emit(Event::Workspaces(workspaces(dir)?)) {
        return Ok(Stop::ReceiverGone);
    }
    for line in BufReader::new(events).lines() {
        if is_relevant(&line?) && !emit(Event::Workspaces(workspaces(dir)?)) {
            return Ok(Stop::ReceiverGone);
        }
    }
    Ok(Stop::Disconnected)
}

fn socket_dir() -> Option<PathBuf> {
    let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(PathBuf::from(runtime).join("hypr").join(signature))
}

fn is_relevant(line: &str) -> bool {
    line.split_once(">>")
        .is_some_and(|(name, _)| RELEVANT.contains(&name))
}

fn workspaces(dir: &Path) -> io::Result<Workspaces> {
    let all = request(dir, "j/workspaces")?;
    let active = request(dir, "j/activeworkspace")?;
    parse_workspaces(&all, &active).map_err(io::Error::other)
}

fn request(dir: &Path, command: &str) -> io::Result<String> {
    let mut stream = UnixStream::connect(dir.join(".socket.sock"))?;
    stream.write_all(command.as_bytes())?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply)
}

/// Reduces Hyprland's JSON replies to what matters. Drops special workspaces (id < 1).
fn parse_workspaces(all: &str, active: &str) -> serde_json::Result<Workspaces> {
    let mut ids: Vec<i32> = serde_json::from_str::<Vec<Workspace>>(all)?
        .into_iter()
        .map(|workspace| workspace.id)
        .filter(|id| *id > 0)
        .collect();
    ids.sort_unstable();
    let active = serde_json::from_str::<Workspace>(active)?.id;
    Ok(Workspaces { ids, active })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_sorts_ignoring_special() {
        let all =
            r#"[{"id": 3, "name": "3"}, {"id": -98, "name": "special:x"}, {"id": 1, "name": "1"}]"#;
        let active = r#"{"id": 3, "name": "3", "windows": 2}"#;
        assert_eq!(
            parse_workspaces(all, active).unwrap(),
            Workspaces {
                ids: vec![1, 3],
                active: 3
            }
        );
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(parse_workspaces("ok", "{}").is_err());
    }

    /// `cargo test -- --ignored`: queries the real Hyprland session (read-only).
    #[test]
    #[ignore = "requires a Hyprland session"]
    fn queries_live_hyprland() {
        let dir = socket_dir().expect("HYPRLAND_INSTANCE_SIGNATURE");
        let workspaces = workspaces(&dir).unwrap();
        println!("{workspaces:?}");
        assert!(!workspaces.ids.is_empty());
        assert!(workspaces.ids.contains(&workspaces.active));
    }

    #[test]
    fn filters_events() {
        assert!(is_relevant("workspacev2>>2,2"));
        assert!(is_relevant("focusedmonv2>>eDP-1,1"));
        assert!(!is_relevant("workspace>>2"));
        assert!(!is_relevant("activewindowv2>>55fd"));
        assert!(!is_relevant("no separator"));
    }
}
