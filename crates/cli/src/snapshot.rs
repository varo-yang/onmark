//! Exact single-frame feedback through the production render-unit path.
//!
//! Planning and visual admission complete for the whole film before one
//! existing unit is narrowed. Snapshot capture therefore cannot select a
//! cheaper path than the corresponding production frame.

use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use onmark_core::compiler;
use onmark_core::diagnostics::Diagnostic;
use onmark_core::model::{FrameIndex, FrameInterval, Timebase};
use onmark_core::render_graph::{PartitionPlan, RenderGraph};
use onmark_core::timeline::TimelineIr;
use onmark_media::Ffprobe;
use onmark_render::{
    BrowserCaptureMode, BrowserGraphicsBackend, CapturedFrame, ExecutableUnit, FrameArtifactError,
    RawRgbaHash, RenderProfile, RenderUnit,
};
use serde::Serialize;

use crate::arguments::{SnapshotArgs, source_directory};
use crate::artifact_cache::{ArtifactCache, ArtifactReuse, CacheAdmission};
use crate::assets::FrozenCatalog;
use crate::bundler::{BundleArtifact, BundleRegion, PresentationBundler};
use crate::compilation;
use crate::diagnostic::{self, AuthoredReport, JsonDiagnostic};
use crate::environment::Executables;
use crate::execution;
use crate::failure::CliError;
use crate::input;
use crate::output;
use crate::progress::Progress;
use crate::subtitle::SubtitleImport;
use crate::variant::VariantImport;

struct PreparedSnapshot {
    args: SnapshotArgs,
    source: String,
    diagnostics: Vec<Diagnostic>,
    timeline: TimelineIr,
    frozen: FrozenCatalog,
    output: PathBuf,
    profile: RenderProfile,
    executables: Executables,
    elapsed: Duration,
}

