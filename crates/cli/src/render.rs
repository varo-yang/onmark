use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use onmark_core::diagnostics::Diagnostic;
use onmark_core::model::Timebase;
use onmark_core::timeline::TimelineIr;
use onmark_media::Ffprobe;
use onmark_render::{
    BrowserLimits, EncodeLimits, EncodedVideo, ExecutableUnit, Ffmpeg, RenderExecutor,
    RenderProfile, RenderUnit, UnitRootLimits,
};

use crate::arguments::{RenderArgs, source_directory};
use crate::assets::FrozenCatalog;
use crate::bundler::{BundleArtifact, PresentationBundler};
use crate::compilation;
use crate::diagnostic;
use crate::environment::Executables;
use crate::failure::CliError;

const PROCESS_DEADLINE: Duration = Duration::from_mins(2);
const ENCODER_INACTIVITY_TIMEOUT: Duration = Duration::from_mins(1);
const MAX_CAPTURE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENCODED_FRAMES: u64 = 1_000_000;
const MAX_ENCODER_INPUT_BYTES: u64 = 128 * 1024 * 1024 * 1024;
const MAX_PROCESS_STDERR_BYTES: usize = 1024 * 1024;
const MAX_UNIT_FILES: usize = 10_000;
const MAX_UNIT_BYTES: u64 = 256 * 1024 * 1024 * 1024;

pub(super) enum RenderOutcome {
    Rejected {
        source_path: PathBuf,
        source: String,
        diagnostics: Vec<Diagnostic>,
    },
    Completed {
        source_path: PathBuf,
        source: String,
        diagnostics: Vec<Diagnostic>,
        video: EncodedVideo,
    },
}

impl RenderOutcome {
    fn rejected(source_path: PathBuf, source: String, diagnostics: Vec<Diagnostic>) -> Self {
        Self::Rejected {
            source_path,
            source,
            diagnostics,
        }
    }

    pub(super) fn write(self) -> ExitCode {
        let result = match self {
            Self::Rejected {
                source_path,
                source,
                diagnostics,
            } => {
                let mut stderr = io::stderr().lock();
                diagnostic::write_all(&mut stderr, &source_path, &source, &diagnostics)
                    .map(|()| ExitCode::FAILURE)
            }
            Self::Completed {
                source_path,
                source,
                diagnostics,
                video,
            } => write_completed(&source_path, &source, &diagnostics, &video),
        };
        result.unwrap_or(ExitCode::FAILURE)
    }
}

pub(super) async fn run(args: RenderArgs) -> Result<RenderOutcome, CliError> {
    let presentation = args.presentation();
    let output = args.output();
    let profile = RenderProfile::new(args.width, args.height)?;
    let source = fs::read_to_string(&args.screenplay)
        .map_err(|error| CliError::read_screenplay(&args.screenplay, error))?;

    let resolved = compilation::resolve(&source);
    let (film, diagnostics) = resolved.into_parts();
    let Some(film) = film else {
        return Ok(RenderOutcome::rejected(
            args.screenplay,
            source,
            diagnostics,
        ));
    };

    validate_presentation(&presentation)?;
    reject_existing_output(&output)?;
    let executables = Executables::discover(&args)?;
    create_output_directory(&output)?;
    let ffprobe = ffprobe(executables.ffprobe);
    let frozen = FrozenCatalog::freeze(&film, source_directory(&args.screenplay), &ffprobe).await?;
    let solved = compilation::solve(
        film,
        frozen.facts(),
        Timebase::new(args.frame_rate),
        diagnostics,
    )?;
    let (timeline, diagnostics) = solved.into_parts();
    let Some(timeline) = timeline else {
        return Ok(RenderOutcome::rejected(
            args.screenplay,
            source,
            diagnostics,
        ));
    };

    let bundle = PresentationBundler::new(executables.bundler)
        .bundle(&presentation)
        .await?;
    let executable = materialize_unit(&timeline, profile, bundle, frozen)?;

    let executor = render_executor(executables.browser, executables.ffmpeg);
    let video = executor.render(executable, &output).await?;
    Ok(RenderOutcome::Completed {
        source_path: args.screenplay,
        source,
        diagnostics,
        video,
    })
}

fn materialize_unit(
    timeline: &TimelineIr,
    profile: RenderProfile,
    bundle: BundleArtifact,
    frozen: FrozenCatalog,
) -> Result<ExecutableUnit, CliError> {
    let (bundle_directory, manifest, _bundle_root) = bundle.into_parts();
    let materialized = frozen.into_materialized()?;
    let (assets, _asset_root) = materialized.into_parts();
    let unit = RenderUnit::whole_film(timeline, manifest, profile, assets)?;

    ExecutableUnit::materialize(unit, &bundle_directory, unit_root_limits()).map_err(Into::into)
}

fn reject_existing_output(output: &Path) -> Result<(), CliError> {
    if output.exists() {
        return Err(CliError::OutputExists(output.to_owned()));
    }
    Ok(())
}

fn create_output_directory(output: &Path) -> Result<(), CliError> {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| CliError::create_output_directory(parent, error))
}

fn validate_presentation(presentation: &Path) -> Result<(), CliError> {
    let metadata = fs::metadata(presentation)
        .map_err(|error| CliError::inspect_presentation(presentation, error))?;
    if !metadata.is_file() {
        return Err(CliError::InvalidPresentation(presentation.to_owned()));
    }
    Ok(())
}

fn ffprobe(executable: PathBuf) -> Ffprobe {
    Ffprobe::new(executable, PROCESS_DEADLINE, Ffprobe::MAX_OUTPUT_BYTES)
        .expect("the Gate-one probe policy stays within the media safety envelope")
}

fn render_executor(browser: PathBuf, ffmpeg: PathBuf) -> RenderExecutor {
    let browser_limits = BrowserLimits::new(PROCESS_DEADLINE, MAX_CAPTURE_BYTES)
        .expect("the Gate-one browser policy stays within the render safety envelope");
    let encode_limits = EncodeLimits::new(
        ENCODER_INACTIVITY_TIMEOUT,
        MAX_ENCODED_FRAMES,
        MAX_ENCODER_INPUT_BYTES,
        MAX_PROCESS_STDERR_BYTES,
    )
    .expect("the Gate-one encoder policy stays within the render safety envelope");
    let ffmpeg = Ffmpeg::new(ffmpeg, encode_limits)
        .expect("environment discovery returns a non-empty FFmpeg path");

    RenderExecutor::new(browser, browser_limits, ffmpeg)
}

fn unit_root_limits() -> UnitRootLimits {
    UnitRootLimits::new(MAX_UNIT_FILES, MAX_UNIT_BYTES)
        .expect("the Gate-one unit policy stays within the render safety envelope")
}

fn write_completed(
    source_path: &Path,
    source: &str,
    diagnostics: &[Diagnostic],
    video: &EncodedVideo,
) -> io::Result<ExitCode> {
    let mut stderr = io::stderr().lock();
    diagnostic::write_all(&mut stderr, source_path, source, diagnostics)?;
    drop(stderr);

    let mut stdout = io::stdout().lock();
    writeln!(
        stdout,
        "Rendered {} frames to {}",
        video.frames(),
        video.path().display(),
    )?;
    Ok(ExitCode::SUCCESS)
}
