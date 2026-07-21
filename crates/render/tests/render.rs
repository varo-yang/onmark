//! Opt-in real-process conformance for capture, partitioning, and assembly.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use onmark_core::compiler;
use onmark_core::model::{
    AssetRef, FrameRate, FrozenAsset, FrozenAssetId, PresentationTemporalCapability, SourceId,
    Timebase,
};
use onmark_core::protocol::{
    BrowserCommand, BrowserEvent, BrowserOverlayKind, BrowserPlan, BrowserRequest, BundleManifest,
    RequestId, WireFrame,
};
use onmark_core::render_graph::{PartitionPlan, RenderGraph};
use onmark_media::{Ffprobe, SubtitleLimits, parse_webvtt};
use onmark_render::{
    BrowserErrorKind, BrowserLaunchPolicy, BrowserLimits, BrowserSession, CaptureEnvironmentId,
    EncodeLimits, EncodedPng, ExecutableUnit, Ffmpeg, FrameArtifact, FrameArtifactLimits,
    MaterializedAsset, RawRgbaHash, RenderErrorKind, RenderExecutor, RenderProfile, RenderUnit,
    UnitRootLimits,
};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use tempfile::tempdir;
use tokio::process::Command;
use tokio::time::timeout;
use url::Url;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const FRAME_COUNT: u64 = 75;
const TWO_UNIT_FRAME_COUNT: u64 = 60;
const TEMPORAL_SEEK_SEQUENCE: [u64; 4] = [17, 3, 29, 17];
const MICROS_PER_SECOND: i64 = 1_000_000;
const AUDIO_TIMESTAMP_TOLERANCE_MICROS: u64 = 25_000;

#[tokio::test]
async fn rejects_units_that_do_not_match_the_partition_plan_before_launching_browser() {
    let timeline = solve_timeline(
        r#"<film><scene><shot duration="1s" /><shot duration="1s" /></scene></film>"#,
        &BTreeMap::new(),
    );
    let partitions =
        RenderGraph::from_timeline(&timeline, PresentationTemporalCapability::RandomAccess)
            .into_partition();
    let directory = tempdir().expect("the test output directory must be available");
    let output = directory.path().join("partitioned.mp4");
    let limits = EncodeLimits::new(Duration::from_secs(1), 2, 2, 2)
        .expect("the fixture encoding limits are bounded");
    let ffmpeg = Ffmpeg::new("ffmpeg", limits).expect("the fixture executable path is present");
    let executor = RenderExecutor::new("browser", browser_limits(Duration::from_secs(1)), ffmpeg);

    let error = executor
        .render_partitioned(&partitions, Vec::new(), &output)
        .await
        .expect_err("all partition units must be present before browser launch");

    assert_eq!(error.kind(), RenderErrorKind::InvalidPlan);
    assert!(!output.exists());
}

#[tokio::test]
#[ignore = "requires ONMARK_HEADLESS_SHELL"]
async fn rejects_a_page_that_never_installs_the_runtime_host() {
    let session = BrowserSession::launch(
        headless_shell(),
        BrowserLaunchPolicy::local(),
        render_profile(),
        browser_limits(Duration::from_secs(5)),
    )
    .await
    .expect("headless shell must launch");
    let fixture = render_fixture("missing-runtime.html");

    let error = session
        .navigate(fixture.as_str())
        .await
        .expect_err("the missing host must miss its readiness deadline");
    let shutdown = session.shutdown().await;

    assert_eq!(error.kind(), BrowserErrorKind::RuntimeHost);
    shutdown.expect("headless shell must shut down after a readiness failure");
}

#[tokio::test]
#[ignore = "requires ONMARK_HEADLESS_SHELL and a built @onmark/runtime package"]
async fn bounds_a_runtime_adapter_that_never_finishes_loading() {
    let session = BrowserSession::launch(
        headless_shell(),
        BrowserLaunchPolicy::local(),
        render_profile(),
        browser_limits(Duration::from_secs(5)),
    )
    .await
    .expect("headless shell must launch");
    let fixture = render_fixture("stalled-runtime.html");
    session
        .navigate(fixture.as_str())
        .await
        .expect("the stalled fixture must install its runtime host");

    let request = BrowserRequest::new(
        RequestId::new(1),
        BrowserCommand::Load {
            plan: gate_one_plan(),
        },
    );
    let error = session
        .dispatch(&request)
        .await
        .expect_err("the stalled adapter must miss its protocol deadline");
    let shutdown = session.shutdown().await;

    assert_eq!(error.kind(), BrowserErrorKind::Protocol);
    shutdown.expect("headless shell must shut down after a protocol timeout");
}

