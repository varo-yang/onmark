//! Disposable media-strategy experiment across real Chromium and `FFmpeg`.
//!
//! The test stays opt-in until CI owns pinned browser and codec binaries. It
//! measures selection and repeatability without turning an experimental media
//! path into a product API.

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use onmark_core::compiler;
use onmark_core::model::{
    AssetMetadata, AssetRef, Duration as MediaDuration, FrameRate, FrozenAsset, FrozenAssetId,
    SourceId, Timebase, VideoMetadata, VideoTiming,
};
use onmark_core::protocol::{
    BrowserCommand, BrowserEvent, BrowserPlan, BrowserRequest, RequestId, WireFrame,
};
use onmark_media::Ffprobe;
use onmark_render::{AdmittedVideo, BrowserLimits, BrowserSession, EncodedPng, UnsupportedVideo};
use serde::Deserialize;
use tempfile::{NamedTempFile, tempdir};
use tokio::process::Command;
use tokio::time::timeout;
use url::Url;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const SEEK_SEQUENCE: [u64; 4] = [17, 3, 29, 17];
const PROCESS_DEADLINE: Duration = Duration::from_secs(20);
const MAX_REFERENCE_BYTES: usize = 8 * 1024 * 1024;

#[tokio::test]
#[ignore = "requires ONMARK_CHROME, ONMARK_FFMPEG, ONMARK_FFPROBE, and built runtime"]
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

async fn run_case(name: &str, frame_rate: FrameRate, timing: FixtureTiming) {
    let directory = tempdir().expect("the experiment directory must be available");
    let media = directory.path().join(format!("{name}.mp4"));
    generate_video(&media, frame_rate, timing).await;
    let Some(source_frame_rate) = admitted_source_rate(&media, timing).await else {
        return;
    };
    assert_eq!(source_frame_rate, frame_rate);

    let source_frames = probe_source_frames(&media).await;
    let expected = expected_frames(frame_rate, &source_frames);
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
    let session = BrowserSession::launch(chrome(), browser_limits())
        .await
        .expect("Chrome must launch");
    let capture_result = capture_video_frames(&session, fixture, plan).await;
    let shutdown_result = session.shutdown().await;

    let frames = capture_result.expect("the browser must seek every decoded frame");
    shutdown_result.expect("Chrome must shut down after the seek experiment");
    frames
}

async fn capture_video_frames(
    session: &BrowserSession,
    fixture: &Url,
    plan: &BrowserPlan,
) -> Result<MeasuredFrames<EncodedPng>, Box<dyn Error>> {
    load_and_prepare(session, fixture, plan).await?;

    let mut frames = Vec::with_capacity(SEEK_SEQUENCE.len());
    let mut elapsed = Vec::with_capacity(SEEK_SEQUENCE.len());
    for (offset, index) in SEEK_SEQUENCE.into_iter().enumerate() {
        let request_id = u32::try_from(offset + 3).expect("the seek fixture is small");
        let started = Instant::now();
        seek(session, request_id, index).await?;
        frames.push(session.capture_png().await?);
        elapsed.push(started.elapsed());
    }

    let dispose_id = u32::try_from(SEEK_SEQUENCE.len() + 3).expect("the seek fixture is small");
    let disposed = session
        .dispatch(&BrowserRequest::new(
            RequestId::new(dispose_id),
            BrowserCommand::Dispose,
        ))
        .await?;
    assert_eq!(disposed.event(), &BrowserEvent::Disposed);

    Ok(MeasuredFrames { frames, elapsed })
}

async fn load_and_prepare(
    session: &BrowserSession,
    fixture: &Url,
    plan: &BrowserPlan,
) -> Result<(), Box<dyn Error>> {
    session.navigate(fixture.as_str()).await?;
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
    Ok(())
}

