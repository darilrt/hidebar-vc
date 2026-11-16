//! Autostart in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//! No admin and no system changes: a single per-user key.

use std::process::ExitCode;

use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteValueW, RegSetValueExW,
};
use windows::core::HSTRING;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "hidebar";

pub(crate) fn install() -> ExitCode {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            eprintln!("hidebar: could not resolve the executable: {error}");
            return ExitCode::FAILURE;
        }
    };

    let command = format!("\"{}\" daemon", exe.display());
    match write(&command) {
        Ok(()) => {
            println!("hidebar: installed to start at login ({command})");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("hidebar: could not install autostart: {error}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn uninstall() -> ExitCode {
    match remove() {
        Ok(()) => {
            println!("hidebar: autostart removed");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("hidebar: could not remove autostart: {error}");
            ExitCode::FAILURE
        }
    }
}

fn write(command: &str) -> windows::core::Result<()> {
    let key = open()?;
    let utf16: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `utf16` lives until `RegSetValueExW` returns; the size in bytes
    // is `utf16.len() * size_of::<u16>()`.
    let bytes = unsafe { std::slice::from_raw_parts(utf16.as_ptr().cast(), utf16.len() * 2) };

    // SAFETY: `key` is open; the name and data live during the call.
    let result =
        unsafe { RegSetValueExW(key, &HSTRING::from(VALUE_NAME), None, REG_SZ, Some(bytes)) };
    // SAFETY: `key` is valid and not used after closing it.
    unsafe {
        let _ = RegCloseKey(key);
    }
    result.ok()
}

fn remove() -> windows::core::Result<()> {
    let key = open()?;
    // SAFETY: `key` is open and the name lives during the call.
    let result = unsafe { RegDeleteValueW(key, &HSTRING::from(VALUE_NAME)) };
    // SAFETY: `key` is valid and not used afterwards.
    unsafe {
        let _ = RegCloseKey(key);
    }
    result.ok()
}

fn open() -> windows::core::Result<HKEY> {
    let mut key = HKEY::default();

    // SAFETY: `HKEY_CURRENT_USER` is valid; the name and class live during
    // the call; `key` is an out initialized by the API.
    let result = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    result.ok()?;

    Ok(key)
}