#[tokio::test]
#[ignore = "requires ONMARK_HEADLESS_SHELL and a built @onmark/runtime package"]
async fn captures_stable_raw_rgba_frames_across_independent_browser_sessions() {
    let fixture = browser_fixture();
    let first = capture_protocol_fingerprint(&fixture).await;
    let second = capture_protocol_fingerprint(&fixture).await;

    assert_eq!(
        first, second,
        "locked browser sessions must capture equal RGBA"
    );
}

#[tokio::test]
#[ignore = "requires ONMARK_BUNDLER and ONMARK_HEADLESS_SHELL"]
async fn seeks_browser_animation_playheads_deterministically() {
    let directory = tempdir().expect("the experiment workspace must be available");
    let fixture = temporal_experiment_fixture(directory.path()).await;
    let first = capture_temporal_sequence(&fixture).await;
    let second = capture_temporal_sequence(&fixture).await;

    assert_eq!(first, second, "independent browser processes must agree");
    assert_eq!(first[0], first[3], "repeated exact frames must agree");
    assert!(
        first.windows(2).any(|frames| frames[0] != frames[1]),
        "the experiment must contain visible temporal change",
    );
}

#[tokio::test]
#[ignore = "requires ONMARK_HEADLESS_SHELL, ONMARK_FFMPEG, and ONMARK_FFPROBE"]
async fn renders_the_gate_one_plan_to_a_verified_mp4() {
    let directory = tempdir().expect("the test output directory must be available");
    let source = directory.path().join("source.mp4");
    let output = directory.path().join("gate-one.mp4");
    generate_source_video(&source, "2.5").await;
    let frozen = freeze_asset(&source).await;
    let executor = real_executor(100);
    let unit = executable_gate_one_unit(frozen, source);

    let video = executor
        .render(unit, &output)
        .await
        .expect("the real local renderer must produce an MP4");

    assert_eq!(video.path(), output);
    assert_eq!(video.frames(), FRAME_COUNT);
    assert!(output.metadata().expect("the MP4 must exist").len() > 0);
    assert_video_stream(&output, FRAME_COUNT).await;
    assert_decodable_motion(&output).await;
}

#[tokio::test]
#[ignore = "requires ONMARK_HEADLESS_SHELL, ONMARK_FFMPEG, and ONMARK_FFPROBE"]
async fn renders_random_access_media_equally_as_one_or_two_units() {
    let directory = tempdir().expect("the test output directory must be available");
    let fixture = GateFourFixture::materialize(directory.path()).await;
    let whole_output = directory.path().join("whole.mp4");
    let partitioned_output = directory.path().join("partitioned.mp4");
    let executor = real_executor(TWO_UNIT_FRAME_COUNT);

    let whole = executor
        .render(fixture.whole_film, &whole_output)
        .await
        .expect("the whole-film random-access plan must render");
    let partitioned = executor
        .render_partitioned(
            &fixture.partition_plan,
            fixture.partitioned_units,
            &partitioned_output,
        )
        .await
        .expect("the two unit plan must render");

    assert_eq!(whole.frames(), TWO_UNIT_FRAME_COUNT);
    assert_eq!(partitioned.frames(), TWO_UNIT_FRAME_COUNT);
    let whole = inspect_gate_four_output(&whole_output).await;
    let partitioned = inspect_gate_four_output(&partitioned_output).await;
    assert_eq!(
        whole.audio_hashes, partitioned.audio_hashes,
        "partitioning must not change the decoded final audio",
    );
}