enum Preparation {
    Rejected(Box<SnapshotOutcome>),
    Ready(Box<PreparedSnapshot>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SnapshotSelection {
    frame: FrameIndex,
    region: usize,
    evaluation: FrameInterval,
    region_output: FrameInterval,
    shots: Vec<usize>,
}

#[derive(Clone, Copy, Debug)]
struct SnapshotSummary {
    capture_mode: BrowserCaptureMode,
    graphics_backend: BrowserGraphicsBackend,
    reuse: ArtifactReuse,
    raw_rgba_hash: RawRgbaHash,
    timings: SnapshotTimings,
}

#[derive(Clone, Copy, Debug)]
struct SnapshotTimings {
    prepare: Duration,
    plan: Duration,
    bundle: Duration,
    capture: Duration,
    publish: Duration,
    total: Duration,
}

/// Authored rejection or one completed exact-frame capture.
pub(super) enum SnapshotOutcome {
    Rejected { report: AuthoredReport, json: bool },
    Completed(Box<CompletedSnapshot>),
}

pub(super) struct CompletedSnapshot {
    report: AuthoredReport,
    output: PathBuf,
    selection: SnapshotSelection,
    summary: SnapshotSummary,
    json: bool,
}

impl SnapshotOutcome {
    fn rejected(path: PathBuf, source: String, diagnostics: Vec<Diagnostic>, json: bool) -> Self {
        Self::Rejected {
            report: AuthoredReport {
                path,
                source,
                diagnostics,
            },
            json,
        }
    }

    fn rejected_subtitle(rejected: crate::subtitle::RejectedSubtitle, json: bool) -> Self {
        let (path, source, diagnostics) = rejected.into_parts();
        Self::rejected(path, source, diagnostics, json)
    }

    pub(super) fn write(self) -> ExitCode {
        let result = match self {
            Self::Rejected { report, json } => write_rejected(&report, json),
            Self::Completed(completed) => {
                let CompletedSnapshot {
                    report,
                    output,
                    selection,
                    summary,
                    json,
                } = *completed;
                write_completed(&report, &output, &selection, summary, json)
            }
        };
        result.unwrap_or(ExitCode::FAILURE)
    }
}

pub(super) async fn run(args: SnapshotArgs, json: bool) -> Result<SnapshotOutcome, CliError> {
    let total_started = Instant::now();
    let progress = Progress::for_command(json);
    progress.started("prepare")?;
    let prepared = match prepare(args, json, total_started).await? {
        Preparation::Rejected(outcome) => return Ok(*outcome),
        Preparation::Ready(prepared) => *prepared,
    };
    progress.completed("prepare", prepared.elapsed)?;

    execute(prepared, json, total_started, progress).await
}

async fn prepare(
    args: SnapshotArgs,
    json: bool,
    started: Instant,
) -> Result<Preparation, CliError> {
    let output = args.output()?;
    let profile = RenderProfile::new(args.validation.width, args.validation.height)?;
    let source = read_screenplay(&args)?;
    let resolved = compilation::resolve(&source);
    let (film, diagnostics) = resolved.into_parts();
    let Some(film) = film else {
        return Ok(Preparation::Rejected(Box::new(SnapshotOutcome::rejected(
            args.validation.screenplay,
            source,
            diagnostics,
            json,
        ))));
    };
    let film = match VariantImport::apply(args.validation.variant.as_deref(), film)? {
        VariantImport::Film(film) => film,
        VariantImport::Rejected(rejected) => {
            let (path, source, diagnostics) = rejected.into_parts();
            return Ok(Preparation::Rejected(Box::new(SnapshotOutcome::rejected(
                path,
                source,
                diagnostics,
                json,
            ))));
        }
    };
    let caption_track = match args
        .validation
        .subtitle
        .as_deref()
        .map(SubtitleImport::load)
        .transpose()?
    {
        Some(SubtitleImport::Track(track)) => Some(track),
        Some(SubtitleImport::Rejected(rejected)) => {
            return Ok(Preparation::Rejected(Box::new(
                SnapshotOutcome::rejected_subtitle(rejected, json),
            )));
        }
        None => None,
    };

    output::reject_existing(&output)?;
    let executables = Executables::discover_snapshot(&args).await?;
    output::create_parent(&output)?;
    let probe = ffprobe(executables.ffprobe.clone());
    let frozen =
        FrozenCatalog::freeze(&film, source_directory(&args.validation.screenplay), &probe).await?;
    let solved = compilation::solve(
        film,
        frozen.facts(),
        Timebase::new(args.validation.frame_rate),
        diagnostics,
    )?;
    let (timeline, diagnostics) = solved.into_parts();
    let Some(timeline) = timeline else {
        return Ok(Preparation::Rejected(Box::new(SnapshotOutcome::rejected(
            args.validation.screenplay,
            source,
            diagnostics,
            json,
        ))));
    };
    let timeline = compiler::import_captions(timeline, caption_track)?;

    Ok(Preparation::Ready(Box::new(PreparedSnapshot {
        args,
        source,
        diagnostics,
        timeline,
        frozen,
        output,
        profile,
        executables,
        elapsed: started.elapsed(),
    })))
}

async fn execute(
    prepared: PreparedSnapshot,
    json: bool,
    total_started: Instant,
    progress: Progress,
) -> Result<SnapshotOutcome, CliError> {
    let PreparedSnapshot {
        args,
        source,
        diagnostics,
        timeline,
        frozen,
        output,
        profile,
        executables,
        elapsed: prepare,
    } = prepared;

    progress.started("plan")?;
    let plan_started = Instant::now();
    let partitions =
        RenderGraph::from_timeline(&timeline, PresentationBundler::temporal_capability())?
            .into_partition();
    let selection = select_frame(&partitions, FrameIndex::new(args.frame))?;
    let plan = plan_started.elapsed();
    progress.completed("plan", plan)?;

    progress.started("bundle")?;
    let bundle_started = Instant::now();
    let bundler = PresentationBundler::new(executables.bundler.clone());
    let bundle = bundler
        .bundle(
            &source,
            source_directory(&args.validation.screenplay),
            &partitions,
        )
        .await?;
    let unit = materialize_frame(&timeline, profile, &partitions, &bundle, frozen, &selection)?;
    let bundle = bundle_started.elapsed();
    progress.completed("bundle", bundle)?;

    let graphics_backend = args
        .graphics_backend()
        .unwrap_or_else(execution::local_graphics_backend);
    let executor = execution::visual_feedback_executor(
        &executables.browser.path,
        executables.browser.capture_mode,
        &executables.ffmpeg,
        graphics_backend,
    );
    let capture_mode = executor.capture_mode();
    let cache_admission = if args.browser.is_some() {
        CacheAdmission::Ephemeral
    } else {
        CacheAdmission::Persistent
    };
    let cache = ArtifactCache::from_environment(cache_admission, capture_mode, graphics_backend)?;

    progress.started("capture")?;
    let capture_started = Instant::now();
    let artifacts = cache
        .capture(
            &executor,
            std::slice::from_ref(&unit),
            execution::snapshot_artifact_limits(),
        )
        .await?;
    let capture = capture_started.elapsed();
    progress.completed("capture", capture)?;
    let reuse = artifacts.reuse();
    let frame = artifacts.as_slice()[0]
        .single_frame()
        .await
        .map_err(SnapshotError::from)?;

    progress.started("publish")?;
    let publish_started = Instant::now();
    publish_png(&frame, &output)?;
    let publish = publish_started.elapsed();
    progress.completed("publish", publish)?;

    Ok(SnapshotOutcome::Completed(Box::new(CompletedSnapshot {
        report: AuthoredReport {
            path: args.validation.screenplay,
            source,
            diagnostics,
        },
        output,
        selection,
        summary: SnapshotSummary {
            capture_mode,
            graphics_backend,
            reuse,
            raw_rgba_hash: frame.raw_rgba_hash(),
            timings: SnapshotTimings {
                prepare,
                plan,
                bundle,
                capture,
                publish,
                total: total_started.elapsed(),
            },
        },
        json,
    })))
}

fn select_frame(
    partitions: &PartitionPlan,
    frame: FrameIndex,
) -> Result<SnapshotSelection, SnapshotError> {
    let selected = partitions
        .units()
        .iter()
        .enumerate()
        .find(|(_, partition)| contains(partition.output(), frame));
    let Some((region, partition)) = selected else {
        return Err(SnapshotError::FrameOutsideTimeline {
            frame,
            timeline: partitions.interval(),
        });
    };

    Ok(SnapshotSelection {
        frame,
        region,
        evaluation: partition.evaluation(),
        region_output: partition.output(),
        shots: partition.shots().map(|shot| shot.get()).collect(),
    })
}

const fn contains(interval: FrameInterval, frame: FrameIndex) -> bool {
    interval.start().get() <= frame.get() && frame.get() < interval.end().get()
}

fn materialize_frame(
    timeline: &TimelineIr,
    profile: RenderProfile,
    partitions: &PartitionPlan,
    bundle: &BundleArtifact,
    frozen: FrozenCatalog,
    selection: &SnapshotSelection,
) -> Result<ExecutableUnit, CliError> {
    let materialized = frozen.into_materialized()?;
    // Admission is normalized across the complete region sequence. Building
    // only the selected region could choose a pixel path that production later
    // rejects when a neighboring region is considered.
    let regions = (0..partitions.units().len())
        .map(|index| bundle.region(index))
        .collect::<Result<Vec<_>, _>>()?;
    let (directories, manifests): (Vec<_>, Vec<_>) =
        regions.into_iter().map(BundleRegion::into_parts).unzip();
    let units = RenderUnit::from_partitioned_bundles(
        timeline,
        partitions,
        manifests,
        profile,
        materialized.assets().iter().cloned(),
    )?;
    let unit = units
        .into_iter()
        .nth(selection.region)
        .expect("the checked partition and bundle counts retain the selected unit")
        .into_frame(selection.frame)?;
    let directory = directories
        .get(selection.region)
        .expect("the checked bundle count retains the selected directory");

    Ok(ExecutableUnit::materialize(
        unit,
        directory,
        execution::unit_root_limits(),
    )?)
}

fn read_screenplay(args: &SnapshotArgs) -> Result<String, CliError> {
    let limit = u64::try_from(onmark_core::syntax::MAX_SCREENPLAY_BYTES)
        .expect("the screenplay byte limit fits in u64");
    input::read_utf8(&args.validation.screenplay, limit)
        .map_err(|error| CliError::read_screenplay(&args.validation.screenplay, error))
}

fn ffprobe(executable: PathBuf) -> Ffprobe {
    Ffprobe::new(
        executable,
        execution::process_deadline(),
        Ffprobe::MAX_OUTPUT_BYTES,
    )
    .expect("the CLI probe policy stays within the media safety envelope")
}

fn publish_png(frame: &CapturedFrame, output: &Path) -> Result<(), SnapshotError> {
    let mut staging = tempfile::Builder::new()
        .prefix(".onmark-snapshot-")
        .tempfile_in(output::parent(output))
        .map_err(|source| SnapshotError::Stage {
            output: output.to_owned(),
            source,
        })?;
    staging
        .write_all(frame.png().as_bytes())
        .map_err(|source| SnapshotError::Write {
            output: output.to_owned(),
            source,
        })?;
    staging
        .persist_noclobber(output)
        .map_err(|error| SnapshotError::Publish {
            output: output.to_owned(),
            source: error.error,
        })?;
    Ok(())
}

fn write_rejected(report: &AuthoredReport, json: bool) -> io::Result<ExitCode> {
    if json {
        write_json(report, None)?;
    } else {
        let mut stderr = io::stderr().lock();
        diagnostic::write_all(
            &mut stderr,
            &report.path,
            &report.source,
            &report.diagnostics,
        )?;
    }
    Ok(ExitCode::FAILURE)
}

fn write_completed(
    report: &AuthoredReport,
    output: &Path,
    selection: &SnapshotSelection,
    summary: SnapshotSummary,
    json: bool,
) -> io::Result<ExitCode> {
    if json {
        write_json(report, Some(JsonCompleted::new(output, selection, summary)))?;
        return Ok(ExitCode::SUCCESS);
    }

    let mut stderr = io::stderr().lock();
    diagnostic::write_all(
        &mut stderr,
        &report.path,
        &report.source,
        &report.diagnostics,
    )?;
    drop(stderr);

    let mut stdout = io::stdout().lock();
    writeln!(
        stdout,
        "Captured frame {} from region {} with {} capture on {} to {}",
        selection.frame.get(),
        selection.region,
        summary.capture_mode,
        summary.graphics_backend,
        output.display(),
    )?;
    writeln!(stdout, "Raw RGBA SHA-256: {}", summary.raw_rgba_hash)?;
    writeln!(
        stdout,
        "Reused {}/{} region; timing: {}",
        summary.reuse.reused_regions(),
        summary.reuse.regions(),
        summary.timings,
    )?;
    Ok(ExitCode::SUCCESS)
}

fn write_json(report: &AuthoredReport, completed: Option<JsonCompleted>) -> io::Result<()> {
    let document = JsonSnapshotReport {
        version: 1,
        command: "snapshot",
        captured: completed.is_some(),
        source: report.path.display().to_string(),
        diagnostics: report
            .diagnostics
            .iter()
            .map(JsonDiagnostic::from)
            .collect(),
        completed,
    };
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &document)?;
    writeln!(stdout)
}

impl fmt::Display for SnapshotTimings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "prepare {} ms, plan {} ms, bundle {} ms, capture {} ms, publish {} ms, total {} ms",
            self.prepare.as_millis(),
            self.plan.as_millis(),
            self.bundle.as_millis(),
            self.capture.as_millis(),
            self.publish.as_millis(),
            self.total.as_millis(),
        )
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonSnapshotReport<'a> {
    version: u16,
    command: &'static str,
    captured: bool,
    source: String,
    diagnostics: Vec<JsonDiagnostic<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed: Option<JsonCompleted>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonCompleted {
    output: String,
    frame: u64,
    region: usize,
    evaluation: JsonInterval,
    region_output: JsonInterval,
    shots: Vec<usize>,
    raw_rgba_sha256: String,
    capture_mode: String,
    graphics_backend: String,
    reused: bool,
    timing_milliseconds: JsonTimings,
}

