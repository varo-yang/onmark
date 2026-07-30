//! Exact static visual review through production regions and frame artifacts.
//!
//! The command owns checkpoint policy and report publication only. Compilation,
//! dependency planning, capture, pixels, and cache admission remain the same
//! contracts used by a complete render.

use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use onmark_core::compiler;
use onmark_core::diagnostics::Diagnostic;
use onmark_core::model::Timebase;
use onmark_core::render_graph::{PartitionPlan, RenderGraph};
use onmark_core::timeline::TimelineIr;
use onmark_media::Ffprobe;
use onmark_render::{
    BrowserCaptureMode, BrowserGraphicsBackend, CapturedFrame, FrameArtifact, FrameArtifactError,
    RenderProfile,
};
use serde::Serialize;

use crate::arguments::{ReviewArgs, source_directory};
use crate::artifact_cache::{ArtifactCache, ArtifactReuse, CacheAdmission, CapturedArtifacts};
use crate::assets::FrozenCatalog;
use crate::bundler::PresentationBundler;
use crate::captions::CaptionImport;
use crate::compilation;
use crate::diagnostic::{self, AuthoredReport, JsonDiagnostic};
use crate::environment::Executables;
use crate::execution;
use crate::failure::CliError;
use crate::input;
use crate::output;
use crate::progress::Progress;
use crate::variant::VariantImport;

use self::plan::{ReviewPlan, ReviewPlanError};
use self::report::{
    ReviewBaseline, ReviewComparison, ReviewDocument, ReviewPublication, ReviewReportError,
};

mod plan;
mod report;

struct PreparedReview {
    args: ReviewArgs,
    source: String,
    diagnostics: Vec<Diagnostic>,
    timeline: TimelineIr,
    frozen: FrozenCatalog,
    profile: RenderProfile,
    executables: Executables,
    baseline: Option<ReviewBaseline>,
    elapsed: Duration,
}

struct CapturedReview {
    artifacts: CapturedArtifacts,
    frames: Vec<CapturedFrame>,
    capture_mode: BrowserCaptureMode,
    graphics_backend: BrowserGraphicsBackend,
}

enum Preparation {
    Rejected(Box<ReviewOutcome>),
    Ready(Box<PreparedReview>),
}

#[derive(Clone, Copy, Debug)]
struct ReviewSummary {
    capture_mode: BrowserCaptureMode,
    graphics_backend: BrowserGraphicsBackend,
    reuse: ArtifactReuse,
    publication: ReviewPublication,
    timings: ReviewTimings,
}

#[derive(Clone, Copy, Debug)]
struct ReviewTimings {
    prepare: Duration,
    plan: Duration,
    bundle: Duration,
    capture: Duration,
    publish: Duration,
    total: Duration,
}

pub(super) enum ReviewOutcome {
    Rejected { report: AuthoredReport, json: bool },
    Completed(Box<CompletedReview>),
}

pub(super) struct CompletedReview {
    report: AuthoredReport,
    output: PathBuf,
    document_id: String,
    regions: usize,
    checkpoints: usize,
    baseline: Option<PathBuf>,
    comparison: Option<ReviewComparison>,
    summary: ReviewSummary,
    json: bool,
}

impl ReviewOutcome {
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

    fn rejected_captions(rejected: crate::captions::RejectedCaptions, json: bool) -> Self {
        let (path, source, diagnostics) = rejected.into_parts();
        Self::rejected(path, source, diagnostics, json)
    }

    pub(super) fn write(self) -> ExitCode {
        let result = match self {
            Self::Rejected { report, json } => write_rejected(&report, json),
            Self::Completed(completed) => write_completed(&completed),
        };
        result.unwrap_or(ExitCode::FAILURE)
    }
}