#[tokio::test]
#[ignore = "requires ONMARK_HEADLESS_SHELL, ONMARK_FFMPEG, and ONMARK_FFPROBE"]
async fn assembles_worker_frame_artifacts_equivalently_to_the_whole_film() {
    let directory = tempdir().expect("the test output directory must be available");
    let fixture = GateFourFixture::materialize(directory.path()).await;
    let whole_artifact_path = directory.path().join("whole-film.onmark-frames");
    let assembled_output = directory.path().join("assembled-from-artifacts.mp4");
    let executor = real_executor(TWO_UNIT_FRAME_COUNT);

    let whole = executor
        .capture_frame_artifact(
            &fixture.whole_film,
            capture_environment(),
            &whole_artifact_path,
            frame_artifact_limits(),
        )
        .await
        .expect("the whole-film baseline must capture canonical pixels");

    let mut artifacts = Vec::new();
    for (index, unit) in fixture.partitioned_units.iter().enumerate() {
        let artifact = directory
            .path()
            .join(format!("worker-{index}.onmark-frames"));
        let captured = executor
            .capture_frame_artifact(
                unit,
                capture_environment(),
                &artifact,
                frame_artifact_limits(),
            )
            .await
            .expect("each independent unit must publish a verified frame artifact");
        artifacts.push(captured);
    }

    let assembled = executor
        .assemble_frame_artifacts(
            &fixture.partition_plan,
            &fixture.partitioned_units,
            &artifacts,
            capture_environment(),
            &assembled_output,
        )
        .await
        .expect("the assembler must reuse worker artifacts through one encoder");

    FrameArtifact::verify_raw_rgba_equivalence(std::slice::from_ref(&whole), &artifacts)
        .await
        .expect("partition artifacts must reproduce the whole-film pixel sequence");
    assert_eq!(assembled.frames(), TWO_UNIT_FRAME_COUNT);
    inspect_gate_four_output(&assembled_output).await;
}

async fn inspect_gate_four_output(output: &Path) -> DecodedOutput {
    assert_video_stream(output, TWO_UNIT_FRAME_COUNT).await;
    let output = inspect_output(output).await;
    assert!(
        output.has_motion(),
        "the Gate-four video must contain motion"
    );
    assert!(
        !output.audio_hashes.is_empty(),
        "the Gate-four video must retain its final audio mix",
    );
    assert_audio_starts_at(&output, 0);
    output
}

async fn generate_source_video(output: &Path, duration_seconds: &str) {
    let source = format!("testsrc2=size={WIDTH}x{HEIGHT}:rate=30:duration={duration_seconds}");
    let generated = Command::new(required_path("ONMARK_FFMPEG"))
        .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i", &source])
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
    let generated = timeout(Duration::from_secs(20), generated)
        .await
        .expect("source generation must finish before its deadline")
        .expect("FFmpeg must generate the source video");
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr),
    );
}

async fn generate_voice_over(output: &Path) {
    let generated = Command::new(required_path("ONMARK_FFMPEG"))
        .args([
            "-nostdin",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=1",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-y",
        ])
        .arg(output)
        .output();
    let generated = timeout(Duration::from_secs(20), generated)
        .await
        .expect("voice-over generation must finish before its deadline")
        .expect("FFmpeg must generate the voice-over");
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr),
    );
}

async fn generate_audio(
    output: &Path,
    frequency: u32,
    sample_rate: u32,
    channels: u8,
    duration_seconds: &str,
) {
    let source =
        format!("sine=frequency={frequency}:sample_rate={sample_rate}:duration={duration_seconds}");
    let generated = Command::new(required_path("ONMARK_FFMPEG"))
        .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i", &source])
        .arg("-ac")
        .arg(channels.to_string())
        .args(["-c:a", "pcm_s16le", "-y"])
        .arg(output)
        .output();
    let generated = timeout(Duration::from_secs(20), generated)
        .await
        .expect("audio generation must finish before its deadline")
        .expect("FFmpeg must generate the audio fixture");
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr),
    );
}

async fn freeze_asset(path: &Path) -> FrozenAsset {
    let probe = Ffprobe::new(
        required_path("ONMARK_FFPROBE"),
        Duration::from_secs(20),
        Ffprobe::MAX_OUTPUT_BYTES,
    )
    .expect("the fixture probe is bounded");
    let source = path.to_owned();
    let metadata = tokio::task::spawn_blocking(move || probe.probe(&source))
        .await
        .expect("the probe task must complete")
        .expect("ffprobe must normalize the source media");
    let bytes = fs::read(path).expect("the source video must remain readable");
    let digest: [u8; 32] = Sha256::digest(bytes).into();

    FrozenAsset::new(FrozenAssetId::from_sha256(digest), metadata)
}

