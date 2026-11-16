//! Client → daemon communication over a Unix socket: one line of text per command.

use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use crate::command::Command;

/// `$HIDEBAR_SOCKET` if set (e.g. a development daemon next to the usual one);
/// otherwise `$XDG_RUNTIME_DIR/hidebar.sock`, or `/tmp/hidebar-$UID.sock`.
pub fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("HIDEBAR_SOCKET") {
        return PathBuf::from(path);
    }
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("hidebar.sock"),
        None => {
            // The owner of /proc/self is the process's user.
            let uid = std::fs::metadata("/proc/self").map_or(0, |meta| meta.uid());
            PathBuf::from(format!("/tmp/hidebar-{uid}.sock"))
        }
    }
}

/// Sends a command to the daemon and returns its reply.
pub fn send(command: Command) -> io::Result<String> {
    let path = socket_path();
    let mut stream = UnixStream::connect(&path).map_err(|error| {
        if matches!(
            error.kind(),
            ErrorKind::NotFound | ErrorKind::ConnectionRefused
        ) {
            io::Error::new(
                error.kind(),
                format!(
                    "the daemon is not running ({}); start it with `hidebar daemon`",
                    path.display()
                ),
            )
        } else {
            error
        }
    })?;
    writeln!(stream, "{command}")?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    Ok(reply.trim_end().to_owned())
}

/// Creates the daemon socket. Fails if another daemon is already using it;
/// removes a stale socket left by a previous run.
pub fn bind() -> io::Result<UnixListener> {
    let path = socket_path();
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            return Err(io::Error::new(
                ErrorKind::AddrInUse,
                format!("a daemon is already listening on {}", path.display()),
            ));
        }
        std::fs::remove_file(&path)?;
    }
    UnixListener::bind(&path)
}

/// Serves connections on its own thread and hands valid commands to `deliver`,
/// which returns `false` once the daemon can no longer receive them.
pub fn serve(listener: UnixListener, deliver: impl Fn(Command) -> bool + Send + 'static) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            if let Ok(false) = handle(stream, &deliver) {
                break;
            }
        }
    });
}

/// Returns whether the daemon still accepts commands.
fn handle(mut stream: UnixStream, deliver: &impl Fn(Command) -> bool) -> io::Result<bool> {
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let mut alive = true;
    let reply = match line.parse::<Command>() {
        Ok(command) => {
            alive = deliver(command);
            if alive {
                "ok".to_owned()
            } else {
                "error: the daemon is shutting down".to_owned()
            }
        }
        Err(message) => format!("error: {message}"),
    };
    writeln!(stream, "{reply}")?;
    Ok(alive)
}
