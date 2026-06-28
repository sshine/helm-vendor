//! ANSI color helpers for the LATEST column.
//!
//! Color is enabled iff stdout is a terminal and `NO_COLOR` is unset (the
//! [no-color.org](https://no-color.org/) convention). Severity is classified
//! by SemVer diff: patch=green, minor=yellow, major=orange.

use std::io::IsTerminal;

#[derive(Clone, Copy, Debug)]
pub enum Severity {
    Patch,
    Minor,
    Major,
}

pub fn enabled() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    std::io::stdout().is_terminal()
}

pub fn severity(pinned: &str, latest: &str) -> Severity {
    let p = pinned.trim_start_matches('v');
    let l = latest.trim_start_matches('v');
    match (semver::Version::parse(p), semver::Version::parse(l)) {
        (Ok(p), Ok(l)) => {
            if p.major != l.major {
                Severity::Major
            } else if p.minor != l.minor {
                Severity::Minor
            } else {
                Severity::Patch
            }
        }
        // If we can't parse, lean toward "minor" — neutral middle ground.
        _ => Severity::Minor,
    }
}

pub fn paint(s: &str, sev: Severity, on: bool) -> String {
    if !on {
        return s.to_string();
    }
    let code = match sev {
        Severity::Patch => "\x1b[32m",       // green
        Severity::Minor => "\x1b[33m",       // yellow
        Severity::Major => "\x1b[38;5;208m", // orange (256-color)
    };
    format!("{code}{s}\x1b[0m")
}

/// Paint `s` gray (bright-black) — used for cells we can't or shouldn't treat
/// as an upgrade: a failed/unknown upstream lookup, or an upstream version
/// that's *older* than the pin (source moved, catalog reshuffled).
pub fn gray(s: &str, on: bool) -> String {
    if !on {
        return s.to_string();
    }
    format!("\x1b[90m{s}\x1b[0m")
}