async fn capture_protocol_fingerprint(fixture: &Url) -> RawRgbaHash {
    let mut session = BrowserSession::launch(
        headless_shell(),
        BrowserLaunchPolicy::local(),
        render_profile(),
        browser_limits(Duration::from_secs(10)),
    )
    .await
    .expect("headless shell must launch");
    let result = exercise_protocol(&mut session, fixture).await;
    let shutdown = session.shutdown().await;

    let fingerprint = result.expect("the real browser protocol must capture deterministic frames");
    shutdown.expect("headless shell must shut down cleanly");
    fingerprint
}

async fn capture_temporal_sequence(fixture: &Url) -> Vec<RawRgbaHash> {
    let mut session = BrowserSession::launch(
        headless_shell(),
        BrowserLaunchPolicy::local(),
        render_profile(),
        browser_limits(Duration::from_secs(10)),
    )
    .await
    .expect("headless shell must launch");
    let result = exercise_temporal_sequence(&mut session, fixture).await;
    let shutdown = session.shutdown().await;

    let fingerprints = result.expect("the temporal experiment must capture every frame");
    shutdown.expect("headless shell must shut down cleanly");
    fingerprints
}

async fn exercise_temporal_sequence(
    session: &mut BrowserSession,
    fixture: &Url,
) -> Result<Vec<RawRgbaHash>, Box<dyn Error>> {
    load_and_prepare(session, fixture).await?;
    let frame_rate = gate_one_plan().frame_rate();
    let mut fingerprints = Vec::with_capacity(TEMPORAL_SEEK_SEQUENCE.len());
    let mut request_id = 3_u32;

    for index in TEMPORAL_SEEK_SEQUENCE {
        stage(session, request_id, index).await?;
        let captured = session.capture_frame(frame(index), frame_rate).await?;
        confirm(session, request_id + 1, index).await?;
        fingerprints.push(captured.raw_rgba_hash());
        request_id += 2;
    }

    let disposed = session
        .dispatch(&BrowserRequest::new(
            RequestId::new(request_id),
            BrowserCommand::Dispose,
        ))
        .await?;
    assert_eq!(disposed.event(), &BrowserEvent::Disposed);

    Ok(fingerprints)
}

async fn exercise_protocol(
    session: &mut BrowserSession,
    fixture: &Url,
) -> Result<RawRgbaHash, Box<dyn Error>> {
    load_and_prepare(session, fixture).await?;

    stage(session, 3, 15).await?;
    let captured = session
        .capture_frame(frame(15), gate_one_plan().frame_rate())
        .await?;
    confirm(session, 4, 15).await?;
    let disposed = session
        .dispatch(&BrowserRequest::new(
            RequestId::new(5),
            BrowserCommand::Dispose,
        ))
        .await?;
    assert_eq!(disposed.event(), &BrowserEvent::Disposed);

    assert_png(captured.png());
    Ok(captured.raw_rgba_hash())
}

