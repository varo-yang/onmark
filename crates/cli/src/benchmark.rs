//! Repeatable complete-render measurement with phase-level medians.

use std::io::{self, Write};
use std::process::ExitCode;
use std::time::Duration;

use serde::Serialize;

use crate::arguments::BenchmarkArgs;
use crate::diagnostic::{self, JsonDiagnostic};
use crate::failure::CliError;
use crate::progress::Progress;
use crate::render::{
    AuthoredReport, BenchmarkAttempt, BenchmarkSample, RenderTimings, run_uncached,
};

const REPORT_VERSION: u16 = 1;

pub(super) struct BenchmarkOutcome {
    report: AuthoredReport,
    measurements: Option<BenchmarkMeasurements>,
    json: bool,
}

impl BenchmarkOutcome {
    pub(super) fn write(self) -> ExitCode {
        let valid = self.measurements.is_some();
        let result = if self.json {
            write_json(&self.report, self.measurements.as_ref())
        } else {
            write_human(&self.report, self.measurements.as_ref())
        };
        result.map_or(ExitCode::FAILURE, |()| {
            if valid {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        })
    }
}

struct BenchmarkMeasurements {
    samples: Vec<BenchmarkSample>,
}

impl BenchmarkMeasurements {
    fn new(first: BenchmarkSample) -> Self {
        Self {
            samples: vec![first],
        }
    }

    fn first(&self) -> &BenchmarkSample {
        &self.samples[0]
    }

    fn push(&mut self, sample: BenchmarkSample) {
        self.samples.push(sample);
    }

    fn as_slice(&self) -> &[BenchmarkSample] {
        &self.samples
    }
}

pub(super) async fn run(args: BenchmarkArgs, json: bool) -> Result<BenchmarkOutcome, CliError> {
    let workspace = tempfile::tempdir().map_err(CliError::benchmark_workspace)?;
    let progress = Progress::for_command(json);
    progress.sample(1, args.runs)?;
    let first = run_uncached(
        args.render_args(workspace.path().join("sample-0.mp4")),
        true,
    )
    .await?
    .into_benchmark_attempt();
    let (report, first) = match first {
        BenchmarkAttempt::Completed { report, sample } => (report, sample),
        BenchmarkAttempt::Rejected(report) => {
            return Ok(BenchmarkOutcome {
                report,
                measurements: None,
                json,
            });
        }
    };
    let mut measurements = BenchmarkMeasurements::new(first);

    for index in 1..args.runs {
        progress.sample(index + 1, args.runs)?;
        let output = workspace.path().join(format!("sample-{index}.mp4"));
        let attempt = run_uncached(args.render_args(output), true)
            .await?
            .into_benchmark_attempt();
        match attempt {
            BenchmarkAttempt::Rejected(rejected) => {
                return Ok(BenchmarkOutcome {
                    report: rejected,
                    measurements: None,
                    json,
                });
            }
            BenchmarkAttempt::Completed { sample, .. } => {
                verify_stable_sample(measurements.first(), &sample)?;
                measurements.push(sample);
            }
        }
    }

    Ok(BenchmarkOutcome {
        report,
        measurements: Some(measurements),
        json,
    })
}

fn verify_stable_sample(
    expected: &BenchmarkSample,
    actual: &BenchmarkSample,
) -> Result<(), CliError> {
    if expected.frames != actual.frames {
        return Err(CliError::benchmark_drift("frame count"));
    }
    if expected.capture_mode != actual.capture_mode {
        return Err(CliError::benchmark_drift("capture mode"));
    }
    if expected.graphics_backend != actual.graphics_backend {
        return Err(CliError::benchmark_drift("graphics backend"));
    }
    if expected.encode_profile != actual.encode_profile {
        return Err(CliError::benchmark_drift("output profile"));
    }
    Ok(())
}

fn write_human(
    report: &AuthoredReport,
    measurements: Option<&BenchmarkMeasurements>,
) -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    diagnostic::write_all(
        &mut stderr,
        &report.path,
        &report.source,
        &report.diagnostics,
    )?;
    drop(stderr);

