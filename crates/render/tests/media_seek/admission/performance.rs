//! Self-adjudicating performance evidence for the layered-media candidate.

use std::env;
use std::fs::File;
use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use onmark_core::model::{FrameRate, FrozenAssetId};
use onmark_core::protocol::{BrowserPlan, RequestId, WireFrameRate};
use onmark_render::{BrowserSession, CaptureEnvironmentId, EncodeLimits, Ffmpeg, FfmpegSession};
use sha2::{Digest as _, Sha256};
use url::Url;

use super::{capture_layered_sequence, request_offset};
use crate::layered::LayeredOutput;
use crate::measurement::{Measurement, measure};
use crate::{
    BENCHMARK_FRAME_COUNT, PROCESS_DEADLINE, StrategyFixture, confirm, dispose,
    experiment_dimensions, frame, launch_browser, load_and_prepare, required_path, stage,
    transparent_overlay_fixture,
};

const ADMISSION_RUNS: usize = 5;
const ADMISSION_WIDTH: u32 = 1_920;
const ADMISSION_HEIGHT: u32 = 1_080;
const CAPTURE_ENVIRONMENT: &str = "ONMARK_CAPTURE_ENVIRONMENT";

#[tokio::test]
#[ignore = "requires pinned Linux tools, ONMARK_CAPTURE_ENVIRONMENT, and 1920x1080 experiment dimensions"]
async fn meets_performance_thresholds() {
    assert_eq!(
        env::consts::OS,
        "linux",
        "performance admission is Linux-only"
    );
    assert_eq!(
        experiment_dimensions(),
        (ADMISSION_WIDTH, ADMISSION_HEIGHT),
        "performance admission requires the frozen 1920x1080 profile",
    );
    let environment = admission_environment();

    let fixture = StrategyFixture::build().await;
    let fixture_id = fixture_identity(&fixture.media);
    let browser_presentation = fixture.native_fixture();
    let layered_presentation = transparent_overlay_fixture();
    let mut browser_runs = Vec::with_capacity(ADMISSION_RUNS);
    let mut layered_runs = Vec::with_capacity(ADMISSION_RUNS);

    for run in 0..ADMISSION_RUNS {
        let measured =
            measure_pair(&fixture, &browser_presentation, &layered_presentation, run).await;
        browser_runs.push(measured.browser);
        layered_runs.push(measured.layered);
    }

    let evidence = AdmissionEvidence::from_runs(&browser_runs, &layered_runs);
    assert_repeatability(&browser_runs, &layered_runs);
    report_runs(&browser_runs, &layered_runs);
    evidence.report(environment, fixture_id);
    evidence.assert_performance();
}

async fn measure_pair(
    fixture: &StrategyFixture,
    browser_presentation: &Url,
    layered_presentation: &Url,
    run: usize,
) -> MeasuredPair {
    if run.is_multiple_of(2) {
        let browser = measure_browser(fixture, browser_presentation, run).await;
        let layered = measure_layered(fixture, layered_presentation, run).await;
        MeasuredPair { browser, layered }
    } else {
        let layered = measure_layered(fixture, layered_presentation, run).await;
        let browser = measure_browser(fixture, browser_presentation, run).await;
        MeasuredPair { browser, layered }
    }
}

struct MeasuredPair {
    browser: Measurement<BrowserEncodedOutput>,
    layered: Measurement<LayeredOutput>,
}

async fn measure_browser(
    fixture: &StrategyFixture,
    presentation: &Url,
    run: usize,
) -> Measurement<BrowserEncodedOutput> {
    measure(capture_encoded_browser_sequence(
        fixture,
        presentation,
        &format!("admission-browser-{run}.mp4"),
    ))
    .await
}

async fn measure_layered(
    fixture: &StrategyFixture,
    presentation: &Url,
    run: usize,
) -> Measurement<LayeredOutput> {
    measure(capture_layered_sequence(
        fixture,
        presentation,
        &format!("admission-layered-{run}.mp4"),
    ))
    .await
}