async fn load_and_prepare(session: &BrowserSession, fixture: &Url) -> Result<(), Box<dyn Error>> {
    session.navigate(fixture.as_str()).await?;
    let plan = gate_one_plan();
    let frame_rate = plan.frame_rate();
    let loaded = session
        .dispatch(&BrowserRequest::new(
            RequestId::new(1),
            BrowserCommand::Load { plan },
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
    session.initialize_capture_surface(frame_rate).await?;
    Ok(())
}

async fn stage(
    session: &BrowserSession,
    request_id: u32,
    index: u64,
) -> Result<(), Box<dyn Error>> {
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
        &BrowserEvent::FrameStaged {
            frame: frame(index),
        },
    );
    Ok(())
}

async fn confirm(
    session: &BrowserSession,
    request_id: u32,
    index: u64,
) -> Result<(), Box<dyn Error>> {
    let response = session
        .dispatch(&BrowserRequest::new(
            RequestId::new(request_id),
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

fn frame(index: u64) -> WireFrame {
    WireFrame::new(index).expect("fixture frames are browser-safe")
}

fn assert_png(frame: &EncodedPng) {
    assert!(frame.as_bytes().starts_with(b"\x89PNG\r\n\x1a\n"));
}

async fn assert_video_stream(output: &Path, expected_frames: u64) {
    let probe = Command::new(required_path("ONMARK_FFPROBE"))
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=width,height,avg_frame_rate,nb_read_frames",
            "-of",
            "json",
            "--",
        ])
        .arg(output)
        .output();
    let probe = timeout(Duration::from_secs(10), probe)
        .await
        .expect("ffprobe must finish before the conformance deadline")
        .expect("ffprobe must inspect the encoded MP4");
    assert!(
        probe.status.success(),
        "{}",
        String::from_utf8_lossy(&probe.stderr)
    );
    let response: ProbeResponse =
        serde_json::from_slice(&probe.stdout).expect("ffprobe must emit its JSON response");
    let [stream] = response.streams.as_slice() else {
        panic!("ffprobe must report exactly one video stream");
    };

    assert_eq!(stream.width, WIDTH);
    assert_eq!(stream.height, HEIGHT);
    assert_eq!(stream.avg_frame_rate, "30/1");
    assert_eq!(stream.nb_read_frames, expected_frames.to_string());
}

async fn assert_decodable_motion(output: &Path) {
    let hashes = decoded_hashes(output, "0:v:0").await;
    let hashes = hashes.iter().collect::<BTreeSet<_>>();
    assert!(hashes.len() > 1, "the rendered video must contain motion");
}

struct DecodedOutput {
    video_hashes: Vec<String>,
    audio_hashes: Vec<String>,
    audio_start_micros: i64,
}

impl DecodedOutput {
    fn has_motion(&self) -> bool {
        let Some(first) = self.video_hashes.first() else {
            return false;
        };
        self.video_hashes.iter().any(|hash| hash != first)
    }
}

async fn inspect_output(output: &Path) -> DecodedOutput {
    DecodedOutput {
        video_hashes: decoded_hashes(output, "0:v:0").await,
        audio_hashes: decoded_hashes(output, "0:a:0").await,
        audio_start_micros: first_audio_packet_micros(output).await,
    }
}

async fn decoded_hashes(output: &Path, stream: &str) -> Vec<String> {
    let decoded = Command::new(required_path("ONMARK_FFMPEG"))
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(output)
        .args(["-map", stream, "-f", "framemd5", "-"])
        .output();
    let decoded = timeout(Duration::from_secs(10), decoded)
        .await
        .expect("frame hashing must finish before the conformance deadline")
        .expect("FFmpeg must hash the completed MP4");
    assert!(
        decoded.status.success(),
        "{}",
        String::from_utf8_lossy(&decoded.stderr),
    );

    String::from_utf8(decoded.stdout)
        .expect("framemd5 output must be UTF-8")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| {
            line.rsplit_once(',')
                .expect("every framemd5 record contains a hash")
                .1
                .trim()
                .to_owned()
        })
        .collect()
}

async fn first_audio_packet_micros(output: &Path) -> i64 {
    let probe = Command::new(required_path("ONMARK_FFPROBE"))
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "packet=pts_time",
            "-show_packets",
            "-of",
            "json",
            "--",
        ])
        .arg(output)
        .output();
    let probe = timeout(Duration::from_secs(10), probe)
        .await
        .expect("audio timestamp probing must finish before its deadline")
        .expect("ffprobe must inspect the output audio");
    assert!(
        probe.status.success(),
        "{}",
        String::from_utf8_lossy(&probe.stderr),
    );
    let response: AudioPacketProbe =
        serde_json::from_slice(&probe.stdout).expect("ffprobe must emit its JSON response");
    let packet = response
        .packets
        .first()
        .expect("the output audio stream must have a first packet");

    timestamp_micros(&packet.pts_time)
}

