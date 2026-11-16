//! Niri event source: talks directly to the IPC socket (`$NIRI_SOCKET`).

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::event::{Event, Workspaces};

const RETRY: Duration = Duration::from_secs(2);

#[derive(Deserialize)]
struct NiriWorkspace {
    id: u64,
    idx: u8,
    name: Option<String>,
    output: Option<String>,
    is_focused: bool,
    active_window_id: Option<u64>,
}

#[derive(Deserialize)]
struct WorkspacesChanged {
    workspaces: Vec<NiriWorkspace>,
}

#[derive(Deserialize)]
struct WorkspaceActivated {
    id: u64,
    focused: bool,
}

#[derive(Deserialize)]
struct WorkspaceActiveWindowChanged {
    workspace_id: u64,
    active_window_id: Option<u64>,
}

#[derive(Deserialize)]
struct NiriEvent {
    #[serde(rename = "WorkspacesChanged")]
    workspaces_changed: Option<WorkspacesChanged>,
    #[serde(rename = "WorkspaceActivated")]
    workspace_activated: Option<WorkspaceActivated>,
    #[serde(rename = "WorkspaceActiveWindowChanged")]
    workspace_active_window_changed: Option<WorkspaceActiveWindowChanged>,
}

/// Listens to Niri on its own thread and emits the workspace state. Does nothing without `NIRI_SOCKET`.
pub fn listen(emit: impl Fn(Event) -> bool + Send + 'static) {
    let Some(path) = socket_path() else {
        eprintln!("hidebar: NIRI_SOCKET is not set; no Niri events");
        return;
    };
    std::thread::spawn(move || {
        loop {
            match run(&path, &emit) {
                Ok(Stop::ReceiverGone) => return,
                Ok(Stop::Disconnected) => {}
                Err(error) => eprintln!("hidebar: Niri connection: {error}"),
            }
            std::thread::sleep(RETRY);
        }
    });
}

enum Stop {
    ReceiverGone,
    Disconnected,
}

pub fn socket_path() -> Option<PathBuf> {
    std::env::var_os("NIRI_SOCKET").map(PathBuf::from)
}

/// Niri answers on `NIRI_SOCKET`. The file existing is not enough: an uncleanly closed session leaves the socket
/// behind and the variable may be inherited (systemd environment), which would then shadow Hyprland.
pub fn available() -> bool {
    socket_path().is_some_and(|path| UnixStream::connect(path).is_ok())
}

fn run(path: &Path, emit: &impl Fn(Event) -> bool) -> io::Result<Stop> {
    let mut stream = UnixStream::connect(path)?;
    stream.write_all(b"\"EventStream\"\n")?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    reader.read_line(&mut reply)?;
    // The first reply is {"Ok":"Handled"}; if niri rejects the request, the stream would never bring events
    // and this thread would wait forever, without retrying.
    if !reply.contains("\"Ok\"") {
        return Err(io::Error::other(format!(
            "Niri rejected the event stream: {}",
            reply.trim()
        )));
    }

    // The full state arrives on connect and is replaced on each `WorkspacesChanged`.
    let mut last: Option<Vec<NiriWorkspace>> = None;
    for line in reader.lines() {
        let Ok(event) = serde_json::from_str::<NiriEvent>(&line?) else {
            continue;
        };
        if let Some(changed) = event.workspaces_changed {
            last = Some(changed.workspaces);
        } else if let Some(activated) = event.workspace_activated.filter(|it| it.focused) {
            let Some(workspaces) = &mut last else {
                continue;
            };
            focus(workspaces, activated.id);
        } else if let Some(changed) = event.workspace_active_window_changed {
            // Opening or closing windows does not always resend the list: it decides which workspaces are empty.
            let Some(workspaces) = &mut last else {
                continue;
            };
            set_active_window(workspaces, changed.workspace_id, changed.active_window_id);
        } else {
            continue;
        }
        if let Some(workspaces) = last.as_deref().and_then(workspaces_from_list)
            && !emit(Event::Workspaces(workspaces))
        {
            return Ok(Stop::ReceiverGone);
        }
    }
    Ok(Stop::Disconnected)
}

/// Moves focus within the cached list: niri does not resend it when a workspace is activated.
fn focus(workspaces: &mut [NiriWorkspace], id: u64) {
    for workspace in workspaces {
        workspace.is_focused = workspace.id == id;
    }
}

fn set_active_window(workspaces: &mut [NiriWorkspace], id: u64, window: Option<u64>) {
    if let Some(workspace) = workspaces.iter_mut().find(|workspace| workspace.id == id) {
        workspace.active_window_id = window;
    }
}

