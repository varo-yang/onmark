//! Local composition root for compile, freeze, partition, execute, and assemble.
//!
//! Each phase consumes the previous phase's checked value. No timing or render-
//! graph rule is recreated at this I/O boundary.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use onmark_core::compiler;
use onmark_core::diagnostics::Diagnostic;
use onmark_core::model::Timebase;
use onmark_core::render_graph::{PartitionPlan, RenderGraph};
use onmark_core::timeline::TimelineIr;
use onmark_media::Ffprobe;
use onmark_render::{
    BrowserCaptureMode, BrowserGraphicsBackend, EncodedVideo, ExecutableUnit, Ffmpeg,
    RenderExecutor, RenderProfile, RenderUnit,
};

use crate::arguments::{RenderArgs, source_directory};
use crate::assets::FrozenCatalog;
use crate::bundler::{BundleArtifact, PresentationBundler, PresentationSource};
use crate::compilation;
use crate::diagnostic;
use crate::environment::Executables;
use crate::execution;
use crate::failure::CliError;
use crate::input;
use crate::subtitle::SubtitleImport;

pub(super) struct AuthoredReport {
    path: PathBuf,
    source: String,
    diagnostics: Vec<Diagnostic>,
}

struct LocalExecutorOptions {
    browser: PathBuf,
    ffmpeg: PathBuf,
    graphics_backend: BrowserGraphicsBackend,
    video_encoder_threads: usize,
}

/// Authored rejection or a completed local render, both retaining diagnostics.
pub(super) enum RenderOutcome {
    Rejected {
        report: AuthoredReport,
    },
    Completed {
        screenplay: AuthoredReport,
        capture_mode: BrowserCaptureMode,
        graphics_backend: BrowserGraphicsBackend,
        video: EncodedVideo,
    },
}

impl RenderOutcome {
    fn rejected(source_path: PathBuf, source: String, diagnostics: Vec<Diagnostic>) -> Self {
        Self::Rejected {
            report: AuthoredReport {
                path: source_path,
                source,
                diagnostics,
            },
        }
    }

    fn rejected_subtitle(rejected: crate::subtitle::RejectedSubtitle) -> Self {
        let (path, source, diagnostics) = rejected.into_parts();
        Self::Rejected {
            report: AuthoredReport {
                path,
                source,
                diagnostics,
            },
        }
    }

    pub(super) fn write(self) -> ExitCode {
        let result = match self {
            Self::Rejected { report } => {
                let mut stderr = io::stderr().lock();
                write_report(&mut stderr, &report).map(|()| ExitCode::FAILURE)
            }
            Self::Completed {
                screenplay,
                capture_mode,
                graphics_backend,
                video,
            } => write_completed(&screenplay, capture_mode, graphics_backend, &video),
        };
        result.unwrap_or(ExitCode::FAILURE)
    }
}

pub(super) async fn run(args: RenderArgs) -> Result<RenderOutcome, CliError> {
    let output = args.output();
    let profile = RenderProfile::new(args.width, args.height)?;
    let source = input::read_utf8(
        &args.screenplay,
        u64::try_from(onmark_core::syntax::MAX_SCREENPLAY_BYTES)
            .expect("the screenplay byte limit fits in u64"),
    )
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
    let caption_track = match args
        .subtitle
        .as_deref()
        .map(SubtitleImport::load)
        .transpose()?
    {
        Some(SubtitleImport::Track(track)) => Some(track),
        Some(SubtitleImport::Rejected(rejected)) => {
            return Ok(RenderOutcome::rejected_subtitle(rejected));
        }
        None => None,
    };

    let presentation = presentation_source(&args)?;
    reject_existing_output(&output)?;
    let executables = Executables::discover(&args).await?;
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
    let timeline = compiler::import_captions(timeline, caption_track)?;

    let bundle = PresentationBundler::new(executables.bundler)
        .bundle(&presentation)
        .await?;
    let (partitions, units) = materialize_units(&timeline, profile, &bundle, frozen)?;

    let graphics_backend = args
        .graphics_backend()
        .unwrap_or_else(local_graphics_backend);
    let executor = LocalExecutorOptions {
        browser: executables.browser,
        ffmpeg: executables.ffmpeg,
        graphics_backend,
        video_encoder_threads: args.video_encoder_threads(),
    }
    .into_executor();
    let capture_mode = executor.capture_mode();
    let graphics_backend = executor.graphics_backend();
    let video = executor
        .render_partitioned(&partitions, units, &output)
        .await?;
    Ok(RenderOutcome::Completed {
        screenplay: AuthoredReport {
            path: args.screenplay,
            source,
            diagnostics,
        },
        capture_mode,
        graphics_backend,
        video,
    })
}