pub(super) async fn run(args: ReviewArgs, json: bool) -> Result<ReviewOutcome, CliError> {
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

async fn prepare(args: ReviewArgs, json: bool, started: Instant) -> Result<Preparation, CliError> {
    if let Some(output) = &args.output {
        output::reject_existing(output)?;
    }
    let profile = RenderProfile::new(args.validation.width, args.validation.height)?;
    let source = read_screenplay(&args)?;
    let resolved = compilation::resolve(&source);
    let (film, diagnostics) = resolved.into_parts();
    let Some(film) = film else {
        return Ok(Preparation::Rejected(Box::new(ReviewOutcome::rejected(
            args.validation.screenplay,
            source,
            diagnostics,
            json,
        ))));
    };
    let film = match VariantImport::apply(args.validation.variant.as_deref(), film)? {
        VariantImport::Film(film) => *film,
        VariantImport::Rejected(rejected) => {
            let (path, source, diagnostics) = rejected.into_parts();
            return Ok(Preparation::Rejected(Box::new(ReviewOutcome::rejected(
                path,
                source,
                diagnostics,
                json,
            ))));
        }
    };
    let caption_tracks = match CaptionImport::load(
        film.captions(),
        &args.validation.caption_tracks,
        source_directory(&args.validation.screenplay),
    )? {
        CaptionImport::Ready(tracks) => tracks,
        CaptionImport::Rejected(rejected) => {
            return Ok(Preparation::Rejected(Box::new(
                ReviewOutcome::rejected_captions(rejected, json),
            )));
        }
    };
    let baseline = args
        .against
        .clone()
        .map(ReviewBaseline::load)
        .transpose()
        .map_err(ReviewError::from)?;

    let executables = Executables::discover_visual_feedback(
        &args.validation,
        args.browser.as_deref(),
        &args.ffmpeg,
    )
    .await?;
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
        return Ok(Preparation::Rejected(Box::new(ReviewOutcome::rejected(
            args.validation.screenplay,
            source,
            diagnostics,
            json,
        ))));
    };
    let timeline = compiler::import_captions(timeline, caption_tracks)?;

    Ok(Preparation::Ready(Box::new(PreparedReview {
        args,
        source,
        diagnostics,
        timeline,
        frozen,
        profile,
        executables,
        baseline,
        elapsed: started.elapsed(),
    })))
}