impl JsonCompleted {
    fn new(output: &Path, selection: &SnapshotSelection, summary: SnapshotSummary) -> Self {
        Self {
            output: output.display().to_string(),
            frame: selection.frame.get(),
            region: selection.region,
            evaluation: selection.evaluation.into(),
            region_output: selection.region_output.into(),
            shots: selection.shots.clone(),
            raw_rgba_sha256: summary.raw_rgba_hash.to_string(),
            capture_mode: summary.capture_mode.to_string(),
            graphics_backend: summary.graphics_backend.to_string(),
            reused: summary.reuse.reused_regions() == 1,
            timing_milliseconds: summary.timings.into(),
        }
    }
}

#[derive(Serialize)]
struct JsonInterval {
    start: u64,
    end: u64,
}

impl From<FrameInterval> for JsonInterval {
    fn from(interval: FrameInterval) -> Self {
        Self {
            start: interval.start().get(),
            end: interval.end().get(),
        }
    }
}

#[derive(Serialize)]
struct JsonTimings {
    prepare: u128,
    plan: u128,
    bundle: u128,
    capture: u128,
    publish: u128,
    total: u128,
}

impl From<SnapshotTimings> for JsonTimings {
    fn from(timings: SnapshotTimings) -> Self {
        Self {
            prepare: timings.prepare.as_millis(),
            plan: timings.plan.as_millis(),
            bundle: timings.bundle.as_millis(),
            capture: timings.capture.as_millis(),
            publish: timings.publish.as_millis(),
            total: timings.total.as_millis(),
        }
    }
}