    let Some(measurements) = measurements else {
        return Ok(());
    };
    let samples = measurements.as_slice();
    let first = measurements.first();
    let mut stdout = io::stdout().lock();
    writeln!(
        stdout,
        "{} complete uncached renders, {} frames, {}, {}, {}",
        samples.len(),
        first.frames,
        first.capture_mode,
        first.graphics_backend,
        first.encode_profile.as_str(),
    )?;
    for (index, sample) in samples.iter().enumerate() {
        writeln!(stdout, "Sample {}: {}", index + 1, sample.timings)?;
    }
    writeln!(stdout, "Median: {}", median_timings(measurements))
}

fn write_json(
    report: &AuthoredReport,
    measurements: Option<&BenchmarkMeasurements>,
) -> io::Result<()> {
    let document = JsonBenchmarkReport {
        version: REPORT_VERSION,
        command: "benchmark",
        valid: measurements.is_some(),
        source: report.path.display().to_string(),
        diagnostics: report
            .diagnostics
            .iter()
            .map(JsonDiagnostic::from)
            .collect(),
        benchmark: measurements.map(JsonBenchmark::from),
    };
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &document)?;
    writeln!(stdout)
}

fn median_timings(measurements: &BenchmarkMeasurements) -> RenderTimings {
    RenderTimings {
        prepare: median(measurements, |sample| sample.timings.prepare),
        bundle: median(measurements, |sample| sample.timings.bundle),
        plan: median(measurements, |sample| sample.timings.plan),
        capture: median(measurements, |sample| sample.timings.capture),
        assemble: median(measurements, |sample| sample.timings.assemble),
        total: median(measurements, |sample| sample.timings.total),
    }
}

fn median(
    measurements: &BenchmarkMeasurements,
    select: impl Fn(&BenchmarkSample) -> Duration,
) -> Duration {
    let mut values = measurements
        .as_slice()
        .iter()
        .map(select)
        .collect::<Vec<_>>();
    values.sort_unstable();
    values[values.len() / 2]
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonBenchmarkReport<'a> {
    version: u16,
    command: &'static str,
    valid: bool,
    source: String,
    diagnostics: Vec<JsonDiagnostic<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    benchmark: Option<JsonBenchmark>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonBenchmark {
    uncached: bool,
    runs: usize,
    frames: u64,
    capture_mode: String,
    graphics_backend: String,
    output_profile: &'static str,
    samples: Vec<JsonTimings>,
    median: JsonTimings,
}

impl From<&BenchmarkMeasurements> for JsonBenchmark {
    fn from(measurements: &BenchmarkMeasurements) -> Self {
        let samples = measurements.as_slice();
        let first = measurements.first();
        Self {
            uncached: true,
            runs: samples.len(),
            frames: first.frames,
            capture_mode: first.capture_mode.to_string(),
            graphics_backend: first.graphics_backend.to_string(),
            output_profile: first.encode_profile.as_str(),
            samples: samples.iter().map(|sample| sample.timings.into()).collect(),
            median: median_timings(measurements).into(),
        }
    }
}

#[derive(Serialize)]
struct JsonTimings {
    prepare: u128,
    bundle: u128,
    plan: u128,
    capture: u128,
    assemble: u128,
    total: u128,
}

impl From<RenderTimings> for JsonTimings {
    fn from(timings: RenderTimings) -> Self {
        Self {
            prepare: timings.prepare.as_millis(),
            bundle: timings.bundle.as_millis(),
            plan: timings.plan.as_millis(),
            capture: timings.capture.as_millis(),
            assemble: timings.assemble.as_millis(),
            total: timings.total.as_millis(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use onmark_render::{BrowserCaptureMode, BrowserGraphicsBackend, EncodeProfile};

    use super::{BenchmarkMeasurements, BenchmarkSample, median_timings};
    use crate::render::RenderTimings;

    #[test]
    fn reports_the_middle_complete_render_sample() {
        let measurements = BenchmarkMeasurements {
            samples: vec![sample(30), sample(10), sample(20)],
        };

        assert_eq!(
            median_timings(&measurements).total,
            Duration::from_millis(20),
        );
    }

    fn sample(milliseconds: u64) -> BenchmarkSample {
        let elapsed = Duration::from_millis(milliseconds);
        BenchmarkSample {
            frames: 30,
            capture_mode: BrowserCaptureMode::Screenshot,
            graphics_backend: BrowserGraphicsBackend::SwiftShader,
            encode_profile: EncodeProfile::H264Mp4,
            timings: RenderTimings {
                prepare: elapsed,
                bundle: elapsed,
                plan: elapsed,
                capture: elapsed,
                assemble: elapsed,
                total: elapsed,
            },
        }
    }
}