async fn execute(
    prepared: PreparedReview,
    json: bool,
    total_started: Instant,
    progress: Progress,
) -> Result<ReviewOutcome, CliError> {
    let PreparedReview {
        args,
        source,
        diagnostics,
        timeline,
        frozen,
        profile,
        executables,
        baseline,
        elapsed: prepare,
    } = prepared;

    progress.started("plan")?;
    let plan_started = Instant::now();
    let (partitions, review) = plan_review(&timeline)?;
    let plan = plan_started.elapsed();
    progress.completed("plan", plan)?;

    progress.started("bundle")?;
    let bundle_started = Instant::now();
    let units = bundle_units(
        &args,
        &source,
        &timeline,
        profile,
        &partitions,
        frozen,
        &executables,
    )
    .await?;
    let bundle = bundle_started.elapsed();
    progress.completed("bundle", bundle)?;

    progress.started("capture")?;
    let capture_started = Instant::now();
    let captured = capture_review(&args, &executables, &partitions, &review, &units).await?;
    let capture = capture_started.elapsed();
    progress.completed("capture", capture)?;
    let CapturedReview {
        artifacts,
        frames,
        capture_mode,
        graphics_backend,
    } = captured;

    progress.started("publish")?;
    let publish_started = Instant::now();
    let document = ReviewDocument::build(
        &source,
        &timeline,
        profile,
        &partitions,
        &review,
        artifacts.as_slice(),
        frames,
    )
    .map_err(ReviewError::from)?;
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| document.default_output(&args.validation.screenplay));
    let publication = document
        .publish(&output, args.output.is_none())
        .map_err(ReviewError::from)?;
    let comparison = baseline.as_ref().map(|previous| document.compare(previous));
    let publish = publish_started.elapsed();
    progress.completed("publish", publish)?;

    Ok(ReviewOutcome::Completed(Box::new(CompletedReview {
        report: AuthoredReport {
            path: args.validation.screenplay,
            source,
            diagnostics,
        },
        output,
        document_id: document.id().to_owned(),
        regions: document.regions(),
        checkpoints: document.checkpoints(),
        baseline: baseline.map(|previous| previous.path().to_owned()),
        comparison,
        summary: ReviewSummary {
            capture_mode,
            graphics_backend,
            reuse: artifacts.reuse(),
            publication,
            timings: ReviewTimings {
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

fn plan_review(timeline: &TimelineIr) -> Result<(PartitionPlan, ReviewPlan), CliError> {
    let partitions =
        RenderGraph::from_timeline(timeline, PresentationBundler::temporal_capability())?
            .into_partition();
    let review = ReviewPlan::from_timeline(timeline, &partitions).map_err(ReviewError::from)?;
    Ok((partitions, review))
}

async fn bundle_units(
    args: &ReviewArgs,
    source: &str,
    timeline: &TimelineIr,
    profile: RenderProfile,
    partitions: &PartitionPlan,
    frozen: FrozenCatalog,
    executables: &Executables,
) -> Result<Vec<onmark_render::ExecutableUnit>, CliError> {
    let bundler = PresentationBundler::new(executables.bundler.clone());
    let bundle = bundler
        .bundle(
            source,
            source_directory(&args.validation.screenplay),
            partitions,
        )
        .await?;
    let materialized = frozen.into_materialized()?;
    crate::render::materialize_units(
        timeline,
        profile,
        partitions,
        &bundle,
        materialized.assets(),
    )
}

async fn capture_review(
    args: &ReviewArgs,
    executables: &Executables,
    partitions: &PartitionPlan,
    review: &ReviewPlan,
    units: &[onmark_render::ExecutableUnit],
) -> Result<CapturedReview, CliError> {
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
    let admission = if args.browser.is_some() {
        CacheAdmission::Ephemeral
    } else {
        CacheAdmission::Persistent
    };
    let cache = ArtifactCache::from_environment(admission, capture_mode, graphics_backend)?;
    let artifacts = cache
        .capture(&executor, units, execution::frame_artifact_limits())
        .await?;
    let frames = read_checkpoints(review, partitions, artifacts.as_slice()).await?;

    Ok(CapturedReview {
        artifacts,
        frames,
        capture_mode,
        graphics_backend,
    })
}

async fn read_checkpoints(
    review: &ReviewPlan,
    partitions: &PartitionPlan,
    artifacts: &[FrameArtifact],
) -> Result<Vec<CapturedFrame>, ReviewError> {
    if partitions.units().len() != artifacts.len() {
        return Err(ReviewError::ArtifactCount {
            regions: partitions.units().len(),
            artifacts: artifacts.len(),
        });
    }

    let mut requested = (0..artifacts.len()).map(|_| Vec::new()).collect::<Vec<_>>();
    for (index, checkpoint) in review.checkpoints().iter().enumerate() {
        requested
            .get_mut(checkpoint.region())
            .expect("the review plan assigns every checkpoint to a production region")
            .push((index, checkpoint.position()));
    }

    let mut frames = std::iter::repeat_with(|| None)
        .take(review.checkpoints().len())
        .collect::<Vec<_>>();
    let mut captured = 0;
    for (artifact, region) in artifacts.iter().zip(requested) {
        let positions = region
            .iter()
            .map(|(_, position)| *position)
            .collect::<Vec<_>>();
        let selected = artifact.frames_at(&positions).await?;
        captured += selected.len();
        for ((index, _), frame) in region.into_iter().zip(selected) {
            frames[index] = Some(frame);
        }
    }
    frames
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or(ReviewError::CheckpointCount {
            planned: review.checkpoints().len(),
            captured,
        })
}

fn read_screenplay(args: &ReviewArgs) -> Result<String, CliError> {
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

fn write_completed(completed: &CompletedReview) -> io::Result<ExitCode> {
    if completed.json {
        write_json(
            &completed.report,
            Some(JsonCompleted::from_completed(completed)),
        )?;
        return Ok(ExitCode::SUCCESS);
    }

    let mut stderr = io::stderr().lock();
    diagnostic::write_all(
        &mut stderr,
        &completed.report.path,
        &completed.report.source,
        &completed.report.diagnostics,
    )?;
    drop(stderr);

    let mut stdout = io::stdout().lock();
    writeln!(
        stdout,
        "{} exact review {} with {} checkpoints across {} regions at {}",
        completed.summary.publication,
        completed.document_id,
        completed.checkpoints,
        completed.regions,
        completed.output.display(),
    )?;
    writeln!(
        stdout,
        "Reused {}/{} regions ({} frames); timing: {}",
        completed.summary.reuse.reused_regions(),
        completed.summary.reuse.regions(),
        completed.summary.reuse.reused_frames(),
        completed.summary.timings,
    )?;
    if let (Some(path), Some(comparison)) = (&completed.baseline, completed.comparison) {
        writeln!(
            stdout,
            "Compared with {}: {} unchanged, {} changed, {} added, {} removed regions",
            path.display(),
            comparison.unchanged_regions(),
            comparison.changed_regions(),
            comparison.added_regions(),
            comparison.removed_regions(),
        )?;
    }
    Ok(ExitCode::SUCCESS)
}

fn write_json(report: &AuthoredReport, completed: Option<JsonCompleted>) -> io::Result<()> {
    let document = JsonReviewReport {
        version: 1,
        command: "review",
        completed: completed.is_some(),
        source: report.path.display().to_string(),
        diagnostics: report
            .diagnostics
            .iter()
            .map(JsonDiagnostic::from)
            .collect(),
        review: completed,
    };
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &document)?;
    writeln!(stdout)
}

impl fmt::Display for ReviewTimings {
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
struct JsonReviewReport<'a> {
    version: u16,
    command: &'static str,
    completed: bool,
    source: String,
    diagnostics: Vec<JsonDiagnostic<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    review: Option<JsonCompleted>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonCompleted {
    output: String,
    manifest: String,
    contact_sheet: String,
    review_id: String,
    regions: usize,
    checkpoints: usize,
    capture_mode: String,
    graphics_backend: String,
    reused_regions: usize,
    reused_frames: u64,
    publication: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    comparison: Option<JsonComparison>,
    timing_milliseconds: JsonTimings,
}

impl JsonCompleted {
    fn from_completed(completed: &CompletedReview) -> Self {
        Self {
            output: completed.output.display().to_string(),
            manifest: completed.output.join("manifest.json").display().to_string(),
            contact_sheet: completed.output.join("index.html").display().to_string(),
            review_id: completed.document_id.clone(),
            regions: completed.regions,
            checkpoints: completed.checkpoints,
            capture_mode: completed.summary.capture_mode.to_string(),
            graphics_backend: completed.summary.graphics_backend.to_string(),
            reused_regions: completed.summary.reuse.reused_regions(),
            reused_frames: completed.summary.reuse.reused_frames(),
            publication: completed.summary.publication.to_string(),
            comparison: completed.baseline.as_ref().zip(completed.comparison).map(
                |(path, comparison)| JsonComparison {
                    against: path.display().to_string(),
                    comparison,
                },
            ),
            timing_milliseconds: completed.summary.timings.into(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonComparison {
    against: String,
    #[serde(flatten)]
    comparison: ReviewComparison,
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

impl From<ReviewTimings> for JsonTimings {
    fn from(timings: ReviewTimings) -> Self {
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

#[derive(Debug)]
pub(super) enum ReviewError {
    Plan(ReviewPlanError),
    Report(ReviewReportError),
    Artifact(FrameArtifactError),
    ArtifactCount { regions: usize, artifacts: usize },
    CheckpointCount { planned: usize, captured: usize },
}

impl fmt::Display for ReviewError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan(source) => source.fmt(formatter),
            Self::Report(source) => source.fmt(formatter),
            Self::Artifact(source) => source.fmt(formatter),
            Self::ArtifactCount { regions, artifacts } => write!(
                formatter,
                "exact review has {regions} regions but {artifacts} frame artifacts",
            ),
            Self::CheckpointCount { planned, captured } => write!(
                formatter,
                "exact review planned {planned} checkpoints but read {captured} frames",
            ),
        }
    }
}

impl Error for ReviewError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Plan(source) => Some(source),
            Self::Report(source) => Some(source),
            Self::Artifact(source) => Some(source),
            Self::ArtifactCount { .. } | Self::CheckpointCount { .. } => None,
        }
    }
}

impl From<ReviewPlanError> for ReviewError {
    fn from(source: ReviewPlanError) -> Self {
        Self::Plan(source)
    }
}

impl From<ReviewReportError> for ReviewError {
    fn from(source: ReviewReportError) -> Self {
        Self::Report(source)
    }
}

impl From<FrameArtifactError> for ReviewError {
    fn from(source: FrameArtifactError) -> Self {
        Self::Artifact(source)
    }
}
