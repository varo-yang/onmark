//! Browser-free validation of one authored film through render-unit planning.
//!
//! The command follows the production compiler, asset, bundler, and planner
//! path. It stops before unit-root materialization, Chromium, and encoding.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use onmark_core::compiler;
use onmark_core::compiler::ResolvedFilm;
use onmark_core::diagnostics::Diagnostic;
use onmark_core::model::{ImportedCaptionTrack, Timebase};
use onmark_core::render_graph::{PartitionPlan, RenderGraph};
use onmark_core::timeline::TimelineIr;
use onmark_media::Ffprobe;
use onmark_render::{RenderProfile, RenderUnit};
use serde::Serialize;

use crate::arguments::{CheckArgs, source_directory};
use crate::assets::FrozenCatalog;
use crate::bundler::{BundleRegion, PresentationBundler};
use crate::captions::CaptionImport;
use crate::compilation;
use crate::diagnostic::{self, AuthoredReport, JsonDiagnostic};
use crate::environment::CheckExecutables;
use crate::execution;
use crate::failure::CliError;
use crate::input;
use crate::variant::VariantImport;

const REPORT_VERSION: u16 = 1;

pub(super) struct Validation {
    pub(super) report: AuthoredReport,
    pub(super) inspection: Option<Inspection>,
}

pub(super) struct Inspection {
    pub(super) timeline: TimelineIr,
    pub(super) assets: usize,
    pub(super) regions: Vec<RegionInspection>,
}

pub(super) struct RegionInspection {
    pub(super) evaluation_start: u64,
    pub(super) evaluation_end: u64,
    pub(super) output_start: u64,
    pub(super) output_end: u64,
    pub(super) visual_mode: &'static str,
    pub(super) capture_cadence: &'static str,
    pub(super) native_media: usize,
    pub(super) variant_fields: Vec<String>,
    pub(super) bundle_id: Box<str>,
}

struct ResolvedInput {
    args: crate::arguments::ValidationArgs,
    source: String,
    film: ResolvedFilm,
    diagnostics: Vec<Diagnostic>,
    caption_tracks: Vec<ImportedCaptionTrack>,
}

enum InitialValidation {
    Rejected(Validation),
    Resolved(ResolvedInput),
}

pub(super) struct CheckOutcome {
    validation: Validation,
    json: bool,
}

impl CheckOutcome {
    pub(super) fn write(self) -> ExitCode {
        let exit_code = if self.validation.inspection.is_some() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
        let result = write_result(&self.validation, self.json).map(|()| exit_code);
        result.unwrap_or(ExitCode::FAILURE)
    }
}

pub(super) async fn run(args: CheckArgs, json: bool) -> Result<CheckOutcome, CliError> {
    let validation = validate(args.validation).await?;
    Ok(CheckOutcome { validation, json })
}

pub(super) async fn validate(
    args: crate::arguments::ValidationArgs,
) -> Result<Validation, CliError> {
    match resolve_input(args)? {
        InitialValidation::Rejected(validation) => Ok(validation),
        InitialValidation::Resolved(input) => validate_resolved(input).await,
    }
}

fn resolve_input(args: crate::arguments::ValidationArgs) -> Result<InitialValidation, CliError> {
    let source = input::read_utf8(
        &args.screenplay,
        u64::try_from(onmark_core::syntax::MAX_SCREENPLAY_BYTES)
            .expect("the screenplay byte limit fits in u64"),
    )
    .map_err(|error| CliError::read_screenplay(&args.screenplay, error))?;

    let resolved = compilation::resolve(&source);
    let (film, diagnostics) = resolved.into_parts();
    let Some(film) = film else {
        return Ok(InitialValidation::Rejected(Validation {
            report: authored_report(args.screenplay, source, diagnostics),
            inspection: None,
        }));
    };
    let film = match VariantImport::apply(args.variant.as_deref(), film)? {
        VariantImport::Film(film) => *film,
        VariantImport::Rejected(rejected) => {
            let (path, source, diagnostics) = rejected.into_parts();
            return Ok(InitialValidation::Rejected(Validation {
                report: authored_report(path, source, diagnostics),
                inspection: None,
            }));
        }
    };
    let caption_tracks = match CaptionImport::load(
        film.captions(),
        &args.caption_tracks,
        source_directory(&args.screenplay),
    )? {
        CaptionImport::Ready(tracks) => tracks,
        CaptionImport::Rejected(rejected) => {
            let (path, source, diagnostics) = rejected.into_parts();
            return Ok(InitialValidation::Rejected(Validation {
                report: authored_report(path, source, diagnostics),
                inspection: None,
            }));
        }
    };

    Ok(InitialValidation::Resolved(ResolvedInput {
        args,
        source,
        film,
        diagnostics,
        caption_tracks,
    }))
}

