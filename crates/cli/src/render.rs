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
    EncodedVideo, ExecutableUnit, Ffmpeg, RenderExecutor, RenderProfile, RenderUnit,
};

use crate::arguments::{RenderArgs, source_directory};
use crate::assets::FrozenCatalog;
use crate::bundler::{BundleArtifact, PresentationBundler};
use crate::compilation;
use crate::diagnostic;
use crate::environment::Executables;
use crate::execution;
use crate::failure::CliError;
use crate::subtitle::SubtitleImport;

pub(super) struct AuthoredReport {
    path: PathBuf,
    source: String,
    diagnostics: Vec<Diagnostic>,
}

/// Authored rejection or a completed local render, both retaining diagnostics.
pub(super) enum RenderOutcome {
    Rejected {
        report: AuthoredReport,
    },
    Completed {
        screenplay: AuthoredReport,
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
            Self::Completed { screenplay, video } => write_completed(&screenplay, &video),
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
    let timeline = compiler::import_captions(timeline, caption_track)?;

    let bundle = PresentationBundler::new(executables.bundler)
        .bundle(&presentation, args.temporal_capability)
        .await?;
    let (partitions, units) = materialize_units(&timeline, profile, &bundle, frozen)?;

    let executor = render_executor(executables.browser, executables.ffmpeg);
    let video = executor
        .render_partitioned(&partitions, units, &output)
        .await?;
    Ok(RenderOutcome::Completed {
        screenplay: AuthoredReport {
            path: args.screenplay,
            source,
            diagnostics,
        },
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
    let partitions = RenderGraph::from_timeline(timeline, bundle.manifest().temporal_capability())
        .into_partition();
    let mut units = Vec::with_capacity(partitions.units().len());

    for partition in partitions.units() {
        // Partition roots are isolated. Shared frozen bytes may therefore be
        // selected by multiple partitions, but only graph-proven inputs enter
        // each composition.
        let required_assets = materialized
            .assets()
            .iter()
            .filter(|asset| partition.requires_media_asset(asset.id()))
            .cloned();
        let unit = RenderUnit::from_partition(
            timeline,
            partition,
            bundle.manifest().clone(),
            profile,
            required_assets,
        )?;
        units.push(ExecutableUnit::materialize(
            unit,
            &bundle_directory,
            execution::unit_root_limits(),
        )?);
    }

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

fn validate_presentation(presentation: &Path) -> Result<(), CliError> {
    let metadata = fs::metadata(presentation)
        .map_err(|error| CliError::inspect_presentation(presentation, error))?;
    if !metadata.is_file() {
        return Err(CliError::InvalidPresentation(presentation.to_owned()));
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

fn render_executor(browser: PathBuf, ffmpeg: PathBuf) -> RenderExecutor {
    let ffmpeg = Ffmpeg::new(ffmpeg, execution::encode_limits())
        .expect("environment discovery returns a non-empty FFmpeg path");

    RenderExecutor::new(browser, execution::browser_limits(), ffmpeg)
}

fn write_report(writer: &mut impl Write, report: &AuthoredReport) -> io::Result<()> {
    diagnostic::write_all(writer, &report.path, &report.source, &report.diagnostics)
}

fn write_completed(report: &AuthoredReport, video: &EncodedVideo) -> io::Result<ExitCode> {
    let mut stderr = io::stderr().lock();
    write_report(&mut stderr, report)?;
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
