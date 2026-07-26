//! Local composition root for compile, freeze, partition, execute, and assemble.
//!
//! Each phase consumes the previous phase's checked value. No timing or render-
//! graph rule is recreated at this I/O boundary.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

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
use crate::artifact_cache::{ArtifactCache, ArtifactReuse, CacheAdmission};
use crate::assets::FrozenCatalog;
use crate::bundler::{BundleArtifact, BundleRegion, PresentationBundler};
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
    capture_mode: BrowserCaptureMode,
    ffmpeg: PathBuf,
    graphics_backend: BrowserGraphicsBackend,
    video_encoder_threads: usize,
}

struct ExecutedRender {
    capture_mode: BrowserCaptureMode,
    graphics_backend: BrowserGraphicsBackend,
    reuse: ArtifactReuse,
    capture: Duration,
    assemble: Duration,
    video: EncodedVideo,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LocalRenderSummary {
    reuse: ArtifactReuse,
    timings: RenderTimings,
}

#[derive(Clone, Copy, Debug)]
struct RenderTimings {
    prepare: Duration,
    bundle: Duration,
    plan: Duration,
    capture: Duration,
    assemble: Duration,
    total: Duration,
}

impl fmt::Display for RenderTimings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "prepare {} ms, bundle {} ms, plan {} ms, capture {} ms, assemble {} ms, total {} ms",
            self.prepare.as_millis(),
            self.bundle.as_millis(),
            self.plan.as_millis(),
            self.capture.as_millis(),
            self.assemble.as_millis(),
            self.total.as_millis(),
        )
    }
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
        summary: LocalRenderSummary,
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
                summary,
                video,
            } => write_completed(&screenplay, capture_mode, graphics_backend, summary, &video),
        };
        result.unwrap_or(ExitCode::FAILURE)
    }
}

pub(super) async fn run(args: RenderArgs) -> Result<RenderOutcome, CliError> {
    let total_started = Instant::now();
    let output = args.output();
    let cache_admission = match args.browser.as_ref() {
        Some(_) => CacheAdmission::Ephemeral,
        None => CacheAdmission::Persistent,
    };
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

    let prepare = total_started.elapsed();
    let bundle_started = Instant::now();
    let bundle_artifact = PresentationBundler::new(executables.bundler)
        .bundle(&source, source_directory(&args.screenplay))
        .await?;
    let bundle = bundle_started.elapsed();

    let plan_started = Instant::now();
    let (partitions, units) = materialize_units(&timeline, profile, &bundle_artifact, frozen)?;
    let plan = plan_started.elapsed();

    let graphics_backend = args
        .graphics_backend()
        .unwrap_or_else(local_graphics_backend);
    let executed = LocalExecutorOptions {
        browser: executables.browser.path,
        capture_mode: executables.browser.capture_mode,
        ffmpeg: executables.ffmpeg,
        graphics_backend,
        video_encoder_threads: args.video_encoder_threads(),
    }
    .execute(&partitions, &units, cache_admission, &output)
    .await?;
    let summary = LocalRenderSummary {
        reuse: executed.reuse,
        timings: RenderTimings {
            prepare,
            bundle,
            plan,
            capture: executed.capture,
            assemble: executed.assemble,
            total: total_started.elapsed(),
        },
    };
    Ok(RenderOutcome::Completed {
        screenplay: AuthoredReport {
            path: args.screenplay,
            source,
            diagnostics,
        },
        capture_mode: executed.capture_mode,
        graphics_backend: executed.graphics_backend,
        summary,
        video: executed.video,
    })
}

fn materialize_units(
    timeline: &TimelineIr,
    profile: RenderProfile,
    bundle: &BundleArtifact,
    frozen: FrozenCatalog,
) -> Result<(PartitionPlan, Vec<ExecutableUnit>), CliError> {
    let materialized = frozen.into_materialized()?;
    let partitions = RenderGraph::from_timeline(timeline, bundle.manifest().temporal_capability())?
        .into_partition();
    let regions = (0..partitions.units().len())
        .map(|index| bundle.region(index))
        .collect::<Result<Vec<_>, _>>()?;
    let (directories, manifests): (Vec<_>, Vec<_>) =
        regions.into_iter().map(BundleRegion::into_parts).unzip();
    let planned = RenderUnit::from_partitioned_bundles(
        timeline,
        &partitions,
        manifests,
        profile,
        materialized.assets().iter().cloned(),
    )?;
    let units = planned
        .into_iter()
        .zip(&directories)
        .map(|(unit, directory)| {
            ExecutableUnit::materialize(unit, directory, execution::unit_root_limits())
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
            capture_mode,
            ffmpeg,
            graphics_backend,
            video_encoder_threads,
        } = self;
        let ffmpeg = Ffmpeg::new(
            ffmpeg,
            execution::local_encode_limits(video_encoder_threads),
        )
        .expect("environment discovery returns a non-empty FFmpeg path");

        RenderExecutor::new(browser, capture_mode, execution::browser_limits(), ffmpeg)
            .with_graphics_backend(graphics_backend)
    }

    async fn execute(
        self,
        partitions: &PartitionPlan,
        units: &[ExecutableUnit],
        cache_admission: CacheAdmission,
        output: &Path,
    ) -> Result<ExecutedRender, CliError> {
        let executor = self.into_executor();
        let capture_mode = executor.capture_mode();
        let graphics_backend = executor.graphics_backend();
        let cache =
            ArtifactCache::from_environment(cache_admission, capture_mode, graphics_backend)?;

        let capture_started = Instant::now();
        let artifacts = cache
            .capture(&executor, units, execution::frame_artifact_limits())
            .await?;
        let capture = capture_started.elapsed();
        let reuse = artifacts.reuse();

        let assemble_started = Instant::now();
        let video = executor
            .assemble_frame_artifacts(
                partitions,
                units,
                artifacts.as_slice(),
                cache.environment(),
                output,
            )
            .await?;

        Ok(ExecutedRender {
            capture_mode,
            graphics_backend,
            reuse,
            capture,
            assemble: assemble_started.elapsed(),
            video,
        })
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
    summary: LocalRenderSummary,
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
    writeln!(
        stdout,
        "Reused {}/{} regions and {}/{} frames",
        summary.reuse.reused_regions(),
        summary.reuse.regions(),
        summary.reuse.reused_frames(),
        video.frames(),
    )?;
    writeln!(stdout, "Timing: {}", summary.timings)?;
    Ok(ExitCode::SUCCESS)
}