fn assert_audio_starts_at(output: &DecodedOutput, frame: u64) {
    let expected = i64::try_from(frame)
        .expect("the fixture frame fits in signed microseconds")
        .checked_mul(MICROS_PER_SECOND)
        .expect("the fixture timestamp fits in signed microseconds")
        / 30;
    assert!(
        output.audio_start_micros.abs_diff(expected) <= AUDIO_TIMESTAMP_TOLERANCE_MICROS,
        "audio starts at {}µs instead of frame {frame} ({expected}µs)",
        output.audio_start_micros,
    );
}

fn timestamp_micros(timestamp: &str) -> i64 {
    let (negative, timestamp) = timestamp
        .strip_prefix('-')
        .map_or((false, timestamp), |timestamp| (true, timestamp));
    let (seconds, fraction) = timestamp.split_once('.').unwrap_or((timestamp, ""));
    let seconds = seconds
        .parse::<i64>()
        .expect("the fixture packet timestamp has integral seconds");
    let mut micros = 0_i64;
    let mut digits = 0_u32;

    for digit in fraction.bytes().take(6) {
        assert!(digit.is_ascii_digit());
        micros = micros * 10 + i64::from(digit - b'0');
        digits += 1;
    }
    for _ in digits..6 {
        micros *= 10;
    }

    let micros = seconds
        .checked_mul(MICROS_PER_SECOND)
        .and_then(|seconds| seconds.checked_add(micros))
        .expect("the fixture packet timestamp fits in signed microseconds");
    if negative { -micros } else { micros }
}

fn headless_shell() -> PathBuf {
    required_path("ONMARK_HEADLESS_SHELL")
}

fn required_path(variable: &str) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{variable} must name an executable"))
}

fn browser_limits(deadline: Duration) -> BrowserLimits {
    BrowserLimits::new(deadline, 8 * 1024 * 1024).expect("the fixture browser limits are bounded")
}

fn render_profile() -> RenderProfile {
    RenderProfile::new(WIDTH, HEIGHT).expect("the fixture render profile is valid")
}

fn capture_environment() -> CaptureEnvironmentId {
    CaptureEnvironmentId::from_sha256([7; CaptureEnvironmentId::BYTE_LENGTH])
}

fn real_executor(max_frames: u64) -> RenderExecutor {
    let limits = EncodeLimits::new(
        Duration::from_secs(30),
        max_frames,
        64 * 1024 * 1024,
        64 * 1024,
    )
    .expect("the fixture encoding limits are bounded");
    let ffmpeg = Ffmpeg::new(required_path("ONMARK_FFMPEG"), limits)
        .expect("the FFmpeg executable path is present");

    RenderExecutor::new(
        headless_shell(),
        browser_limits(Duration::from_secs(10)),
        ffmpeg,
    )
}

fn frame_artifact_limits() -> FrameArtifactLimits {
    FrameArtifactLimits::new(TWO_UNIT_FRAME_COUNT, 64 * 1024 * 1024, 8 * 1024 * 1024)
        .expect("the fixture artifact limits are bounded")
}

fn browser_fixture() -> Url {
    let repository = repository();
    let fixture = repository.join("conformance/browser/gate-one.html");
    let runtime = repository.join("packages/runtime/dist/src/index.js");
    assert!(runtime.is_file(), "run `pnpm --dir packages/runtime build`");
    Url::from_file_path(fixture).expect("the fixture path is absolute")
}

async fn temporal_experiment_fixture(workspace: &Path) -> Url {
    let output = workspace.join("temporal-bundle");
    let bundled = Command::new(required_path("ONMARK_BUNDLER"))
        .args(["--entry"])
        .arg(repository().join("conformance/browser/temporal-experiment.ts"))
        .args(["--output"])
        .arg(&output)
        .args(["--max-output-bytes", "2000000"])
        .args(["--temporal-capability", "randomAccess"])
        .output();
    let bundled = timeout(Duration::from_secs(30), bundled)
        .await
        .expect("the experiment bundle must finish before its deadline")
        .expect("the presentation bundler must start");
    assert!(
        bundled.status.success(),
        "{}",
        String::from_utf8_lossy(&bundled.stderr),
    );
    let manifest = fs::read_to_string(output.join(BundleManifest::FILE_NAME))
        .expect("the experiment bundle must contain its manifest");
    let manifest: BundleManifest =
        serde_json::from_str(&manifest).expect("the experiment manifest must be valid");
    assert_eq!(
        manifest.temporal_capability(),
        PresentationTemporalCapability::RandomAccess,
    );

    Url::from_file_path(output.join(BundleManifest::ENTRY_POINT))
        .expect("the experiment bundle path is absolute")
}

