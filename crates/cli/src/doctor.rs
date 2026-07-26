//! Read-only admission report for the local render toolchain.

use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::time::timeout;

use crate::arguments::DoctorArgs;
use crate::environment::DoctorExecutables;
use crate::failure::CliError;

const REPORT_VERSION: u16 = 1;
const PROBE_DEADLINE: Duration = Duration::from_secs(10);
const CLEANUP_DEADLINE: Duration = Duration::from_secs(5);
const MAX_PROBE_OUTPUT_BYTES: u64 = 64 * 1024;

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
        probe(
            "browser",
            &tools.browser.path,
            ProbeContract::Browser,
            &mut browser,
        ),
        probe(
            "presentation bundler",
            tools.bundler.executable(),
            ProbeContract::Bundler,
            &mut bundler,
        ),
        probe("FFmpeg", &tools.ffmpeg, ProbeContract::Ffmpeg, &mut ffmpeg),
        probe(
            "ffprobe",
            &tools.ffprobe,
            ProbeContract::Ffprobe,
            &mut ffprobe,
        ),
    )?;
    Ok(())
}

fn command(executable: &Path, argument: &'static str) -> Command {
    let mut command = Command::new(executable);
    command.arg(argument);
    command
}

async fn probe(
    role: &'static str,
    path: &Path,
    contract: ProbeContract,
    command: &mut Command,
) -> Result<(), DoctorError> {
    let mut child = command
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| DoctorError::spawn(role, path, source))?;
    let stdout = child
        .stdout
        .take()
        .expect("stdout is piped immediately before admission");
    let stderr = child
        .stderr
        .take()
        .expect("stderr is piped immediately before admission");
    let observation = observe(&mut child, stdout, stderr);
    let observation = match timeout(PROBE_DEADLINE, observation).await {
        Ok(Ok(observation)) => observation,
        Ok(Err(ProbeObservationError::OutputLimit)) => {
            terminate(role, path, &mut child).await?;
            return Err(DoctorError::new(role, path, DoctorErrorKind::OutputLimit));
        }
        Ok(Err(ProbeObservationError::Wait(source))) => {
            return Err(DoctorError::wait(role, path, source));
        }
        Ok(Err(ProbeObservationError::Read(source))) => {
            return Err(DoctorError::read(role, path, source));
        }
        Err(_) => {
            terminate(role, path, &mut child).await?;
            return Err(DoctorError::new(role, path, DoctorErrorKind::Timeout));
        }
    };
    if !observation.status.success() {
        return Err(DoctorError::new(
            role,
            path,
            DoctorErrorKind::Failed(observation.status.to_string().into()),
        ));
    }
    if !contract.accepts(&observation.stdout, &observation.stderr) {
        return Err(DoctorError::new(role, path, DoctorErrorKind::WrongContract));
    }
    Ok(())
}

async fn observe(
    child: &mut tokio::process::Child,
    stdout: impl AsyncRead + Unpin,
    stderr: impl AsyncRead + Unpin,
) -> Result<ProbeObservation, ProbeObservationError> {
    let (status, stdout, stderr) = tokio::try_join!(
        async { child.wait().await.map_err(ProbeObservationError::Wait) },
        capture(stdout),
        capture(stderr),
    )?;
    Ok(ProbeObservation {
        status,
        stdout,
        stderr,
    })
}