async fn validate_resolved(input: ResolvedInput) -> Result<Validation, CliError> {
    let ResolvedInput {
        args,
        source,
        film,
        diagnostics,
        caption_tracks,
    } = input;
    let executables = CheckExecutables::discover(&args)?;
    let probe = Ffprobe::new(
        executables.ffprobe,
        execution::process_deadline(),
        Ffprobe::MAX_OUTPUT_BYTES,
    )
    .expect("the CLI probe policy stays within the media safety envelope");
    let frozen = FrozenCatalog::freeze(&film, source_directory(&args.screenplay), &probe).await?;
    let asset_count = frozen.facts().len();
    let solved = compilation::solve(
        film,
        frozen.facts(),
        Timebase::new(args.frame_rate),
        diagnostics,
    )?;
    let (timeline, diagnostics) = solved.into_parts();
    let Some(timeline) = timeline else {
        return Ok(Validation {
            report: authored_report(args.screenplay, source, diagnostics),
            inspection: None,
        });
    };
    let timeline = compiler::import_captions(timeline, caption_tracks)?;

    let bundler = PresentationBundler::new(executables.bundler);
    let partitions =
        RenderGraph::from_timeline(&timeline, PresentationBundler::temporal_capability())?
            .into_partition();
    let bundle = bundler
        .bundle(&source, source_directory(&args.screenplay), &partitions)
        .await?;
    let manifests = (0..partitions.units().len())
        .map(|index| bundle.region(index))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(BundleRegion::into_parts)
        .map(|(_, manifest)| manifest)
        .collect();
    let materialized = frozen.into_materialized()?;
    let units = RenderUnit::from_partitioned_bundles(
        &timeline,
        &partitions,
        manifests,
        RenderProfile::new(args.width, args.height)?,
        materialized.assets().iter().cloned(),
    )?;
    let regions = inspect_regions(&partitions, &units);

    Ok(Validation {
        report: authored_report(args.screenplay, source, diagnostics),
        inspection: Some(Inspection {
            timeline,
            assets: asset_count,
            regions,
        }),
    })
}

fn inspect_regions(partitions: &PartitionPlan, units: &[RenderUnit]) -> Vec<RegionInspection> {
    partitions
        .units()
        .iter()
        .zip(units)
        .map(|(partition, unit)| RegionInspection {
            evaluation_start: partition.evaluation().start().get(),
            evaluation_end: partition.evaluation().end().get(),
            output_start: partition.output().start().get(),
            output_end: partition.output().end().get(),
            visual_mode: unit.visual_execution().capability().as_str(),
            capture_cadence: match unit.visual_execution().capture_cadence() {
                onmark_render::BrowserCaptureCadence::EveryFrame => "everyFrame",
                onmark_render::BrowserCaptureCadence::PlacementBounded => "placementBounded",
            },
            native_media: unit.visual_execution().native_media_count(),
            variant_fields: partition
                .variant_fields()
                .map(ToString::to_string)
                .collect(),
            bundle_id: unit.bundle_id().into(),
        })
        .collect()
}

fn authored_report(path: PathBuf, source: String, diagnostics: Vec<Diagnostic>) -> AuthoredReport {
    AuthoredReport {
        path,
        source,
        diagnostics,
    }
}

fn write_result(validation: &Validation, json: bool) -> io::Result<()> {
    if json {
        return write_json(validation);
    }

    let report = &validation.report;
    let mut stderr = io::stderr().lock();
    diagnostic::write_all(
        &mut stderr,
        &report.path,
        &report.source,
        &report.diagnostics,
    )?;
    drop(stderr);

    if let Some(inspection) = &validation.inspection {
        let timeline = &inspection.timeline;
        let frame_rate = timeline.timebase().frame_rate();
        let mut stdout = io::stdout().lock();
        writeln!(
            stdout,
            "Checked {} frames at {}/{} fps across {} render regions and {} frozen assets",
            timeline.interval().len().get(),
            frame_rate.numerator(),
            frame_rate.denominator(),
            inspection.regions.len(),
            inspection.assets,
        )?;
    }
    Ok(())
}

fn write_json(validation: &Validation) -> io::Result<()> {
    let report = &validation.report;
    let document = JsonCheckReport {
        version: REPORT_VERSION,
        command: "check",
        valid: validation.inspection.is_some(),
        source: report.path.display().to_string(),
        diagnostics: report
            .diagnostics
            .iter()
            .map(JsonDiagnostic::from)
            .collect(),
        summary: validation.inspection.as_ref().map(JsonCheckSummary::from),
    };
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &document)?;
    writeln!(stdout)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonCheckReport<'a> {
    version: u16,
    command: &'static str,
    valid: bool,
    source: String,
    diagnostics: Vec<JsonDiagnostic<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<JsonCheckSummary>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonCheckSummary {
    frame_rate_numerator: u32,
    frame_rate_denominator: u32,
    frames: u64,
    assets: usize,
    render_regions: usize,
}

impl From<&Inspection> for JsonCheckSummary {
    fn from(inspection: &Inspection) -> Self {
        let timeline = &inspection.timeline;
        let frame_rate = timeline.timebase().frame_rate();
        Self {
            frame_rate_numerator: frame_rate.numerator(),
            frame_rate_denominator: frame_rate.denominator(),
            frames: timeline.interval().len().get(),
            assets: inspection.assets,
            render_regions: inspection.regions.len(),
        }
    }
}