fn render_fixture(name: &str) -> Url {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    Url::from_file_path(fixture).expect("the fixture path is absolute")
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("render is nested at crates/render")
        .to_owned()
}

fn gate_one_plan() -> BrowserPlan {
    BrowserPlan::from_timeline(&synthetic_timeline(), &BTreeMap::new())
        .expect("the fixture timeline fits the browser frame domain")
}

fn synthetic_timeline() -> onmark_core::timeline::TimelineIr {
    solve_timeline(
        r#"<film><scene><shot duration="2.5s"><title>Opening</title></shot></scene></film>"#,
        &BTreeMap::new(),
    )
}

fn solve_timeline(
    source: &str,
    assets: &BTreeMap<AssetRef, FrozenAsset>,
) -> onmark_core::timeline::TimelineIr {
    let frame_rate = FrameRate::new(30, 1).expect("the fixture frame rate is valid");
    let parsed = compiler::parse(SourceId::new(0), source);
    let (document, diagnostics) = parsed.into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::bind(document).into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::resolve(film.expect("the fixture binds")).into_parts();
    assert!(diagnostics.is_empty());
    let solved = compiler::solve(
        film.expect("the fixture resolves"),
        assets,
        Timebase::new(frame_rate),
    )
    .expect("the fixture metadata is complete");
    let (timeline, diagnostics) = solved.into_parts();
    assert!(diagnostics.is_empty());
    timeline.expect("the fixture solves")
}

fn executable_gate_one_unit(frozen: FrozenAsset, source: PathBuf) -> ExecutableUnit {
    let bundle = FixtureBundle::load();
    let timeline = gate_one_video_timeline(frozen.clone());
    let materialized =
        MaterializedAsset::new(frozen, source).expect("the fixture source path is present");
    let unit = RenderUnit::whole_film(
        &timeline,
        bundle.manifest.clone(),
        render_profile(),
        [materialized],
    )
    .expect("the fixture facts form one whole-film unit");

    bundle.materialize(unit)
}

fn gate_one_video_timeline(frozen: FrozenAsset) -> onmark_core::timeline::TimelineIr {
    let asset = AssetRef::parse("source.mp4").expect("the fixture asset reference is valid");
    let assets = BTreeMap::from([(asset, frozen)]);
    solve_timeline(
        r#"<film><scene><shot><video src="source.mp4" /></shot></scene></film>"#,
        &assets,
    )
}

struct GateFourFixture {
    partition_plan: PartitionPlan,
    whole_film: ExecutableUnit,
    partitioned_units: Vec<ExecutableUnit>,
}