/// Failure specific to exact-frame selection or PNG publication.
#[derive(Debug)]
pub(super) enum SnapshotError {
    FrameOutsideTimeline {
        frame: FrameIndex,
        timeline: FrameInterval,
    },
    Artifact(FrameArtifactError),
    Stage {
        output: PathBuf,
        source: io::Error,
    },
    Write {
        output: PathBuf,
        source: io::Error,
    },
    Publish {
        output: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameOutsideTimeline { frame, timeline } => write!(
                formatter,
                "snapshot frame {} lies outside the film output {}..{}",
                frame.get(),
                timeline.start().get(),
                timeline.end().get(),
            ),
            Self::Artifact(source) => source.fmt(formatter),
            Self::Stage { output, .. } => {
                write!(
                    formatter,
                    "failed to stage snapshot output {}",
                    output.display()
                )
            }
            Self::Write { output, .. } => {
                write!(
                    formatter,
                    "failed to write snapshot output {}",
                    output.display()
                )
            }
            Self::Publish { output, .. } => {
                write!(
                    formatter,
                    "failed to publish snapshot output {}",
                    output.display()
                )
            }
        }
    }
}

impl Error for SnapshotError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::FrameOutsideTimeline { .. } => None,
            Self::Artifact(source) => Some(source),
            Self::Stage { source, .. }
            | Self::Write { source, .. }
            | Self::Publish { source, .. } => Some(source),
        }
    }
}