fn materialize_units(
    timeline: &TimelineIr,
    profile: RenderProfile,
    bundle: &BundleArtifact,
    frozen: FrozenCatalog,
) -> Result<(PartitionPlan, Vec<ExecutableUnit>), CliError> {
    let materialized = frozen.into_materialized()?;
    let bundle_directory = bundle.directory();
    let partitions = RenderGraph::from_timeline(timeline, bundle.manifest().temporal_capability())?
        .into_partition();
    let planned = RenderUnit::from_partition_plan(
        timeline,
        &partitions,
        bundle.manifest(),
        profile,
        materialized.assets().iter().cloned(),
    )?;
    let units = planned
        .into_iter()
        .map(|unit| {
            ExecutableUnit::materialize(unit, &bundle_directory, execution::unit_root_limits())
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok((partitions, units))
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

fn presentation_source(args: &RenderArgs) -> Result<PresentationSource, CliError> {
    if let Some(presentation) = args.presentation() {
        validate_presentation_file(presentation)?;
        return Ok(PresentationSource::Custom(presentation.to_owned()));
    }

    Ok(PresentationSource::SemanticDom {
        stylesheet: optional_presentation_file(args.stylesheet())?,
        motion: optional_presentation_file(args.motion())?,
    })
}

fn optional_presentation_file(path: PathBuf) -> Result<Option<PathBuf>, CliError> {
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(path)),
        Ok(_) => Err(CliError::InvalidPresentationSource(path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CliError::inspect_presentation_source(&path, error)),
    }
}

fn validate_presentation_file(presentation: &Path) -> Result<(), CliError> {
    let metadata = fs::metadata(presentation)
        .map_err(|error| CliError::inspect_presentation_source(presentation, error))?;
    if !metadata.is_file() {
        return Err(CliError::InvalidPresentationSource(presentation.to_owned()));
    }
    Ok(())
}

fn ffprobe(executable: PathBuf) -> Ffprobe {
    Ffprobe::new(
        executable,
        execution::process_deadline(),
        Ffprobe::MAX_OUTPUT_BYTES,
    )
    .expect("the CLI probe policy stays within the media safety envelope")
}

impl LocalExecutorOptions {
    fn into_executor(self) -> RenderExecutor {
        let Self {
            browser,
            ffmpeg,
            graphics_backend,
            video_encoder_threads,
        } = self;
        let ffmpeg = Ffmpeg::new(
            ffmpeg,
            execution::local_encode_limits(video_encoder_threads),
        )
        .expect("environment discovery returns a non-empty FFmpeg path");

        RenderExecutor::new(browser, execution::browser_limits(), ffmpeg)
            .with_graphics_backend(graphics_backend)
    }
}

#[cfg(target_os = "macos")]
const fn local_graphics_backend() -> BrowserGraphicsBackend {
    BrowserGraphicsBackend::Metal
}

#[cfg(not(target_os = "macos"))]
const fn local_graphics_backend() -> BrowserGraphicsBackend {
    BrowserGraphicsBackend::SwiftShader
}

fn write_report(writer: &mut impl Write, report: &AuthoredReport) -> io::Result<()> {
    diagnostic::write_all(writer, &report.path, &report.source, &report.diagnostics)
}

fn write_completed(
    report: &AuthoredReport,
    capture_mode: BrowserCaptureMode,
    graphics_backend: BrowserGraphicsBackend,
    video: &EncodedVideo,
) -> io::Result<ExitCode> {
    let mut stderr = io::stderr().lock();
    write_report(&mut stderr, report)?;
    drop(stderr);

    let mut stdout = io::stdout().lock();
    writeln!(
        stdout,
        "Rendered {} frames with {} capture on {} to {}",
        video.frames(),
        capture_mode,
        graphics_backend,
        video.path().display(),
    )?;
    Ok(ExitCode::SUCCESS)
}