struct BrowserEncodedOutput {
    fingerprints: Vec<[u8; 32]>,
}

async fn capture_encoded_browser_sequence(
    fixture: &StrategyFixture,
    presentation: &Url,
    output_name: &str,
) -> BrowserEncodedOutput {
    let output = fixture.root().join(output_name);
    let mut encoder = start_browser_encoder(fixture.frame_rate, &output);
    let mut session = launch_browser().await;
    load_and_prepare(&session, presentation, &fixture.plan)
        .await
        .expect("the browser-media presentation must prepare");

    let fingerprints =
        capture_browser_frames(&mut session, &mut encoder, &fixture.plan, &fixture.indices).await;
    dispose(&session, fixture.indices.len())
        .await
        .expect("the browser-media presentation must dispose");
    session
        .shutdown()
        .await
        .expect("headless shell must shut down after browser-media capture");
    let encoded = encoder
        .finish()
        .await
        .expect("the browser-media encoder must finish");
    assert_encoded_output(encoded.path());

    BrowserEncodedOutput { fingerprints }
}

async fn capture_browser_frames(
    session: &mut BrowserSession,
    encoder: &mut FfmpegSession,
    plan: &BrowserPlan,
    indices: &[u64],
) -> Vec<[u8; 32]> {
    let mut fingerprints = Vec::with_capacity(indices.len());
    for (offset, index) in indices.iter().copied().enumerate() {
        let request_offset = request_offset(offset);
        stage(session, RequestId::new(3 + request_offset), index)
            .await
            .expect("the browser-media frame must stage");
        let captured = session
            .capture_frame(frame(index), plan.frame_rate())
            .await
            .expect("the browser-media frame must capture");
        encoder
            .write_frame(captured.png())
            .await
            .expect("the browser-media frame must encode");
        confirm(session, RequestId::new(4 + request_offset), index)
            .await
            .expect("the browser-media frame must confirm");
        fingerprints.push(*captured.raw_rgba_hash().as_bytes());
    }
    fingerprints
}

fn start_browser_encoder(frame_rate: FrameRate, output: &Path) -> FfmpegSession {
    let ffmpeg = Ffmpeg::new(required_path("ONMARK_FFMPEG"), encoder_limits())
        .expect("the benchmark encoder must be valid");
    ffmpeg
        .start(output, WireFrameRate::from(frame_rate))
        .expect("the benchmark encoder must start")
}

fn encoder_limits() -> EncodeLimits {
    EncodeLimits::new(PROCESS_DEADLINE, BENCHMARK_FRAME_COUNT, 1 << 30, 64 * 1024)
        .expect("the benchmark encoder limits are bounded")
}

fn assert_encoded_output(output: &Path) {
    let bytes = std::fs::metadata(output)
        .expect("the measured encoder must publish its output")
        .len();
    assert!(bytes > 0, "the measured encoded output cannot be empty");
}

struct AdmissionEvidence {
    browser_time: Duration,
    layered_time: Duration,
    browser_rss_kib: u64,
    layered_rss_kib: u64,
}

impl AdmissionEvidence {
    fn from_runs(
        browser: &[Measurement<BrowserEncodedOutput>],
        layered: &[Measurement<LayeredOutput>],
    ) -> Self {
        Self {
            browser_time: median_duration(browser.iter().map(|run| run.elapsed)),
            layered_time: median_duration(layered.iter().map(|run| run.elapsed)),
            browser_rss_kib: median_peak_rss(browser),
            layered_rss_kib: median_peak_rss(layered),
        }
    }

    fn assert_performance(&self) {
        assert!(
            self.layered_time.saturating_mul(2) <= self.browser_time,
            "layered median wall time must be at least twice as fast as the baseline",
        );
        assert!(
            u128::from(self.layered_rss_kib) * 100 <= u128::from(self.browser_rss_kib) * 85,
            "layered median peak RSS must not exceed 85% of the baseline",
        );
    }

