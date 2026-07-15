//! Gate-three worker command: execute solved visual facts without source compilation.

use std::fs;
use std::io::{self, Write as _};
use std::path::Path;
use std::process::ExitCode;

use onmark_render::{ChromiumSandbox, FrameArtifact, FrameCaptureExecutor, WorkerCaptureRequest};

use crate::arguments::{WorkerArgs, WorkerCaptureArgs, WorkerCommand};
use crate::environment;
use crate::execution;
use crate::failure::CliError;

pub(super) struct WorkerOutcome {
    artifact: FrameArtifact,
}

impl WorkerOutcome {
    pub(super) fn write(self) -> ExitCode {
        let mut stdout = io::stdout().lock();
        writeln!(
            stdout,
            "Worker artifact ready: {} frames at {}",
            self.artifact.frames(),
            self.artifact.path().display(),
        )
        .map_or(ExitCode::FAILURE, |()| ExitCode::SUCCESS)
    }
}

pub(super) async fn run(args: WorkerArgs) -> Result<WorkerOutcome, CliError> {
    match args.command {
        WorkerCommand::Capture(args) => capture(args).await,
    }
}

async fn capture(args: WorkerCaptureArgs) -> Result<WorkerOutcome, CliError> {
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
        ChromiumSandbox::Enabled,
        execution::browser_limits(),
    );
    let artifact = capture
        .capture_frame_artifact(
            &unit,
            capture_environment,
            &args.output,
            execution::frame_artifact_limits(),
        )
        .await?;

    Ok(WorkerOutcome { artifact })
}

fn read_request(input: &Path) -> Result<WorkerCaptureRequest, CliError> {
    let path = input.join(WorkerCaptureRequest::FILE_NAME);
    let source =
        fs::read_to_string(&path).map_err(|source| CliError::read_worker_request(&path, source))?;
    serde_json::from_str(&source).map_err(|source| CliError::parse_worker_request(&path, source))
}

fn create_output_directory(output: &Path) -> Result<(), CliError> {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| CliError::create_output_directory(parent, source))
}
