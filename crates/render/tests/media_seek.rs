//! Layered-media admission and retained strategy evidence across Chromium and
//! `FFmpeg`.
//!
//! The admission modules enforce Gate seven's frame-selection, color, partition,
//! and pinned-performance claims. The broader opt-in comparison remains here as
//! reproducible evidence for the strategy decision; production rendering lives
//! in the crate rather than importing this harness.

#[path = "media_seek/admission.rs"]
mod admission;
#[path = "media_seek/decoder.rs"]
mod decoder;
#[path = "media_seek/layered.rs"]
mod layered;
#[path = "media_seek/measurement.rs"]
mod measurement;
#[path = "media_seek/pixels.rs"]
mod pixels;

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use onmark_core::compiler;
use onmark_core::model::{
    AssetMetadata, AssetRef, Duration as MediaDuration, FrameRate, FrozenAsset, FrozenAssetId,
    SourceId, Timebase, VideoColorProfile, VideoDimensions, VideoMetadata, VideoTiming,
};
use onmark_core::protocol::{
    BrowserCommand, BrowserEvent, BrowserPlan, BrowserRequest, RequestId, WireFrame,
};
use onmark_media::Ffprobe;
use onmark_render::{
    AdmittedVideo, BrowserCaptureMode, BrowserLaunchPolicy, BrowserLimits, BrowserSession,
    EncodedPng, RenderProfile, UnsupportedVideo,
};
use serde::Deserialize;
use tempfile::{Builder as TempDirBuilder, TempDir};
use tokio::process::Command;
use tokio::time::timeout;
use url::Url;

use decoder::{compose_sequence, decode_sequence};
use measurement::{Measurement, measure};
use pixels::{
    PixelDifference, compare_encoded_frames, compare_pixels, composite_pixels,
    decode_browser_frames,
};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const SEEK_SEQUENCE: [u64; 4] = [17, 3, 29, 17];
// Performance covers one two-second sequence; color sampling retains only four
// PNGs so the test does not manufacture the memory growth it is measuring.
const COLOR_SAMPLE_SEQUENCE: [u64; 4] = [0, 17, 29, 59];
const BENCHMARK_FRAME_COUNT: u64 = 60;
const PROCESS_DEADLINE: Duration = Duration::from_secs(20);
const MAX_REFERENCE_BYTES: usize = 8 * 1024 * 1024;
const WIDTH_ENVIRONMENT: &str = "ONMARK_MEDIA_EXPERIMENT_WIDTH";
const HEIGHT_ENVIRONMENT: &str = "ONMARK_MEDIA_EXPERIMENT_HEIGHT";

#[tokio::test]
#[ignore = "requires ONMARK_HEADLESS_SHELL, ONMARK_FFMPEG, ONMARK_FFPROBE, and built runtime"]
async fn validates_admission_and_cfr_decode_paths() {
    run_case(
        "cfr-30",
        FrameRate::new(30, 1).expect("the CFR fixture rate is valid"),
        FixtureTiming::Constant,
    )
    .await;
    run_case(
        "cfr-ntsc",
        FrameRate::new(30_000, 1_001).expect("the NTSC fixture rate is valid"),
        FixtureTiming::Constant,
    )
    .await;
    run_case(
        "vfr-30",
        FrameRate::new(30, 1).expect("the VFR output rate is valid"),
        FixtureTiming::AlternatingVfr,
    )
    .await;
}

#[tokio::test]
#[ignore = "requires Linux ONMARK_HEADLESS_SHELL, ONMARK_FFMPEG, and built runtime"]
async fn compares_native_seek_with_alternative_media_paths() {
    let fixture = StrategyFixture::build().await;
    let measurements = measure_strategies(&fixture).await;
    let evidence = compare_strategies(&fixture, &measurements).await;

    report_strategies(&measurements, &evidence);
}

struct StrategyFixture {
    directory: TempDir,
    frame_rate: FrameRate,
    source_frame_rate: FrameRate,
    color_profile: VideoColorProfile,
    indices: Vec<u64>,
    media: PathBuf,
    expected: BTreeMap<u64, ExpectedFrame>,
    plan: BrowserPlan,
}

impl StrategyFixture {
    async fn build() -> Self {
        let directory = experiment_directory("benchmark");
        let frame_rate = FrameRate::new(30, 1).expect("the benchmark rate is valid");
        let indices = (0..BENCHMARK_FRAME_COUNT).collect::<Vec<_>>();
        let media = directory.path().join("benchmark.mp4");
        generate_video(&media, frame_rate, FixtureTiming::Constant).await;
        let admitted = admitted_source_video(&media, FixtureTiming::Constant)
            .await
            .expect("the layered fixture must satisfy media admission");
        assert_eq!(admitted.frame_rate, frame_rate);

        let expected = expected_cfr_frames(frame_rate, &indices);
        let plan = browser_plan(frame_rate);
        Self {
            directory,
            frame_rate,
            source_frame_rate: frame_rate,
            color_profile: admitted.color_profile,
            indices,
            media,
            expected,
            plan,
        }
    }

