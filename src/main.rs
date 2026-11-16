#![deny(unsafe_op_in_unsafe_fn)]

mod calendar;
mod command;
mod event;
mod platform;
mod state;
mod ui;

use std::process::ExitCode;

use crate::command::Command;

const USAGE: &str = "\
usage:
  hidebar daemon            start the daemon
  hidebar show <module>     show for a few seconds
  hidebar toggle <module>   pin/unpin (clock: open/close the calendar)
  hidebar quit              stop the daemon
  hidebar install           start hidebar at login (Windows)
  hidebar uninstall         stop starting it at login (Windows)

modules: workspaces, clock, status (network, cpu, ram, mute, battery), all";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("daemon") if args.len() == 1 => run_daemon(),
        Some("install") if args.len() == 1 => platform::install(),
        Some("uninstall") if args.len() == 1 => platform::uninstall(),
        Some("-h" | "--help") | None => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(_) => match args.join(" ").parse::<Command>() {
            Ok(command) => run_client(command),
            Err(message) => {
                eprintln!("hidebar: {message}\n\n{USAGE}");
                ExitCode::from(2)
            }
        },
    }
}

fn run_daemon() -> ExitCode {
    platform::run_daemon()
}

fn run_client(command: Command) -> ExitCode {
    match platform::send(command) {
        Ok(reply) if reply == "ok" => ExitCode::SUCCESS,
        Ok(reply) => {
            eprintln!("hidebar: {reply}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("hidebar: {error}");
            ExitCode::FAILURE
        }
    }
}
