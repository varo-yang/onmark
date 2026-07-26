//! Deterministic explanation of solved and planned film facts.

use std::io::{self, Write};
use std::process::ExitCode;

use serde::Serialize;

use crate::arguments::InspectArgs;
use crate::check::{Inspection, RegionInspection, Validation};
use crate::diagnostic::{self, JsonDiagnostic};
use crate::failure::CliError;

const REPORT_VERSION: u16 = 1;

pub(super) struct InspectOutcome {
    validation: Validation,
    json: bool,
}

impl InspectOutcome {
    pub(super) fn write(self) -> ExitCode {
        let exit_code = if self.validation.inspection.is_some() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
        let result = if self.json {
            write_json(&self.validation)
        } else {
            write_human(&self.validation)
        };
        result.map_or(ExitCode::FAILURE, |()| exit_code)
    }
}

pub(super) async fn run(args: InspectArgs, json: bool) -> Result<InspectOutcome, CliError> {
    let validation = crate::check::validate(args.validation).await?;
    Ok(InspectOutcome { validation, json })
}

fn write_human(validation: &Validation) -> io::Result<()> {
    let report = &validation.report;
    let mut stderr = io::stderr().lock();
    diagnostic::write_all(
        &mut stderr,
        &report.path,
        &report.source,
        &report.diagnostics,
    )?;
    drop(stderr);

    let Some(inspection) = &validation.inspection else {
        return Ok(());
    };
    let mut stdout = io::stdout().lock();
    writeln!(
        stdout,
        "Timeline {}..{} at {}/{} fps",
        inspection.interval_start,
        inspection.interval_end,
        inspection.frame_rate.numerator(),
        inspection.frame_rate.denominator(),
    )?;
    writeln!(
        stdout,
        "Structure: {} scenes, {} shots, {} videos, {} overlays",
        inspection.scenes, inspection.shots, inspection.videos, inspection.overlays,
    )?;
    writeln!(
        stdout,
        "Media: {} audio placements, {} captions, {} cues, {} frozen assets",
        inspection.audio, inspection.captions, inspection.cues, inspection.assets,
    )?;
    for (index, region) in inspection.regions.iter().enumerate() {
        writeln!(
            stdout,
            "Region {index}: evaluate {}..{}, output {}..{}, {}, {}",
            region.evaluation_start,
            region.evaluation_end,
            region.output_start,
            region.output_end,
            region.visual_mode,
            region.capture_cadence,
        )?;
    }
    Ok(())
}

fn write_json(validation: &Validation) -> io::Result<()> {
    let report = &validation.report;
    let document = InspectReport {
        version: REPORT_VERSION,
        command: "inspect",
        valid: validation.inspection.is_some(),
        source: report.path.display().to_string(),
        diagnostics: report
            .diagnostics
            .iter()
            .map(JsonDiagnostic::from)
            .collect(),
        inspection: validation.inspection.as_ref().map(JsonInspection::from),
    };
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &document)?;
    writeln!(stdout)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectReport<'a> {
    version: u16,
    command: &'static str,
    valid: bool,
    source: String,
    diagnostics: Vec<JsonDiagnostic<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inspection: Option<JsonInspection<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonInspection<'a> {
    frame_rate_numerator: u32,
    frame_rate_denominator: u32,
    interval_start: u64,
    interval_end: u64,
    assets: usize,
    scenes: usize,
    shots: usize,
    videos: usize,
    overlays: usize,
    audio: usize,
    captions: usize,
    cues: usize,
    regions: Vec<JsonRegion<'a>>,
}

impl<'a> From<&'a Inspection> for JsonInspection<'a> {
    fn from(inspection: &'a Inspection) -> Self {
        Self {
            frame_rate_numerator: inspection.frame_rate.numerator(),
            frame_rate_denominator: inspection.frame_rate.denominator(),
            interval_start: inspection.interval_start,
            interval_end: inspection.interval_end,
            assets: inspection.assets,
            scenes: inspection.scenes,
            shots: inspection.shots,
            videos: inspection.videos,
            overlays: inspection.overlays,
            audio: inspection.audio,
            captions: inspection.captions,
            cues: inspection.cues,
            regions: inspection.regions.iter().map(JsonRegion::from).collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonRegion<'a> {
    evaluation_start: u64,
    evaluation_end: u64,
    output_start: u64,
    output_end: u64,
    visual_mode: &'a str,
    capture_cadence: &'a str,
}

impl<'a> From<&'a RegionInspection> for JsonRegion<'a> {
    fn from(region: &'a RegionInspection) -> Self {
        Self {
            evaluation_start: region.evaluation_start,
            evaluation_end: region.evaluation_end,
            output_start: region.output_start,
            output_end: region.output_end,
            visual_mode: region.visual_mode,
            capture_cadence: region.capture_cadence,
        }
    }
}