    fn root(&self) -> &Path {
        self.directory.path()
    }

    fn native_fixture(&self) -> Url {
        decoded_video_fixture(&self.media, &self.expected)
    }
}

struct StrategyMeasurements {
    native: Measurement<MeasuredFrames<EncodedPng>>,
    extraction: Measurement<ExtractedFrameSequence>,
    injected: Measurement<MeasuredFrames<EncodedPng>>,
    continuous: Measurement<Vec<Vec<u8>>>,
    overlay: Measurement<MeasuredFrames<EncodedPng>>,
    composed: Measurement<Vec<Vec<u8>>>,
    overlay_pattern: PathBuf,
    overlay_bytes: u64,
}

async fn measure_strategies(fixture: &StrategyFixture) -> StrategyMeasurements {
    let native_fixture = fixture.native_fixture();
    let native = measure(capture_frame_sequence(
        &native_fixture,
        &fixture.plan,
        &fixture.indices,
        FrameRetention::Discard,
    ))
    .await;
    let extraction_directory = fixture.root().join("frames");
    let extraction = measure(extract_frame_sequence(
        &fixture.media,
        fixture.frame_rate,
        BENCHMARK_FRAME_COUNT,
        &extraction_directory,
    ))
    .await;
    let injected_fixture = injected_video_fixture(&extraction_directory);
    let injected = measure(capture_frame_sequence(
        &injected_fixture,
        &fixture.plan,
        &fixture.indices,
        FrameRetention::Discard,
    ))
    .await;

    let continuous = measure(decode_sequence(
        &fixture.media,
        fixture.frame_rate,
        experiment_dimensions(),
        &[],
    ))
    .await;

    let overlay_fixture = transparent_overlay_fixture();
    let overlay = measure(capture_transparent_frame_sequence(
        &overlay_fixture,
        &fixture.plan,
        &fixture.indices,
        FrameRetention::Retain,
    ))
    .await;
    let overlay_directory = fixture.root().join("overlay");
    let overlay_bytes = write_overlay_sequence(&overlay.output.frames, &overlay_directory);
    let overlay_pattern = overlay_directory.join("frame-%05d.png");
    let composed = measure(compose_sequence(
        &fixture.media,
        &overlay_pattern,
        fixture.frame_rate,
        experiment_dimensions(),
        &[],
    ))
    .await;
    StrategyMeasurements {
        native,
        extraction,
        injected,
        continuous,
        overlay,
        composed,
        overlay_pattern,
        overlay_bytes,
    }
}

struct StrategyEvidence {
    injected: PixelDifference,
    continuous: PixelDifference,
    split_composition: PixelDifference,
    alpha_composition: PixelDifference,
    native_layer_composition: PixelDifference,
    native_pixels: Vec<Vec<u8>>,
    continuous_frames: Vec<Vec<u8>>,
}

async fn compare_strategies(
    fixture: &StrategyFixture,
    measurements: &StrategyMeasurements,
) -> StrategyEvidence {
    let native_fixture = fixture.native_fixture();
    let native_frames = capture_frame_sequence(
        &native_fixture,
        &fixture.plan,
        &COLOR_SAMPLE_SEQUENCE,
        FrameRetention::Retain,
    )
    .await;
    let extraction_directory = fixture.root().join("frames");
    let injected_fixture = injected_video_fixture(&extraction_directory);
    let injected_frames = capture_frame_sequence(
        &injected_fixture,
        &fixture.plan,
        &COLOR_SAMPLE_SEQUENCE,
        FrameRetention::Retain,
    )
    .await;
    let injected = compare_encoded_frames(&native_frames.frames, &injected_frames.frames).await;

    let continuous_frames = decode_sequence(
        &fixture.media,
        fixture.frame_rate,
        experiment_dimensions(),
        &COLOR_SAMPLE_SEQUENCE,
    )
    .await;
    let native_pixels = decode_browser_frames(&native_frames.frames).await;
    let continuous = compare_pixels(&native_pixels, &continuous_frames);

    let overlay_samples =
        sample_encoded_frames(&measurements.overlay.output.frames, &COLOR_SAMPLE_SEQUENCE);
    let overlay_pixels = decode_browser_frames(&overlay_samples).await;
    assert_transparent_presentation(&overlay_pixels);

    let composited_fixture = composited_video_fixture(&fixture.media, &fixture.expected);
    let composited_frames = capture_frame_sequence(
        &composited_fixture,
        &fixture.plan,
        &COLOR_SAMPLE_SEQUENCE,
        FrameRetention::Retain,
    )
    .await;
    let composited_pixels = decode_browser_frames(&composited_frames.frames).await;
    let split_pixels = composite_pixels(&continuous_frames, &overlay_pixels);
    let split_composition = compare_pixels(&composited_pixels, &split_pixels);
    let browser_split_pixels = composite_pixels(&native_pixels, &overlay_pixels);
    let alpha_composition = compare_pixels(&composited_pixels, &browser_split_pixels);

    let native_composed_frames = compose_sequence(
        &fixture.media,
        &measurements.overlay_pattern,
        fixture.frame_rate,
        experiment_dimensions(),
        &COLOR_SAMPLE_SEQUENCE,
    )
    .await;
    let native_layer_composition = compare_pixels(&composited_pixels, &native_composed_frames);
    StrategyEvidence {
        injected,
        continuous,
        split_composition,
        alpha_composition,
        native_layer_composition,
        native_pixels,
        continuous_frames,
    }
}