async fn capture(pipe: impl AsyncRead + Unpin) -> Result<Vec<u8>, ProbeObservationError> {
    let mut bytes = Vec::new();
    pipe.take(MAX_PROBE_OUTPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(ProbeObservationError::Read)?;
    if u64::try_from(bytes.len()).expect("a probe buffer length fits in u64")
        > MAX_PROBE_OUTPUT_BYTES
    {
        return Err(ProbeObservationError::OutputLimit);
    }
    Ok(bytes)
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
    Read(io::Error),
    Terminate(io::Error),
    Timeout,
    CleanupTimeout,
    OutputLimit,
    WrongContract,
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

    fn read(role: &'static str, path: &Path, source: io::Error) -> Self {
        Self::new(role, path, DoctorErrorKind::Read(source))
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
            DoctorErrorKind::Read(_) => formatter.write_str("output could not be read"),
            DoctorErrorKind::Terminate(_) => formatter.write_str("could not be terminated"),
            DoctorErrorKind::Timeout => {
                formatter.write_str("missed its ten-second admission deadline")
            }
            DoctorErrorKind::CleanupTimeout => {
                formatter.write_str("missed its five-second cleanup deadline")
            }
            DoctorErrorKind::OutputLimit => {
                formatter.write_str("exceeded its 64 KiB admission-output limit")
            }
            DoctorErrorKind::WrongContract => {
                formatter.write_str("did not identify itself as the required tool")
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
            | DoctorErrorKind::Read(source)
            | DoctorErrorKind::Terminate(source) => Some(source),
            DoctorErrorKind::Timeout
            | DoctorErrorKind::CleanupTimeout
            | DoctorErrorKind::OutputLimit
            | DoctorErrorKind::WrongContract
            | DoctorErrorKind::Failed(_) => None,
        }
    }
}

struct ProbeObservation {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

enum ProbeObservationError {
    Wait(io::Error),
    Read(io::Error),
    OutputLimit,
}

#[derive(Clone, Copy)]
enum ProbeContract {
    Browser,
    Bundler,
    Ffmpeg,
    Ffprobe,
}

impl ProbeContract {
    fn accepts(self, stdout: &[u8], stderr: &[u8]) -> bool {
        let signature = match self {
            Self::Browser => "chrome",
            Self::Bundler => "Usage: onmark-bundle",
            Self::Ffmpeg => "ffmpeg version ",
            Self::Ffprobe => "ffprobe version ",
        };
        contains_ascii_case_insensitive(stdout, signature)
            || contains_ascii_case_insensitive(stderr, signature)
    }
}

fn contains_ascii_case_insensitive(output: &[u8], signature: &str) -> bool {
    output
        .windows(signature.len())
        .any(|candidate| candidate.eq_ignore_ascii_case(signature.as_bytes()))
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

    use super::{
        DoctorErrorKind, MAX_PROBE_OUTPUT_BYTES, ProbeContract, ProbeObservationError, capture,
        probe,
    };

    #[tokio::test]
    async fn rejects_a_successful_process_with_unrelated_output() {
        let executable = std::env::current_exe().expect("the test executable has a path");
        let mut command = Command::new(&executable);
        command
            .arg("--list")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        probe("fixture", &executable, ProbeContract::Bundler, &mut command)
            .await
            .expect_err("a successful process with unrelated output is not a bundler");
    }

    #[tokio::test]
    async fn rejects_a_responsive_process_with_the_wrong_contract() {
        let executable = std::env::current_exe().expect("the test executable has a path");
        let mut command = Command::new(&executable);
        command.arg("--not-an-onmark-test-option");

        let error = probe("fixture", &executable, ProbeContract::Bundler, &mut command)
            .await
            .expect_err("an unsuccessful handshake must be rejected");

        assert!(matches!(error.kind, DoctorErrorKind::Failed(_)));
    }

    #[test]
    fn admits_only_role_specific_signatures() {
        assert!(ProbeContract::Browser.accepts(b"Google Chrome 150", b""));
        assert!(ProbeContract::Bundler.accepts(b"Usage: onmark-bundle", b""));
        assert!(ProbeContract::Ffmpeg.accepts(b"", b"ffmpeg version 8.1"));
        assert!(ProbeContract::Ffprobe.accepts(b"ffprobe version 8.1", b""));
        assert!(!ProbeContract::Ffmpeg.accepts(b"ffprobe version 8.1", b""));
        assert!(!ProbeContract::Bundler.accepts(b"", b""));
    }

    #[tokio::test]
    async fn bounds_each_admission_pipe() {
        let output = vec![
            0_u8;
            usize::try_from(MAX_PROBE_OUTPUT_BYTES + 1)
                .expect("the admission limit fits in memory")
        ];

        let error = capture(output.as_slice())
            .await
            .expect_err("one byte beyond the fixed limit is rejected");

        assert!(matches!(error, ProbeObservationError::OutputLimit));
    }
}
