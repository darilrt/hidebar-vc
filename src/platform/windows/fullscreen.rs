//! Detects full-screen games/presentations to hide the overlay.

use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::UI::Shell::{
    QUERY_USER_NOTIFICATION_STATE, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
    SHQueryUserNotificationState,
};

use super::win32::{SendHwnd, post_event};

const POLL: Duration = Duration::from_secs(5);

pub(crate) fn listen(sender: Sender<bool>, hwnd: SendHwnd) {
    thread::spawn(move || {
        let mut last = false;

        loop {
            let active = query();
            if active != last {
                last = active;
                if sender.send(active).is_err() {
                    return;
                }
                post_event(hwnd);
            }

            thread::sleep(POLL);
        }
    });
}

fn query() -> bool {
    // SAFETY: valid out param; the HRESULT is checked before reading `state`.
    let mut state: QUERY_USER_NOTIFICATION_STATE = 0;
    let result = unsafe { SHQueryUserNotificationState(&mut state) };
    result >= 0 && hides(state)
}

fn hides(state: QUERY_USER_NOTIFICATION_STATE) -> bool {
    matches!(
        state,
        QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Shell::{
        QUNS_ACCEPTS_NOTIFICATIONS, QUNS_APP, QUNS_NOT_PRESENT, QUNS_QUIET_TIME,
    };

    #[test]
    fn hides_only_for_busy_fullscreen_or_presentation() {
        assert!(hides(QUNS_BUSY));
        assert!(hides(QUNS_RUNNING_D3D_FULL_SCREEN));
        assert!(hides(QUNS_PRESENTATION_MODE));
        assert!(!hides(QUNS_NOT_PRESENT));
        assert!(!hides(QUNS_ACCEPTS_NOTIFICATIONS));
        assert!(!hides(QUNS_QUIET_TIME));
        assert!(!hides(QUNS_APP));
    }
}
