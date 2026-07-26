//! Stable identity report for the installed CLI and its host target.

use std::io::{self, Write};
use std::process::ExitCode;

use serde::Serialize;

const REPORT_VERSION: u16 = 1;

pub(super) struct InfoOutcome {
    json: bool,
}

impl InfoOutcome {
    pub(super) fn write(self) -> ExitCode {
        let result = if self.json {
            write_json()
        } else {
            write_human()
        };
        result.map_or(ExitCode::FAILURE, |()| ExitCode::SUCCESS)
    }
}

pub(super) const fn run(json: bool) -> InfoOutcome {
    InfoOutcome { json }
}

fn write_human() -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "Onmark {}", env!("CARGO_PKG_VERSION"))?;
    writeln!(
        stdout,
        "Host: {}-{}",
        std::env::consts::ARCH,
        std::env::consts::OS
    )
}

fn write_json() -> io::Result<()> {
    let report = InfoReport {
        version: REPORT_VERSION,
        command: "info",
        onmark_version: env!("CARGO_PKG_VERSION"),
        architecture: std::env::consts::ARCH,
        operating_system: std::env::consts::OS,
    };
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &report)?;
    writeln!(stdout)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InfoReport {
    version: u16,
    command: &'static str,
    onmark_version: &'static str,
    architecture: &'static str,
    operating_system: &'static str,
}
