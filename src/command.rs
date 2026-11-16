//! Commands the daemon understands. Parsing and formatting, no transport.

use std::fmt;
use std::str::FromStr;

/// Commands the daemon understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Show for a few seconds.
    Show(Target),
    /// The module decides: pin/unpin, or open/close its panel (clock → calendar).
    Toggle(Target),
    /// Stop the daemon.
    Quit,
}

/// Strip modules, or all of them at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Workspaces,
    Clock,
    Status,
    All,
}

impl Target {
    pub(crate) const ALL: [Self; 4] = [Self::Workspaces, Self::Clock, Self::Status, Self::All];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Workspaces => "workspaces",
            Self::Clock => "clock",
            Self::Status => "status",
            Self::All => "all",
        }
    }
}

impl FromStr for Command {
    type Err = String;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.as_slice() == ["quit"] {
            return Ok(Self::Quit);
        }
        let verb: fn(Target) -> Self = match words.first() {
            Some(&"show") => Self::Show,
            Some(&"toggle") => Self::Toggle,
            None => return Err("empty command".into()),
            _ => return Err(format!("unknown command: {}", words.join(" "))),
        };
        match words.as_slice() {
            [_, name] => Target::ALL
                .into_iter()
                .find(|target| target.name() == *name)
                .map(verb)
                .ok_or_else(|| format!("unknown module: {name}")),
            _ => Err(format!("unknown command: {}", words.join(" "))),
        }
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Show(target) => write!(f, "show {}", target.name()),
            Self::Toggle(target) => write!(f, "toggle {}", target.name()),
            Self::Quit => f.write_str("quit"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_verbs_and_targets() {
        assert_eq!("show clock".parse(), Ok(Command::Show(Target::Clock)));
        assert_eq!(
            "  toggle   status \n".parse(),
            Ok(Command::Toggle(Target::Status))
        );
        assert_eq!("show all".parse(), Ok(Command::Show(Target::All)));
        assert_eq!("quit".parse(), Ok(Command::Quit));
    }

    #[test]
    fn rejects_unknown_and_empty() {
        assert!("show".parse::<Command>().is_err());
        assert!(
            "show battery".parse::<Command>().is_err(),
            "replaced by status"
        );
        assert!("hide clock".parse::<Command>().is_err());
        assert!("show workspaces now".parse::<Command>().is_err());
        assert!("quit now".parse::<Command>().is_err());
        assert!("\n".parse::<Command>().is_err());
    }

    #[test]
    fn display_round_trips() {
        for target in Target::ALL {
            for command in [Command::Show(target), Command::Toggle(target)] {
                assert_eq!(command.to_string().parse(), Ok(command));
            }
        }
        assert_eq!(Command::Quit.to_string().parse(), Ok(Command::Quit));
    }
}
