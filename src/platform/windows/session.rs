//! Session actions of the status panel (ADR-0005): log out and power off through `shutdown.exe`.

use std::os::windows::process::CommandExt;
use std::process::Command;

use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use super::logging;
use crate::state::Action;

/// Program and arguments for `action`.
fn command(action: Action) -> (&'static str, &'static [&'static str]) {
    match action {
        Action::Poweroff => ("shutdown", &["/s", "/t", "0"]),
        Action::Logout => ("shutdown", &["/l"]),
    }
}

/// Launches the action without a shell or console window and without waiting for it;
/// failures go to the log.
pub(crate) fn run(action: Action) {
    let (program, args) = command(action);
    match Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    {
        // Reap it in the background so the handle is released if the session survives.
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(error) => logging::error(format!("hidebar: {program}: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands() {
        assert_eq!(
            command(Action::Poweroff),
            ("shutdown", &["/s", "/t", "0"][..])
        );
        assert_eq!(command(Action::Logout), ("shutdown", &["/l"][..]));
    }
}