/// Reduces the workspace list to what matters: those on the focused output, sorted by index.
/// Niri always keeps an empty workspace at the end of each output: unnamed empty ones are skipped except the
/// focused one, as in Hyprland, where an empty workspace stops existing.
/// `None` while no workspace is focused (transient state): there is nothing to show.
fn workspaces_from_list(workspaces: &[NiriWorkspace]) -> Option<Workspaces> {
    let focused = workspaces.iter().find(|workspace| workspace.is_focused)?;
    let mut ids: Vec<i32> = workspaces
        .iter()
        .filter(|workspace| workspace.output == focused.output)
        .filter(|workspace| {
            workspace.is_focused || workspace.active_window_id.is_some() || workspace.name.is_some()
        })
        .map(|workspace| i32::from(workspace.idx))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    Some(Workspaces {
        ids,
        active: i32::from(focused.idx),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reply to `"Workspaces"`: `{"Ok":{"Workspaces":[…]}}`.
    #[derive(Deserialize)]
    struct WorkspacesReply {
        #[serde(rename = "Ok")]
        ok: ReplyBody,
    }

    #[derive(Deserialize)]
    struct ReplyBody {
        #[serde(rename = "Workspaces")]
        workspaces: Vec<NiriWorkspace>,
    }

    fn parse_workspaces_event(line: &str) -> Option<Workspaces> {
        let event: NiriEvent = serde_json::from_str(line).ok()?;
        workspaces_from_list(&event.workspaces_changed?.workspaces)
    }

    fn workspace(id: u64, idx: u8, output: &str, is_focused: bool) -> NiriWorkspace {
        NiriWorkspace {
            id,
            idx,
            name: None,
            output: Some(output.to_owned()),
            is_focused,
            active_window_id: Some(id * 10),
        }
    }

    #[test]
    fn parses_event_line() {
        let line = r#"{"WorkspacesChanged":{"workspaces":[{"id":3,"idx":3,"name":null,"output":"HDMI-A-1","is_urgent":false,"is_active":false,"is_focused":false,"active_window_id":12},{"id":1,"idx":1,"name":"main","output":"HDMI-A-1","is_urgent":false,"is_active":true,"is_focused":true,"active_window_id":11}]}}"#;
        assert_eq!(
            parse_workspaces_event(line).unwrap(),
            Workspaces {
                ids: vec![1, 3],
                active: 1
            }
        );
    }

    #[test]
    fn keeps_only_the_focused_output() {
        let line = r#"{"WorkspacesChanged":{"workspaces":[{"id":1,"idx":1,"output":"HDMI-A-1","is_focused":true},{"id":9,"idx":1,"output":"DP-1","is_focused":false},{"id":10,"idx":2,"output":"DP-1","is_focused":false}]}}"#;
        assert_eq!(
            parse_workspaces_event(line).unwrap(),
            Workspaces {
                ids: vec![1],
                active: 1
            }
        );
    }

    #[test]
    fn waits_until_a_workspace_is_focused() {
        let line = r#"{"WorkspacesChanged":{"workspaces":[{"id":1,"idx":1,"output":"eDP-1","is_focused":false}]}}"#;
        assert!(parse_workspaces_event(line).is_none());
    }

    #[test]
    fn focus_moves_the_dot() {
        let mut workspaces = vec![
            workspace(1, 1, "HDMI-A-1", true),
            workspace(2, 2, "HDMI-A-1", false),
        ];
        focus(&mut workspaces, 2);
        assert_eq!(
            workspaces_from_list(&workspaces).unwrap(),
            Workspaces {
                ids: vec![1, 2],
                active: 2
            }
        );
    }

    #[test]
    fn hides_empty_unnamed_workspaces_except_the_focused_one() {
        let line = r#"{"WorkspacesChanged":{"workspaces":[{"id":1,"idx":1,"name":null,"output":"eDP-1","is_focused":false,"active_window_id":5},{"id":2,"idx":2,"name":null,"output":"eDP-1","is_focused":true,"active_window_id":null},{"id":3,"idx":3,"name":"chat","output":"eDP-1","is_focused":false,"active_window_id":null},{"id":4,"idx":4,"name":null,"output":"eDP-1","is_focused":false,"active_window_id":null}]}}"#;
        assert_eq!(
            parse_workspaces_event(line).unwrap(),
            Workspaces {
                ids: vec![1, 2, 3],
                active: 2
            }
        );
    }

    #[test]
    fn active_window_changes_update_emptiness() {
        let mut workspaces = vec![
            workspace(1, 1, "eDP-1", true),
            workspace(2, 2, "eDP-1", false),
        ];
        set_active_window(&mut workspaces, 2, None);
        assert_eq!(workspaces_from_list(&workspaces).unwrap().ids, vec![1]);
        set_active_window(&mut workspaces, 2, Some(7));
        assert_eq!(workspaces_from_list(&workspaces).unwrap().ids, vec![1, 2]);
    }

    #[test]
    fn ignores_other_events() {
        let line = r#"{"WindowsChanged":{"windows":[]}}"#;
        assert!(parse_workspaces_event(line).is_none());
    }

    #[test]
    fn rejects_invalid_json() {
        assert!(parse_workspaces_event("not json").is_none());
    }

    /// `cargo test -- --ignored`: queries the real Niri session (read-only).
    #[test]
    #[ignore = "requires a Niri session"]
    fn queries_live_niri() {
        let path = socket_path().expect("NIRI_SOCKET");
        let mut stream = UnixStream::connect(path).unwrap();
        stream.write_all(b"\"Workspaces\"\n").unwrap();
        let mut reply = String::new();
        // Niri does not close the connection after replying: read one line, not until EOF.
        BufReader::new(stream).read_line(&mut reply).unwrap();
        let reply: WorkspacesReply = serde_json::from_str(&reply).unwrap();
        let workspaces = workspaces_from_list(&reply.ok.workspaces).unwrap();
        println!("{workspaces:?}");
        assert!(!workspaces.ids.is_empty());
        assert!(workspaces.ids.contains(&workspaces.active));
    }
}
