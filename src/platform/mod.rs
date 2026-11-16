//! Platform selection. `main` only knows `run_daemon` and `send`.

#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(windows)]
pub(crate) mod windows;

#[cfg(not(any(target_os = "linux", windows)))]
compile_error!("hidebar only supports Linux and Windows");

#[cfg(target_os = "linux")]
pub(crate) use linux::{install, run_daemon, send, uninstall};
#[cfg(windows)]
pub(crate) use windows::{install, run_daemon, send, uninstall};
