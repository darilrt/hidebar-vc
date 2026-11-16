//! Session actions of the status panel (ADR-0005): log out through the compositor, power off through systemd.

use std::process::Command;

use crate::state::Action;

/// Compositor the daemon is running under, detected at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Compositor {
    Hyprland,
    Niri,
    Unknown,
}

/// Program and arguments for `action`; `None` when it cannot be done here.
fn command(
    action: Action,
    compositor: Compositor,
) -> Option<(&'static str, &'static [&'static str])> {
    match (action, compositor) {
        (Action::Poweroff, _) => Some(("systemctl", &["poweroff"])),
        (Action::Logout, Compositor::Hyprland) => Some(("hyprctl", &["dispatch", "exit"])),
        (Action::Logout, Compositor::Niri) => {
            Some(("niri", &["msg", "action", "quit", "--skip-confirmation"]))
        }
        (Action::Logout, Compositor::Unknown) => None,
    }
}

/// Launches the action without a shell and without waiting for it; failures go to stderr.
pub(crate) fn run(action: Action, compositor: Compositor) {
    let Some((program, args)) = command(action, compositor) else {
        eprintln!("hidebar: cannot log out: compositor not detected");
        return;
    };
    match Command::new(program).args(args).spawn() {
        // Reap it in the background so it does not linger as a zombie if the session survives.
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(error) => eprintln!("hidebar: {program}: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_per_compositor() {
        assert_eq!(
            command(Action::Poweroff, Compositor::Unknown),
            Some(("systemctl", &["poweroff"][..]))
        );
        assert_eq!(
            command(Action::Logout, Compositor::Hyprland),
            Some(("hyprctl", &["dispatch", "exit"][..]))
        );
        assert_eq!(
            command(Action::Logout, Compositor::Niri),
            Some((
                "niri",
                &["msg", "action", "quit", "--skip-confirmation"][..]
            ))
        );
        assert_eq!(command(Action::Logout, Compositor::Unknown), None);
    }
}
