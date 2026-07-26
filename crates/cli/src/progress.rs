//! Human-only phase progress for operations that own long-running processes.

use std::fmt;
use std::io::{self, IsTerminal as _, Write};
use std::time::Duration;

use crate::failure::CliError;

/// One terminal progress sink disabled for redirected or machine output.
#[derive(Clone, Copy)]
pub(super) struct Progress {
    enabled: bool,
}

impl Progress {
    pub(super) fn for_command(json: bool) -> Self {
        Self {
            enabled: !json && io::stderr().is_terminal(),
        }
    }

    pub(super) fn started(self, phase: &'static str) -> Result<(), CliError> {
        if !self.enabled {
            return Ok(());
        }
        Self::write(format_args!("{phase}..."))
    }

    pub(super) fn completed(self, phase: &'static str, elapsed: Duration) -> Result<(), CliError> {
        if !self.enabled {
            return Ok(());
        }
        Self::write(format_args!(
            "{phase} completed in {} ms",
            elapsed.as_millis()
        ))
    }

    pub(super) fn sample(self, index: usize, total: usize) -> Result<(), CliError> {
        if !self.enabled {
            return Ok(());
        }
        Self::write(format_args!("benchmark sample {index}/{total}..."))
    }

    fn write(message: fmt::Arguments<'_>) -> Result<(), CliError> {
        writeln!(io::stderr().lock(), "{message}").map_err(CliError::write_progress)
    }
}
