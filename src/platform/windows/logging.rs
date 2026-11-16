//! Minimal log to `%LOCALAPPDATA%\hidebar\hidebar.log`, rotated at 1 MB.

use std::fmt::Display;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static LOG: OnceLock<Mutex<File>> = OnceLock::new();

pub(crate) fn init() {
    let path = path();

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 1_000_000) {
        let _ = std::fs::rename(&path, path.with_extension("old.log"));
    }

    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = LOG.set(Mutex::new(file));
    }
}

pub(crate) fn error(message: impl Display) {
    let line = format!(
        "[{}] {message}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );

    eprintln!("{line}");

    if let Some(log) = LOG.get()
        && let Ok(mut file) = log.lock()
    {
        let _ = writeln!(file, "{line}");
    }
}

fn path() -> PathBuf {
    match std::env::var_os("LOCALAPPDATA") {
        Some(directory) => PathBuf::from(directory).join("hidebar").join("hidebar.log"),
        None => std::env::temp_dir().join("hidebar.log"),
    }
}