fn report_strategies(measurements: &StrategyMeasurements, evidence: &StrategyEvidence) {
    report_native_strategy(&measurements.native);
    report_preextracted_strategy(&measurements.extraction, &measurements.injected);
    report_continuous_strategy(&measurements.continuous);
    report_transparent_strategy(
        &measurements.overlay,
        &measurements.composed,
        measurements.overlay_bytes,
    );
    report_path_difference("preextract-inject", evidence.injected);
    report_path_difference("continuous-decode", evidence.continuous);
    report_sample_differences(
        "continuous-decode",
        &evidence.native_pixels,
        &evidence.continuous_frames,
    );
    report_path_difference("split-composition", evidence.split_composition);
    report_path_difference("alpha-composition", evidence.alpha_composition);
    report_path_difference(
        "native-layer-composition",
        evidence.native_layer_composition,
    );
}

async fn run_case(name: &str, frame_rate: FrameRate, timing: FixtureTiming) {
    let directory = experiment_directory(name);
    let media = directory.path().join(format!("{name}.mp4"));
    generate_video(&media, frame_rate, timing).await;
    let Some(source_video) = admitted_source_video(&media, timing).await else {
        return;
    };
    assert_eq!(source_video.frame_rate, frame_rate);

    let source_frames = probe_source_frames(&media).await;
    let expected = expected_frames(frame_rate, &source_frames, &SEEK_SEQUENCE);
    let fixture = decoded_video_fixture(&media, &expected);
    let plan = browser_plan(frame_rate);

    let first_browser = capture_seek_sequence(&fixture, &plan).await;
    let second_browser = capture_seek_sequence(&fixture, &plan).await;
    assert_eq!(
        first_browser.frames, second_browser.frames,
        "{name}: browser capture",
    );
    assert_repeated_and_distinct(name, &first_browser.frames);

    let first_native = extract_reference_sequence(&media, &expected).await;
    let second_native = extract_reference_sequence(&media, &expected).await;
    assert_eq!(
        first_native.frames, second_native.frames,
        "{name}: native extraction",
    );
    assert_repeated_and_distinct(name, &first_native.frames);

    let browser_rgba = decode_browser_frames(&first_browser.frames).await;
    let difference = compare_pixels(&browser_rgba, &first_native.frames);
    report_measurement(
        name,
        &first_browser.elapsed,
        &first_native.elapsed,
        difference,
    );
}

fn assert_repeated_and_distinct<T>(name: &str, frames: &[T])
where
    T: Eq + std::fmt::Debug,
{
    assert_eq!(frames.first(), frames.last(), "{name}: repeated frame");
    assert_ne!(frames[0], frames[1], "{name}: first distinct frame");
    assert_ne!(frames[1], frames[2], "{name}: second distinct frame");
}

// ── Browser measurement ──

#[derive(Debug)]
struct MeasuredFrames<T> {
    frames: Vec<T>,
    elapsed: Vec<Duration>,
}

async fn capture_seek_sequence(fixture: &Url, plan: &BrowserPlan) -> MeasuredFrames<EncodedPng> {
    capture_frame_sequence(fixture, plan, &SEEK_SEQUENCE, FrameRetention::Retain).await
}

#[derive(Clone, Copy)]
enum FrameRetention {
    Discard,
    Retain,
}

async fn capture_frame_sequence(
    fixture: &Url,
    plan: &BrowserPlan,
    indices: &[u64],
    retention: FrameRetention,
) -> MeasuredFrames<EncodedPng> {
    let mut session = launch_browser().await;
    let capture_result =
        capture_video_frames(&mut session, fixture, plan, indices, retention).await;
    let shutdown_result = session.shutdown().await;

    let frames = capture_result.expect("the browser must seek every decoded frame");
    shutdown_result.expect("headless shell must shut down after the seek experiment");
    frames
}