impl GateFourFixture {
    async fn materialize(workspace: &Path) -> Self {
        let video_path = workspace.join("source.mp4");
        let voice_over_path = workspace.join("voice.m4a");
        let music_path = workspace.join("music.wav");
        let effect_path = workspace.join("effect.wav");
        generate_source_video(&video_path, "1").await;
        generate_voice_over(&voice_over_path).await;
        generate_audio(&music_path, 220, 44_100, 1, "2").await;
        generate_audio(&effect_path, 880, 48_000, 2, "0.25").await;

        let video = freeze_asset(&video_path).await;
        let voice_over = freeze_asset(&voice_over_path).await;
        let music = freeze_asset(&music_path).await;
        let effect = freeze_asset(&effect_path).await;
        let assets = BTreeMap::from([
            (
                AssetRef::parse("source.mp4").expect("the fixture video path is valid"),
                video.clone(),
            ),
            (
                AssetRef::parse("voice.m4a").expect("the fixture voice-over path is valid"),
                voice_over.clone(),
            ),
            (asset_ref("music.wav"), music.clone()),
            (asset_ref("effect.wav"), effect.clone()),
        ]);
        let source = fs::read_to_string(repository().join("conformance/cli/gate-four.onmark"))
            .expect("the Gate-four screenplay fixture is readable");
        let timeline = solve_timeline(&source, &assets);
        let timeline = compiler::import_captions(timeline, [caption_track()])
            .expect("fixture captions must enter the frame grid");
        let partition_plan =
            RenderGraph::from_timeline(&timeline, PresentationTemporalCapability::RandomAccess)
                .into_partition();
        assert_eq!(
            partition_plan.units().len(),
            2,
            "the random-access fixture must produce two local units",
        );

        let materialized_assets = vec![
            MaterializedAsset::new(video, video_path)
                .expect("the fixture video source path is present"),
            MaterializedAsset::new(voice_over, voice_over_path)
                .expect("the fixture voice-over source path is present"),
            MaterializedAsset::new(music, music_path)
                .expect("the fixture music source path is present"),
            MaterializedAsset::new(effect, effect_path)
                .expect("the fixture sound-effect source path is present"),
        ];
        let bundle = FixtureBundle::load();
        let whole_film = RenderUnit::whole_film(
            &timeline,
            bundle.manifest.clone(),
            render_profile(),
            materialized_assets.clone(),
        )
        .expect("the complete fixture forms one whole-film unit");
        let whole_film = bundle.materialize(whole_film);
        let partitioned_units: Vec<_> = partition_plan
            .units()
            .iter()
            .map(|partition| {
                let assets = materialized_assets
                    .iter()
                    .filter(|asset| partition.requires_media_asset(asset.id()))
                    .cloned();
                let unit = RenderUnit::from_partition(
                    &timeline,
                    partition,
                    bundle.manifest.clone(),
                    render_profile(),
                    assets,
                )
                .expect("each graph partition forms one local unit");

                bundle.materialize(unit)
            })
            .collect();
        assert!(partitioned_units.iter().all(|unit| {
            unit.browser_plan()
                .overlays()
                .iter()
                .any(|overlay| overlay.kind() == BrowserOverlayKind::Caption)
        }));

        Self {
            partition_plan,
            whole_film,
            partitioned_units,
        }
    }
}

fn asset_ref(value: &str) -> AssetRef {
    AssetRef::parse(value).expect("the fixture asset reference is portable")
}

fn caption_track() -> onmark_core::model::CaptionTrack {
    let source = b"WEBVTT\n\n00:00:00.750 --> 00:00:01.250\nAcross the partition\n";
    let limits =
        SubtitleLimits::new(source.len(), 1, 64).expect("the fixture subtitle limits are bounded");
    let report = parse_webvtt(SourceId::new(3), source, limits);
    let (track, errors) = report.into_parts();
    assert!(errors.is_empty());
    track.expect("the fixture subtitle is valid")
}

struct FixtureBundle {
    directory: PathBuf,
    manifest: BundleManifest,
}

impl FixtureBundle {
    fn load() -> Self {
        let directory = repository().join("conformance/protocol/bundle-v2");
        let manifest = fs::read_to_string(directory.join(BundleManifest::FILE_NAME))
            .expect("the executable bundle manifest is readable");
        let manifest: BundleManifest =
            serde_json::from_str(&manifest).expect("the executable bundle manifest is valid");
        assert_eq!(
            manifest.temporal_capability(),
            PresentationTemporalCapability::RandomAccess,
        );

        Self {
            directory,
            manifest,
        }
    }

    fn materialize(&self, unit: RenderUnit) -> ExecutableUnit {
        let limits = UnitRootLimits::new(8, 64 * 1024 * 1024)
            .expect("the fixture materialization limits are bounded");

        ExecutableUnit::materialize(unit, &self.directory, limits)
            .expect("the fixture bundle must become one executable unit")
    }
}

#[derive(Debug, Deserialize)]
struct ProbeResponse {
    streams: Vec<ProbeStream>,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    width: u32,
    height: u32,
    avg_frame_rate: String,
    nb_read_frames: String,
}

#[derive(Debug, Deserialize)]
struct AudioPacketProbe {
    packets: Vec<AudioPacket>,
}

#[derive(Debug, Deserialize)]
struct AudioPacket {
    pts_time: String,
}

#[test]
fn parses_audio_packet_timestamps_without_floating_point() {
    assert_eq!(timestamp_micros("0.978"), 978_000);
    assert_eq!(timestamp_micros("-0.021333"), -21_333);
}
