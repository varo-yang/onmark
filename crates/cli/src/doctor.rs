//! Read-only admission report for the local render toolchain.

use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use tokio::process::Command;
use tokio::time::timeout;

use crate::arguments::DoctorArgs;
use crate::environment::DoctorExecutables;
use crate::failure::CliError;

const REPORT_VERSION: u16 = 1;
const PROBE_DEADLINE: Duration = Duration::from_secs(10);
const CLEANUP_DEADLINE: Duration = Duration::from_secs(5);

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
    admit(&tools).await?;
    Ok(DoctorOutcome { tools, json })
}

async fn admit(tools: &DoctorExecutables) -> Result<(), DoctorError> {
    let mut browser = command(&tools.browser.path, "--version");
    let mut bundler = tools.bundler.command();
    bundler.arg("--help");
    let mut ffmpeg = command(&tools.ffmpeg, "-version");
    let mut ffprobe = command(&tools.ffprobe, "-version");

    tokio::try_join!(
        probe("browser", &tools.browser.path, &mut browser),
        probe(
            "presentation bundler",
            tools.bundler.executable(),
            &mut bundler,
        ),
        probe("FFmpeg", &tools.ffmpeg, &mut ffmpeg),
        probe("ffprobe", &tools.ffprobe, &mut ffprobe),
    )?;
    Ok(())
}

fn command(executable: &Path, argument: &'static str) -> Command {
    let mut command = Command::new(executable);
    command.arg(argument);
    command
}

async fn probe(role: &'static str, path: &Path, command: &mut Command) -> Result<(), DoctorError> {
    let mut child = command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| DoctorError::spawn(role, path, source))?;
    let status = match timeout(PROBE_DEADLINE, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(source)) => return Err(DoctorError::wait(role, path, source)),
        Err(_) => {
            terminate(role, path, &mut child).await?;
            return Err(DoctorError::new(role, path, DoctorErrorKind::Timeout));
        }
    };
    if !status.success() {
        return Err(DoctorError::new(
            role,
            path,
            DoctorErrorKind::Failed(status.to_string().into()),
        ));
    }
    Ok(())
}

async fn terminate(
    role: &'static str,
    path: &Path,
    child: &mut tokio::process::Child,
) -> Result<(), DoctorError> {
    child
        .start_kill()
        .map_err(|source| DoctorError::terminate(role, path, source))?;
    match timeout(CLEANUP_DEADLINE, child.wait()).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(source)) => Err(DoctorError::terminate(role, path, source)),
        Err(_) => Err(DoctorError::new(
            role,
            path,
            DoctorErrorKind::CleanupTimeout,
        )),
    }
}

#[derive(Debug)]
pub(super) struct DoctorError {
    role: &'static str,
    path: PathBuf,
    kind: DoctorErrorKind,
}

#[derive(Debug)]
enum DoctorErrorKind {
    Spawn(io::Error),
    Wait(io::Error),
    Terminate(io::Error),
    Timeout,
    CleanupTimeout,
    Failed(Box<str>),
}

impl DoctorError {
    fn new(role: &'static str, path: &Path, kind: DoctorErrorKind) -> Self {
        Self {
            role,
            path: path.to_owned(),
            kind,
        }
    }

    fn spawn(role: &'static str, path: &Path, source: io::Error) -> Self {
        Self::new(role, path, DoctorErrorKind::Spawn(source))
    }

    fn wait(role: &'static str, path: &Path, source: io::Error) -> Self {
        Self::new(role, path, DoctorErrorKind::Wait(source))
    }

    fn terminate(role: &'static str, path: &Path, source: io::Error) -> Self {
        Self::new(role, path, DoctorErrorKind::Terminate(source))
    }
}

impl fmt::Display for DoctorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} tool {} ", self.role, self.path.display())?;
        match &self.kind {
            DoctorErrorKind::Spawn(_) => formatter.write_str("could not be started"),
            DoctorErrorKind::Wait(_) => formatter.write_str("could not be observed"),
            DoctorErrorKind::Terminate(_) => formatter.write_str("could not be terminated"),
            DoctorErrorKind::Timeout => {
                formatter.write_str("missed its ten-second admission deadline")
            }
            DoctorErrorKind::CleanupTimeout => {
                formatter.write_str("missed its five-second cleanup deadline")
            }
            DoctorErrorKind::Failed(status) => write!(formatter, "exited with {status}"),
        }
    }
}

impl Error for DoctorError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            DoctorErrorKind::Spawn(source)
            | DoctorErrorKind::Wait(source)
            | DoctorErrorKind::Terminate(source) => Some(source),
            DoctorErrorKind::Timeout
            | DoctorErrorKind::CleanupTimeout
            | DoctorErrorKind::Failed(_) => None,
        }
    }
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

#[cfg(test)]
mod tests {
    use std::process::Stdio;

    use tokio::process::Command;

    use super::{DoctorErrorKind, probe};

    #[tokio::test]
    async fn admits_one_real_bounded_process_handshake() {
        let executable = std::env::current_exe().expect("the test executable has a path");
        let mut command = Command::new(&executable);
        command
            .arg("--list")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        probe("fixture", &executable, &mut command)
            .await
            .expect("the test harness responds successfully");
    }

    #[tokio::test]
    async fn rejects_a_responsive_process_with_the_wrong_contract() {
        let executable = std::env::current_exe().expect("the test executable has a path");
        let mut command = Command::new(&executable);
        command.arg("--not-an-onmark-test-option");

        let error = probe("fixture", &executable, &mut command)
            .await
            .expect_err("an unsuccessful handshake must be rejected");

        assert!(matches!(error.kind, DoctorErrorKind::Failed(_)));
    }
}
