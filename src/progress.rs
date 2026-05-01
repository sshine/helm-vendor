//! TTY-aware progress reporter.
//!
//! On a terminal, prints a single `[X/Y] label` line on stderr that overwrites
//! itself in place. On a non-terminal, all methods are no-ops — callers are
//! expected to stream their own per-row output instead.

use std::io::{IsTerminal, Write};

pub struct Progress {
    interactive: bool,
    total: usize,
    current: usize,
}

impl Progress {
    pub fn new(total: usize) -> Self {
        // Interactive iff both stdout and stderr are TTYs. Piping stdout
        // (e.g. `helm-vendor list --check | tee out`) flips us to streaming
        // mode so rows arrive as they resolve instead of in one batch.
        Self {
            interactive: std::io::stdout().is_terminal() && std::io::stderr().is_terminal(),
            total,
            current: 0,
        }
    }

    pub fn interactive(&self) -> bool {
        self.interactive
    }

    pub fn tick(&mut self, label: &str) {
        self.current += 1;
        if self.interactive {
            eprint!("\r\x1b[2K[{}/{}] {}", self.current, self.total, label);
            let _ = std::io::stderr().flush();
        }
    }

    pub fn clear(&self) {
        if self.interactive {
            eprint!("\r\x1b[2K");
            let _ = std::io::stderr().flush();
        }
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        self.clear();
    }
}