impl From<FrameArtifactError> for SnapshotError {
    fn from(source: FrameArtifactError) -> Self {
        Self::Artifact(source)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use onmark_core::model::{FrameIndex, PresentationTemporalCapability};
    use onmark_core::render_graph::RenderGraph;

    use super::{SnapshotError, select_frame};
    use crate::compilation;

    #[test]
    fn selects_the_one_region_that_publishes_the_requested_frame() {
        let source = concat!(
            "<om-film><om-scene>",
            r#"<om-shot duration="1s"></om-shot>"#,
            r#"<om-shot duration="1s"></om-shot>"#,
            "</om-scene></om-film>",
        );
        let resolved = compilation::resolve(source);
        let (film, diagnostics) = resolved.into_parts();
        assert!(diagnostics.is_empty());
        let film = film.expect("the fixture resolves");
        let solved = compilation::solve(
            film,
            &BTreeMap::default(),
            onmark_core::model::Timebase::new(
                onmark_core::model::FrameRate::new(30, 1).expect("the fixture rate is valid"),
            ),
            diagnostics,
        )
        .expect("the fixture solves");
        let (timeline, diagnostics) = solved.into_parts();
        assert!(diagnostics.is_empty());
        let partitions = RenderGraph::from_timeline(
            &timeline.expect("the fixture has Timeline IR"),
            PresentationTemporalCapability::RandomAccess,
        )
        .expect("the fixture graph is complete")
        .into_partition();

        let selection =
            select_frame(&partitions, FrameIndex::new(30)).expect("frame 30 starts region one");

        assert_eq!(selection.region, 1);
        assert_eq!(selection.shots, vec![1]);
        assert!(matches!(
            select_frame(&partitions, FrameIndex::new(60)),
            Err(SnapshotError::FrameOutsideTimeline { .. }),
        ));
    }
}