async fn capture_transparent_frame_sequence(
    fixture: &Url,
    plan: &BrowserPlan,
    indices: &[u64],
    retention: FrameRetention,
) -> MeasuredFrames<EncodedPng> {
    let mut session = launch_transparent_browser().await;
    let capture_result =
        capture_video_frames(&mut session, fixture, plan, indices, retention).await;
    let shutdown_result = session.shutdown().await;

    let frames = capture_result.expect("the browser must capture every presentation frame");
    shutdown_result.expect("headless shell must shut down after transparent capture");
    frames
}

async fn launch_browser() -> BrowserSession {
    BrowserSession::launch(
        headless_shell(),
        browser_launch_policy(),
        BrowserCaptureMode::BeginFrame,
        render_profile(),
        browser_limits(),
    )
    .await
    .expect("headless shell must launch")
}

async fn launch_transparent_browser() -> BrowserSession {
    let session = launch_browser().await;
    session
        .use_transparent_capture_surface()
        .await
        .expect("Chromium must expose a transparent capture surface");
    session
}

async fn capture_video_frames(
    session: &mut BrowserSession,
    fixture: &Url,
    plan: &BrowserPlan,
    indices: &[u64],
    retention: FrameRetention,
) -> Result<MeasuredFrames<EncodedPng>, Box<dyn Error>> {
    load_and_prepare(session, fixture, plan).await?;

    let mut frames = Vec::with_capacity(indices.len());
    let mut elapsed = Vec::with_capacity(indices.len());
    for (offset, index) in indices.iter().copied().enumerate() {
        let (captured, duration) = capture_requested_frame(session, plan, offset, index).await?;
        if matches!(retention, FrameRetention::Retain) {
            frames.push(captured);
        }
        elapsed.push(duration);
    }

    dispose(session, indices.len()).await?;
    Ok(MeasuredFrames { frames, elapsed })
}

async fn capture_requested_frame(
    session: &mut BrowserSession,
    plan: &BrowserPlan,
    offset: usize,
    index: u64,
) -> Result<(EncodedPng, Duration), Box<dyn Error>> {
    let request_offset = u32::try_from(offset * 2).expect("the seek fixture is small");
    let started = Instant::now();
    stage(session, RequestId::new(3 + request_offset), index).await?;
    let frame = WireFrame::new(index).expect("the seek fixture is browser-safe");
    let captured = session.capture_png(frame, plan.frame_rate()).await?;
    confirm(session, RequestId::new(4 + request_offset), index).await?;
    Ok((captured, started.elapsed()))
}

async fn dispose(session: &BrowserSession, frames: usize) -> Result<(), Box<dyn Error>> {
    let request_count = u32::try_from(frames * 2).expect("the seek fixture is small");
    let disposed = session
        .dispatch(&BrowserRequest::new(
            RequestId::new(3 + request_count),
            BrowserCommand::Dispose,
        ))
        .await?;
    assert_eq!(disposed.event(), &BrowserEvent::Disposed);
    Ok(())
}

async fn load_and_prepare(
    session: &mut BrowserSession,
    fixture: &Url,
    plan: &BrowserPlan,
) -> Result<(), Box<dyn Error>> {
    session.navigate(fixture, &fixture_root(fixture)).await?;
    let loaded = session
        .dispatch(&BrowserRequest::new(
            RequestId::new(1),
            BrowserCommand::Load { plan: plan.clone() },
        ))
        .await?;
    assert_eq!(loaded.event(), &BrowserEvent::Loaded);

    let evaluation_start = frame(0);
    let prepared = session
        .dispatch(&BrowserRequest::new(
            RequestId::new(2),
            BrowserCommand::Prepare { evaluation_start },
        ))
        .await?;
    assert_eq!(
        prepared.event(),
        &BrowserEvent::Prepared { evaluation_start },
    );
    session
        .initialize_capture_surface(plan.frame_rate())
        .await?;
    Ok(())
}

async fn stage(
    session: &BrowserSession,
    request_id: RequestId,
    index: u64,
) -> Result<(), Box<dyn Error>> {
    let response = session
        .dispatch(&BrowserRequest::new(
            request_id,
            BrowserCommand::Seek {
                frame: frame(index),
            },
        ))
        .await?;
    assert_eq!(
        response.event(),
        &BrowserEvent::FrameStaged {
            frame: frame(index),
        },
    );
    Ok(())
}