    fn report(&self, environment: CaptureEnvironmentId, fixture: FrozenAssetId) {
        println!(
            concat!(
                "layered-admission:\n",
                "  environment: {environment}\n",
                "  fixture: {fixture}\n",
                "  runs: {runs}\n",
                "  browser-ms: {browser_ms:.2}\n",
                "  layered-ms: {layered_ms:.2}\n",
                "  browser-peak-kib: {browser_peak_kib}\n",
                "  layered-peak-kib: {layered_peak_kib}",
            ),
            environment = environment,
            fixture = fixture,
            runs = ADMISSION_RUNS,
            browser_ms = self.browser_time.as_secs_f64() * 1_000.0,
            layered_ms = self.layered_time.as_secs_f64() * 1_000.0,
            browser_peak_kib = self.browser_rss_kib,
            layered_peak_kib = self.layered_rss_kib,
        );
    }
}

fn report_runs(
    browser: &[Measurement<BrowserEncodedOutput>],
    layered: &[Measurement<LayeredOutput>],
) {
    for (index, (browser, layered)) in browser.iter().zip(layered).enumerate() {
        println!(
            concat!(
                "layered-admission-run-{index}:\n",
                "  browser-ms: {browser_ms:.2}\n",
                "  layered-ms: {layered_ms:.2}\n",
                "  browser-peak-kib: {browser_peak_kib}\n",
                "  layered-peak-kib: {layered_peak_kib}",
            ),
            index = index,
            browser_ms = browser.elapsed.as_secs_f64() * 1_000.0,
            layered_ms = layered.elapsed.as_secs_f64() * 1_000.0,
            browser_peak_kib = browser.incremental_peak_rss_kib(),
            layered_peak_kib = layered.incremental_peak_rss_kib(),
        );
    }
}

fn admission_environment() -> CaptureEnvironmentId {
    let value = env::var(CAPTURE_ENVIRONMENT)
        .unwrap_or_else(|_| panic!("{CAPTURE_ENVIRONMENT} must name the locked toolchain"));
    CaptureEnvironmentId::parse(&value)
        .expect("the admission capture-environment identity must be canonical")
}

fn fixture_identity(path: &Path) -> FrozenAssetId {
    let mut source = File::open(path).expect("the admission fixture must remain readable");
    let mut hasher = Sha256::new();
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let count = source
            .read(&mut chunk)
            .expect("the admission fixture must remain readable");
        if count == 0 {
            return FrozenAssetId::from_sha256(hasher.finalize().into());
        }
        hasher.update(&chunk[..count]);
    }
}

fn assert_repeatability(
    browser: &[Measurement<BrowserEncodedOutput>],
    layered: &[Measurement<LayeredOutput>],
) {
    assert_one_sequence(
        browser.iter().map(|run| run.output.fingerprints.as_slice()),
        "browser-media baseline",
    );
    assert_one_sequence(
        layered.iter().map(|run| run.output.fingerprints.as_slice()),
        "layered-media candidate",
    );
}

fn assert_one_sequence<'a>(mut sequences: impl Iterator<Item = &'a [[u8; 32]]>, path: &str) {
    let expected = sequences
        .next()
        .expect("admission requires at least one measured run");
    for actual in sequences {
        assert_eq!(actual, expected, "{path} must repeat every canonical frame");
    }
}

fn median_duration(values: impl Iterator<Item = Duration>) -> Duration {
    let mut values = values.collect::<Vec<_>>();
    values.sort_unstable();
    values[values.len() / 2]
}

fn median_peak_rss<T>(measurements: &[Measurement<T>]) -> u64 {
    median_u64(
        measurements
            .iter()
            .map(Measurement::incremental_peak_rss_kib),
    )
}

fn median_u64(values: impl Iterator<Item = u64>) -> u64 {
    let mut values = values.collect::<Vec<_>>();
    values.sort_unstable();
    values[values.len() / 2]
}
