//! Remote worker command: execute solved visual facts without source compilation.
//!
//! The command accepts the same serialized worker request as deployment and
//! deliberately bypasses source parsing, asset probing, and graph planning.

use std::fs;
use std::io::{self, Write as _};
use std::path::Path;
use std::process::ExitCode;

use onmark_render::{
    BrowserCaptureMode, BrowserLaunchPolicy, Ffmpeg, FrameArtifact, FrameCaptureExecutor,
    WorkerCaptureRequest,
};
use serde::Serialize;

use crate::arguments::{WorkerArgs, WorkerCaptureArgs, WorkerCommand};
use crate::environment;
use crate::execution;
use crate::failure::CliError;
use crate::input;

pub(super) struct WorkerOutcome {
    artifact: FrameArtifact,
    json: bool,
}

impl WorkerOutcome {
    pub(super) fn write(self) -> ExitCode {
        if self.json {
            return self.write_json();
        }
        let mut stdout = io::stdout().lock();
        writeln!(
            stdout,
            "Worker artifact ready: {} frames at {}",
            self.artifact.frames(),
            self.artifact.path().display(),
        )
        .map_or(ExitCode::FAILURE, |()| ExitCode::SUCCESS)
    }

    fn write_json(&self) -> ExitCode {
        let report = WorkerReport {
            version: 1,
            command: "worker.capture",
            artifact: self.artifact.path().display().to_string(),
            frames: self.artifact.frames(),
        };
        let mut stdout = io::stdout().lock();
        let result = serde_json::to_writer_pretty(&mut stdout, &report)
            .and_then(|()| writeln!(stdout).map_err(serde_json::Error::io));
        result.map_or(ExitCode::FAILURE, |()| ExitCode::SUCCESS)
    }
}

pub(super) async fn run(args: WorkerArgs, json: bool) -> Result<WorkerOutcome, CliError> {
    match args.command {
        WorkerCommand::Capture(args) => capture(args, json).await,
    }
}

async fn capture(args: WorkerCaptureArgs, json: bool) -> Result<WorkerOutcome, CliError> {
    let browser = environment::worker_browser(&args.browser)?;
    create_output_directory(&args.output)?;
    let request = read_request(&args.input)?;
    let capture_environment = request.capture_environment();
    let input = args.input.clone();
    let unit = tokio::task::spawn_blocking(move || {
        request.materialize(&input, execution::unit_root_limits())
    })
    .await
    .map_err(CliError::WorkerTask)??;
    let capture = FrameCaptureExecutor::new(
        browser,
        BrowserLaunchPolicy::local(),
        BrowserCaptureMode::BeginFrame,
        execution::browser_limits(),
        Ffmpeg::new(
            args.ffmpeg,
            execution::worker_encode_limits(),
            onmark_render::EncodeProfile::H264Mp4,
        )?,
    );
    let artifact = capture
        .capture_frame_artifact(
            &unit,
            capture_environment,
            &args.output,
            execution::frame_artifact_limits(),
        )
        .await?;

    Ok(WorkerOutcome { artifact, json })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkerReport {
    version: u16,
    command: &'static str,
    artifact: String,
    frames: u64,
}

fn read_request(input: &Path) -> Result<WorkerCaptureRequest, CliError> {
    let path = input.join(WorkerCaptureRequest::FILE_NAME);
    let source = input::read_utf8(&path, WorkerCaptureRequest::MAX_JSON_BYTES)
        .map_err(|source| CliError::read_worker_request(&path, source))?;
    serde_json::from_str(&source).map_err(|source| CliError::parse_worker_request(&path, source))
}

fn create_output_directory(output: &Path) -> Result<(), CliError> {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| CliError::create_output_directory(parent, source))
}