async fn confirm(
    session: &BrowserSession,
    request_id: RequestId,
    index: u64,
) -> Result<(), Box<dyn Error>> {
    let response = session
        .dispatch(&BrowserRequest::new(
            request_id,
            BrowserCommand::Confirm {
                frame: frame(index),
            },
        ))
        .await?;
    assert_eq!(
        response.event(),
        &BrowserEvent::FrameReady {
            frame: frame(index),
        },
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum FixtureTiming {
    Constant,
    AlternatingVfr,
}

// ── Media fixtures and native reference ──

async fn generate_video(output: &Path, frame_rate: FrameRate, timing: FixtureTiming) {
    let (width, height) = experiment_dimensions();
    let source = match timing {
        FixtureTiming::Constant => format!(
            "testsrc2=size={width}x{height}:rate={}/{}:duration=2.5",
            frame_rate.numerator(),
            frame_rate.denominator(),
        ),
        FixtureTiming::AlternatingVfr => {
            format!("testsrc2=size={width}x{height}:rate=30:duration=1.7")
        }
    };
    let mut command = Command::new(required_path("ONMARK_FFMPEG"));
    command.args(["-nostdin", "-v", "error", "-f", "lavfi", "-i", &source]);
    if matches!(timing, FixtureTiming::AlternatingVfr) {
        command.args(["-vf", "setpts=(N+floor(N/2))/(30*TB)", "-fps_mode", "vfr"]);
    }
    let generated = command
        .args([
            "-an",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-x264-params",
            "colorprim=bt709:transfer=bt709:colormatrix=bt709:range=limited",
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-color_range",
            "tv",
            "-g",
            "30",
            "-bf",
            "3",
            "-movflags",
            "+faststart",
            "-y",
        ])
        .arg(output)
        .output();
    let generated = timeout(PROCESS_DEADLINE, generated)
        .await
        .expect("the experiment video must finish before its deadline")
        .expect("FFmpeg must generate the experiment video");

    assert_process_succeeded("video generation", &generated);
}

struct AdmittedFixtureVideo {
    frame_rate: FrameRate,
    color_profile: VideoColorProfile,
}

async fn admitted_source_video(
    media: &Path,
    timing: FixtureTiming,
) -> Option<AdmittedFixtureVideo> {
    let probe = Ffprobe::new(
        required_path("ONMARK_FFPROBE"),
        PROCESS_DEADLINE,
        Ffprobe::MAX_OUTPUT_BYTES,
    )
    .expect("the experiment probe limits are valid");
    let media = media.to_owned();
    let metadata = tokio::task::spawn_blocking(move || probe.probe(&media))
        .await
        .expect("the probe task must complete")
        .expect("ffprobe must normalize the experiment media");

    match timing {
        FixtureTiming::Constant => {
            let admitted =
                AdmittedVideo::admit(&metadata).expect("the CFR H.264 fixture must enter Gate one");
            let color_profile = admitted
                .metadata()
                .color_profile()
                .expect("the layered experiment requires complete source-color facts");
            Some(AdmittedFixtureVideo {
                frame_rate: admitted.frame_rate(),
                color_profile,
            })
        }
        FixtureTiming::AlternatingVfr => {
            assert_eq!(
                AdmittedVideo::admit(&metadata),
                Err(UnsupportedVideo::VariableFrameRate),
            );
            None
        }
    }
}

async fn probe_source_frames(media: &Path) -> Vec<f64> {
    let probed = Command::new(required_path("ONMARK_FFPROBE"))
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "frame=best_effort_timestamp_time",
            "-of",
            "json",
            "--",
        ])
        .arg(media)
        .output();
    let probed = timeout(PROCESS_DEADLINE, probed)
        .await
        .expect("ffprobe must finish before the experiment deadline")
        .expect("ffprobe must inspect the experiment video");
    assert_process_succeeded("frame probing", &probed);

    let response: FrameProbe =
        serde_json::from_slice(&probed.stdout).expect("ffprobe must emit frame timestamps");
    response
        .frames
        .into_iter()
        .map(|frame| {
            frame
                .best_effort_timestamp_time
                .parse()
                .expect("ffprobe frame timestamps must be decimal seconds")
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
struct ExpectedFrame {
    media_time: f64,
}

fn expected_frames(
    frame_rate: FrameRate,
    source_frames: &[f64],
    indices: &[u64],
) -> BTreeMap<u64, ExpectedFrame> {
    indices
        .iter()
        .copied()
        .map(|index| {
            let sample_time = (index as f64 + 0.5) * f64::from(frame_rate.denominator())
                / f64::from(frame_rate.numerator());
            let media_time = source_frames
                .iter()
                .copied()
                .take_while(|source_time| *source_time <= sample_time)
                .last()
                .expect("every sampled output frame must contain a source frame");
            (index, ExpectedFrame { media_time })
        })
        .collect()
}

fn expected_cfr_frames(frame_rate: FrameRate, indices: &[u64]) -> BTreeMap<u64, ExpectedFrame> {
    indices
        .iter()
        .copied()
        .map(|index| {
            let media_time = index as f64 * f64::from(frame_rate.denominator())
                / f64::from(frame_rate.numerator());
            (index, ExpectedFrame { media_time })
        })
        .collect()
}

async fn extract_reference_sequence(
    media: &Path,
    expected: &BTreeMap<u64, ExpectedFrame>,
) -> MeasuredFrames<Vec<u8>> {
    let mut frames = Vec::with_capacity(SEEK_SEQUENCE.len());
    let mut elapsed = Vec::with_capacity(SEEK_SEQUENCE.len());

    for index in SEEK_SEQUENCE {
        let frame = expected
            .get(&index)
            .expect("every requested frame has a native reference");
        let started = Instant::now();
        frames.push(extract_reference_frame(media, frame.media_time).await);
        elapsed.push(started.elapsed());
    }

    MeasuredFrames { frames, elapsed }
}

async fn extract_reference_frame(media: &Path, sample_time: f64) -> Vec<u8> {
    let sample_time = format!("{sample_time:.9}");
    let extracted = Command::new(required_path("ONMARK_FFMPEG"))
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(media)
        .args([
            "-ss",
            &sample_time,
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .output();
    let extracted = timeout(PROCESS_DEADLINE, extracted)
        .await
        .expect("native extraction must finish before its deadline")
        .expect("FFmpeg must extract the reference frame");
    assert_process_succeeded("native frame extraction", &extracted);
    assert!(
        extracted.stdout.len() <= MAX_REFERENCE_BYTES,
        "native reference exceeds the retained-byte ceiling",
    );
    extracted.stdout
}

#[derive(Clone, Copy, Debug)]
struct ExtractedFrameSequence {
    bytes: u64,
}

async fn extract_frame_sequence(
    media: &Path,
    frame_rate: FrameRate,
    frame_count: u64,
    output: &Path,
) -> ExtractedFrameSequence {
    std::fs::create_dir(output).expect("the extraction directory must be creatable");
    let frame_rate = format!("{}/{}", frame_rate.numerator(), frame_rate.denominator(),);
    let frame_filter = format!("fps={frame_rate}");
    let frame_count_argument = frame_count.to_string();
    let pattern = output.join("frame-%05d.png");
    let extracted = Command::new(required_path("ONMARK_FFMPEG"))
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(media)
        .args([
            "-vf",
            &frame_filter,
            "-frames:v",
            &frame_count_argument,
            "-compression_level",
            "1",
            "-start_number",
            "0",
            "-y",
        ])
        .arg(&pattern)
        .output();
    let extracted = timeout(PROCESS_DEADLINE, extracted)
        .await
        .expect("frame extraction must finish before its deadline")
        .expect("FFmpeg must extract the benchmark frame sequence");
    assert_process_succeeded("frame-sequence extraction", &extracted);

    let bytes = (0..frame_count)
        .map(|index| output.join(format!("frame-{index:05}.png")))
        .map(|frame| {
            std::fs::metadata(frame)
                .expect("FFmpeg must produce every requested frame")
                .len()
        })
        .sum();
    ExtractedFrameSequence { bytes }
}

fn report_measurement(
    name: &str,
    browser: &[Duration],
    native: &[Duration],
    difference: PixelDifference,
) {
    println!(
        "{name}: browser={:.2}ms native={:.2}ms channels={} differing={} max_delta={} mean_delta={:.4}",
        mean_milliseconds(browser),
        mean_milliseconds(native),
        difference.channels,
        difference.differing_channels,
        difference.maximum_delta,
        difference.mean_absolute_delta,
    );
}

fn report_native_strategy(native: &Measurement<MeasuredFrames<EncodedPng>>) {
    let (width, height) = experiment_dimensions();
    println!(
        "native-seek: size={width}x{height} frames={BENCHMARK_FRAME_COUNT} total={:.2}ms frame={:.2}ms peak+={}KiB",
        native.elapsed.as_secs_f64() * 1_000.0,
        mean_milliseconds(&native.output.elapsed),
        native.incremental_peak_rss_kib(),
    );
}

fn report_preextracted_strategy(
    extraction: &Measurement<ExtractedFrameSequence>,
    injected: &Measurement<MeasuredFrames<EncodedPng>>,
) {
    let injected_total = extraction.elapsed + injected.elapsed;
    println!(
        "preextract-inject: total={:.2}ms extract={:.2}ms inject={:.2}ms frame={:.2}ms extract_peak+={}KiB inject_peak+={}KiB files={} bytes",
        injected_total.as_secs_f64() * 1_000.0,
        extraction.elapsed.as_secs_f64() * 1_000.0,
        injected.elapsed.as_secs_f64() * 1_000.0,
        mean_milliseconds(&injected.output.elapsed),
        extraction.incremental_peak_rss_kib(),
        injected.incremental_peak_rss_kib(),
        extraction.output.bytes,
    );
}

fn report_continuous_strategy(continuous: &Measurement<Vec<Vec<u8>>>) {
    println!(
        "continuous-decode: total={:.2}ms frame={:.2}ms peak+={}KiB",
        continuous.elapsed.as_secs_f64() * 1_000.0,
        continuous.elapsed.as_secs_f64() * 1_000.0 / BENCHMARK_FRAME_COUNT as f64,
        continuous.incremental_peak_rss_kib(),
    );
}

fn report_transparent_strategy(
    overlay: &Measurement<MeasuredFrames<EncodedPng>>,
    composed: &Measurement<Vec<Vec<u8>>>,
    overlay_bytes: u64,
) {
    let sequential_total = overlay.elapsed + composed.elapsed;
    println!(
        "native-layer-composition: total={:.2}ms overlay={:.2}ms compose={:.2}ms overlay_frame={:.2}ms overlay_peak+={}KiB compose_peak+={}KiB layer_bytes={}",
        sequential_total.as_secs_f64() * 1_000.0,
        overlay.elapsed.as_secs_f64() * 1_000.0,
        composed.elapsed.as_secs_f64() * 1_000.0,
        mean_milliseconds(&overlay.output.elapsed),
        overlay.incremental_peak_rss_kib(),
        composed.incremental_peak_rss_kib(),
        overlay_bytes,
    );
}

fn report_path_difference(path: &str, difference: PixelDifference) {
    println!(
        "{path}-difference: samples={} channels={} differing={} max_delta={} mean_delta={:.4}",
        COLOR_SAMPLE_SEQUENCE.len(),
        difference.channels,
        difference.differing_channels,
        difference.maximum_delta,
        difference.mean_absolute_delta,
    );
}

fn report_sample_differences(path: &str, expected: &[Vec<u8>], actual: &[Vec<u8>]) {
    for ((index, expected), actual) in COLOR_SAMPLE_SEQUENCE.iter().zip(expected).zip(actual) {
        let difference =
            compare_pixels(std::slice::from_ref(expected), std::slice::from_ref(actual));
        println!(
            "{path}-frame-{index}: differing={} max_delta={} mean_delta={:.4}",
            difference.differing_channels, difference.maximum_delta, difference.mean_absolute_delta,
        );
    }
}

fn mean_milliseconds(values: &[Duration]) -> f64 {
    let total = values.iter().sum::<Duration>();
    total.as_secs_f64() * 1_000.0 / values.len() as f64
}

fn assert_process_succeeded(role: &str, output: &std::process::Output) {
    assert!(
        output.status.success(),
        "{role} failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

// ── Protocol fixture and environment ──

fn decoded_video_fixture(media: &Path, expected: &BTreeMap<u64, ExpectedFrame>) -> Url {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/decoded-video.html");
    let mut fixture = Url::from_file_path(fixture).expect("the fixture path is absolute");
    let media = Url::from_file_path(media).expect("the experiment media path is absolute");
    let expected = expected
        .iter()
        .map(|(index, frame)| (index, frame.media_time))
        .collect::<BTreeMap<_, _>>();
    let expected = serde_json::to_string(&expected).expect("expected frame times must serialize");
    let asset_id = fixture_asset_id().to_string();
    fixture
        .query_pairs_mut()
        .append_pair("media", media.as_str())
        .append_pair("asset", &asset_id)
        .append_pair("expected", &expected);
    fixture
}

fn composited_video_fixture(media: &Path, expected: &BTreeMap<u64, ExpectedFrame>) -> Url {
    let mut fixture = decoded_video_fixture(media, expected);
    fixture.query_pairs_mut().append_pair("overlay", "true");
    fixture
}

fn injected_video_fixture(frames: &Path) -> Url {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/injected-video.html");
    let mut fixture = Url::from_file_path(fixture).expect("the fixture path is absolute");
    let frames = Url::from_directory_path(frames).expect("the frame directory path is absolute");
    fixture
        .query_pairs_mut()
        .append_pair("frames", frames.as_str());
    fixture
}

fn transparent_overlay_fixture() -> Url {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transparent-overlay.html");
    Url::from_file_path(fixture).expect("the fixture path is absolute")
}

fn fixture_root(fixture: &Url) -> PathBuf {
    let path = fixture
        .to_file_path()
        .expect("the browser fixture must be a file URL");
    let repository = repository();

    // Handwritten modules import the built runtime across repository
    // directories. Experiment media is staged beneath the same private root.
    if path.starts_with(&repository) {
        return repository;
    }

    path.parent()
        .expect("the browser fixture must have a parent directory")
        .to_owned()
}

fn experiment_directory(name: &str) -> TempDir {
    let target = repository().join("target");
    fs::create_dir_all(&target).expect("the Cargo target directory must be available");
    TempDirBuilder::new()
        .prefix(&format!("onmark-{name}-"))
        .tempdir_in(target)
        .expect("the private experiment directory must be available")
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("render is nested at crates/render")
        .to_owned()
}

fn write_overlay_sequence(frames: &[EncodedPng], output: &Path) -> u64 {
    fs::create_dir(output).expect("the overlay directory must be creatable");
    frames
        .iter()
        .enumerate()
        .map(|(index, frame)| {
            let path = output.join(format!("frame-{index:05}.png"));
            fs::write(path, frame.as_bytes()).expect("every overlay frame must be writable");
            u64::try_from(frame.as_bytes().len()).expect("overlay bytes must fit their accounting")
        })
        .sum()
}

fn sample_encoded_frames(frames: &[EncodedPng], indices: &[u64]) -> Vec<EncodedPng> {
    indices
        .iter()
        .map(|index| {
            let index = usize::try_from(*index).expect("sample indices must fit this process");
            frames
                .get(index)
                .expect("every sampled overlay frame must exist")
                .clone()
        })
        .collect()
}

fn assert_transparent_presentation(frames: &[Vec<u8>]) {
    for frame in frames {
        let has_transparent = frame.iter().skip(3).step_by(4).any(|alpha| *alpha == 0);
        let has_visible = frame.iter().skip(3).step_by(4).any(|alpha| *alpha > 0);
        assert!(
            has_transparent,
            "the presentation sample must retain transparent pixels",
        );
        assert!(
            has_visible,
            "the presentation sample must retain visible pixels",
        );
    }
}

fn browser_plan(frame_rate: FrameRate) -> BrowserPlan {
    browser_plan_with_source_rate(frame_rate, frame_rate)
}

fn browser_plan_with_source_rate(
    output_frame_rate: FrameRate,
    source_frame_rate: FrameRate,
) -> BrowserPlan {
    let duration = MediaDuration::from_nanos(2_500_000_000);
    let (width, height) = experiment_dimensions();
    let video = VideoMetadata::new(
        duration,
        VideoDimensions::new(width, height).expect("fixture dimensions are positive"),
        "h264",
        "yuv420p",
        VideoTiming::Constant(source_frame_rate),
    )
    .expect("the fixture video metadata is normalized");
    let asset = AssetRef::parse("experiment.mp4").expect("the fixture asset is valid");
    let assets = BTreeMap::from([(
        asset,
        FrozenAsset::new(fixture_asset_id(), AssetMetadata::video(duration, video)),
    )]);
    let parsed = compiler::parse(
        SourceId::new(0),
        r#"<film><scene><shot><video src="experiment.mp4" /></shot></scene></film>"#,
    );
    let (document, diagnostics) = parsed.into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::bind(document).into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::resolve(film.expect("the fixture binds")).into_parts();
    assert!(diagnostics.is_empty());
    let solved = compiler::solve(
        film.expect("the fixture resolves"),
        &assets,
        Timebase::new(output_frame_rate),
    )
    .expect("the fixture metadata is complete");
    assert!(solved.diagnostics().is_empty());

    let source_frame_rates = BTreeMap::from([(fixture_asset_id(), source_frame_rate)]);
    BrowserPlan::from_timeline(
        solved.timeline().expect("the fixture solves"),
        &source_frame_rates,
    )
    .expect("the fixture timeline fits the browser frame domain")
}

fn fixture_asset_id() -> FrozenAssetId {
    FrozenAssetId::from_sha256([1; 32])
}

fn frame(index: u64) -> WireFrame {
    WireFrame::new(index).expect("fixture frames are browser-safe")
}

fn headless_shell() -> PathBuf {
    required_path("ONMARK_HEADLESS_SHELL")
}

fn browser_launch_policy() -> BrowserLaunchPolicy {
    if env::var_os("ONMARK_ISOLATED_WORKER").is_some() {
        BrowserLaunchPolicy::isolated_worker()
    } else {
        BrowserLaunchPolicy::local()
    }
}

fn required_path(variable: &str) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{variable} must name an executable"))
}

fn browser_limits() -> BrowserLimits {
    BrowserLimits::new(Duration::from_secs(10), MAX_REFERENCE_BYTES)
        .expect("the fixture browser limits are bounded")
}

fn render_profile() -> RenderProfile {
    let (width, height) = experiment_dimensions();
    RenderProfile::new(width, height).expect("the fixture render profile is valid")
}

fn experiment_dimensions() -> (u32, u32) {
    (
        experiment_dimension(WIDTH_ENVIRONMENT, WIDTH),
        experiment_dimension(HEIGHT_ENVIRONMENT, HEIGHT),
    )
}

fn experiment_dimension(variable: &str, default: u32) -> u32 {
    let Some(value) = env::var_os(variable) else {
        return default;
    };
    let value = value
        .to_str()
        .unwrap_or_else(|| panic!("{variable} must contain UTF-8"));
    let dimension = value
        .parse::<u32>()
        .unwrap_or_else(|_| panic!("{variable} must be a positive integer"));
    assert!(dimension > 0, "{variable} must be a positive integer");
    dimension
}

#[derive(Debug, Deserialize)]
struct FrameProbe {
    frames: Vec<ProbedFrame>,
}

#[derive(Debug, Deserialize)]
struct ProbedFrame {
    best_effort_timestamp_time: String,
}
