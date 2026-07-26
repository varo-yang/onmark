//! Read-only admission report for the local render toolchain.

use std::io::{self, Write};
use std::process::ExitCode;

use serde::Serialize;

use crate::arguments::DoctorArgs;
use crate::environment::DoctorExecutables;
use crate::failure::CliError;

const REPORT_VERSION: u16 = 1;

pub(super) struct DoctorOutcome {
    tools: DoctorExecutables,
    json: bool,
}

impl DoctorOutcome {
    pub(super) fn write(self) -> ExitCode {
        let result = if self.json {
            write_json(&self.tools)
        } else {
            write_human(&self.tools)
        };
        result.map_or(ExitCode::FAILURE, |()| ExitCode::SUCCESS)
    }
}

pub(super) async fn run(args: DoctorArgs, json: bool) -> Result<DoctorOutcome, CliError> {
    let tools = DoctorExecutables::discover(&args).await?;
    Ok(DoctorOutcome { tools, json })
}

fn write_human(tools: &DoctorExecutables) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(
        stdout,
        "Browser: {} ({})",
        tools.browser.path.display(),
        tools.browser.capture_mode,
    )?;
    writeln!(stdout, "Bundler: {}", tools.bundler.executable().display())?;
    writeln!(stdout, "FFmpeg: {}", tools.ffmpeg.display())?;
    writeln!(stdout, "ffprobe: {}", tools.ffprobe.display())?;
    writeln!(stdout, "Toolchain is admitted for local rendering")
}

fn write_json(tools: &DoctorExecutables) -> io::Result<()> {
    let report = DoctorReport {
        version: REPORT_VERSION,
        command: "doctor",
        admitted: true,
        browser: ToolReport {
            path: tools.browser.path.display().to_string(),
        },
        capture_mode: tools.browser.capture_mode.to_string(),
        bundler: ToolReport {
            path: tools.bundler.executable().display().to_string(),
        },
        ffmpeg: ToolReport {
            path: tools.ffmpeg.display().to_string(),
        },
        ffprobe: ToolReport {
            path: tools.ffprobe.display().to_string(),
        },
    };
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &report)?;
    writeln!(stdout)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DoctorReport {
    version: u16,
    command: &'static str,
    admitted: bool,
    browser: ToolReport,
    capture_mode: String,
    bundler: ToolReport,
    ffmpeg: ToolReport,
    ffprobe: ToolReport,
}

#[derive(Serialize)]
struct ToolReport {
    path: String,
}