async fn seek(session: &BrowserSession, request_id: u32, index: u64) -> Result<(), Box<dyn Error>> {
    let response = session
        .dispatch(&BrowserRequest::new(
            RequestId::new(request_id),
            BrowserCommand::Seek {
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
    let source = match timing {
        FixtureTiming::Constant => format!(
            "testsrc2=size={WIDTH}x{HEIGHT}:rate={}/{}:duration=2.5",
            frame_rate.numerator(),
            frame_rate.denominator(),
        ),
        FixtureTiming::AlternatingVfr => {
            format!("testsrc2=size={WIDTH}x{HEIGHT}:rate=30:duration=1.7")
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

async fn admitted_source_rate(media: &Path, timing: FixtureTiming) -> Option<FrameRate> {
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
            Some(admitted.frame_rate())
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

fn expected_frames(frame_rate: FrameRate, source_frames: &[f64]) -> BTreeMap<u64, ExpectedFrame> {
    std::iter::once(0)
        .chain(SEEK_SEQUENCE)
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

async fn decode_browser_frames(frames: &[EncodedPng]) -> Vec<Vec<u8>> {
    let mut decoded = Vec::with_capacity(frames.len());

    for frame in frames {
        decoded.push(decode_browser_frame(frame).await);
    }

    decoded
}

async fn decode_browser_frame(frame: &EncodedPng) -> Vec<u8> {
    let mut encoded = NamedTempFile::new().expect("the PNG staging file must be available");
    encoded
        .write_all(frame.as_bytes())
        .expect("the captured PNG must fit its staging file");
    encoded.flush().expect("the staged PNG must be readable");

    let decoded = Command::new(required_path("ONMARK_FFMPEG"))
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(encoded.path())
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output();
    let decoded = timeout(PROCESS_DEADLINE, decoded)
        .await
        .expect("PNG decoding must finish before its deadline")
        .expect("FFmpeg must decode the browser capture");
    assert_process_succeeded("browser PNG decoding", &decoded);
    decoded.stdout
}

#[derive(Clone, Copy, Debug)]
struct PixelDifference {
    differing_channels: usize,
    maximum_delta: u8,
    mean_absolute_delta: f64,
}

fn compare_pixels(browser: &[Vec<u8>], native: &[Vec<u8>]) -> PixelDifference {
    let browser = browser.concat();
    let native = native.concat();
    assert_eq!(browser.len(), native.len(), "RGBA frame domains must match");
    let channel_count = native.len();

    let mut differing_channels = 0;
    let mut maximum_delta = 0;
    let mut total_delta = 0_u64;
    for (browser, native) in browser.into_iter().zip(native) {
        let delta = browser.abs_diff(native);
        differing_channels += usize::from(delta != 0);
        maximum_delta = maximum_delta.max(delta);
        total_delta += u64::from(delta);
    }
    let mean_absolute_delta = total_delta as f64 / channel_count as f64;

    PixelDifference {
        differing_channels,
        maximum_delta,
        mean_absolute_delta,
    }
}

fn report_measurement(
    name: &str,
    browser: &[Duration],
    native: &[Duration],
    difference: PixelDifference,
) {
    println!(
        "{name}: browser={:.2}ms native={:.2}ms differing={} max_delta={} mean_delta={:.4}",
        mean_milliseconds(browser),
        mean_milliseconds(native),
        difference.differing_channels,
        difference.maximum_delta,
        difference.mean_absolute_delta,
    );
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

fn browser_plan(frame_rate: FrameRate) -> BrowserPlan {
    let duration = MediaDuration::from_nanos(2_500_000_000);
    let video = VideoMetadata::new(
        duration,
        "h264",
        "yuv420p",
        VideoTiming::Constant(frame_rate),
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
        Timebase::new(frame_rate),
    )
    .expect("the fixture metadata is complete");
    assert!(solved.diagnostics().is_empty());

    let source_frame_rates = BTreeMap::from([(fixture_asset_id(), frame_rate)]);
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

fn chrome() -> PathBuf {
    required_path("ONMARK_CHROME")
}

fn required_path(variable: &str) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{variable} must name an executable"))
}

fn browser_limits() -> BrowserLimits {
    BrowserLimits::new(WIDTH, HEIGHT, Duration::from_secs(10), MAX_REFERENCE_BYTES)
        .expect("the fixture browser limits are bounded")
}

#[derive(Debug, Deserialize)]
struct FrameProbe {
    frames: Vec<ProbedFrame>,
}

#[derive(Debug, Deserialize)]
struct ProbedFrame {
    best_effort_timestamp_time: String,
}
