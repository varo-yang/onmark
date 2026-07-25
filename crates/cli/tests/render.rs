//! Opt-in release-CLI conformance across compilation, Chromium, and FFmpeg.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use tempfile::tempdir;
use tokio::process::Command;
use tokio::time::timeout;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const FRAMES_PER_SECOND: u64 = 30;
const MICROS_PER_SECOND: u64 = 1_000_000;
const AUDIO_SAMPLE_RATE: u64 = 48_000;
const AUDIBLE_SAMPLE_THRESHOLD: u16 = 256;
// AAC can spread a transient across one 1024-sample coding frame.
const AUDIO_START_TOLERANCE_MICROS: u64 = 25_000;
const DESKTOP_FRAME_COUNT: usize = 45;
const PARTITIONED_FRAME_COUNT: usize = 60;
const PROCESS_DEADLINE: Duration = Duration::from_mins(3);
const CACHE_ENVIRONMENT: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";

#[tokio::test]
#[ignore = "requires ONMARK_CLI, ONMARK_FFMPEG, ONMARK_FFPROBE, and browser tools on PATH"]
async fn renders_one_screenplay_reliably_across_real_processes() {
    let directory = tempdir().expect("the conformance workspace is available");
    let fixture = Fixture::materialize(directory.path(), "cli/desktop-release.html");
    let first = render_fixture_twice(&fixture, SourceVideo::Solid, DESKTOP_FRAME_COUNT, 15).await;
    assert!(
        first.inspection.has_motion_before(10),
        "the static source must expose exact-frame GSAP motion before the CTA boundary",
    );

    let original_digest = file_digest(&first.path);
    let rejected = fixture.render_to(&first.path).await;
    assert_eq!(rejected.status.code(), Some(2));
    assert!(rejected.stdout.is_empty());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("already exists"));
    assert_eq!(file_digest(&first.path), original_digest);
}

#[tokio::test]
#[ignore = "requires ONMARK_CLI, ONMARK_FFMPEG, ONMARK_FFPROBE, and browser tools on PATH"]
async fn assembles_two_partitioned_units_across_real_processes() {
    let directory = tempdir().expect("the conformance workspace is available");
    let fixture = Fixture::materialize(directory.path(), "cli/audio-subtitle.html");
    fixture.generate_general_audio().await;

    render_fixture_twice(&fixture, SourceVideo::Moving, PARTITIONED_FRAME_COUNT, 0).await;
}

#[tokio::test]
#[ignore = "requires ONMARK_CLI, ONMARK_BUNDLER, ONMARK_FFMPEG, and a discoverable browser"]
async fn reuses_only_unchanged_regions_across_cli_processes() {
    let directory = tempdir().expect("the conformance workspace is available");
    let fixture = Fixture::write(directory.path(), &incremental_film("Closing"));
    let cache = directory.path().join("frame-cache");

    let first = fixture.render_cached("first.mp4", &cache).await;
    assert_success(&first.output, PARTITIONED_FRAME_COUNT);
    assert_incremental_summary(&first.output, "Reused 0/2 regions and 0/60 frames");
    let first_hashes = decode_video_hashes(&first.path).await;
    let first_artifacts = cache_artifacts(&cache);
    assert_eq!(first_artifacts.len(), 2);

    let warm = fixture.render_cached("warm.mp4", &cache).await;
    assert_success(&warm.output, PARTITIONED_FRAME_COUNT);
    assert_incremental_summary(&warm.output, "Reused 2/2 regions and 60/60 frames");
    assert_eq!(decode_video_hashes(&warm.path).await, first_hashes);
    assert_eq!(cache_artifacts(&cache), first_artifacts);

    fixture.replace_screenplay(&incremental_film("Changed"));
    let second = fixture.render_cached("second.mp4", &cache).await;
    assert_success(&second.output, PARTITIONED_FRAME_COUNT);
    assert_incremental_summary(&second.output, "Reused 1/2 regions and 30/60 frames");
    let second_hashes = decode_video_hashes(&second.path).await;
    let second_artifacts = cache_artifacts(&cache);
    assert_eq!(second_artifacts.len(), 3);
    assert_eq!(first_hashes.len(), PARTITIONED_FRAME_COUNT);
    assert_eq!(second_hashes.len(), PARTITIONED_FRAME_COUNT);
    assert_ne!(first_hashes, second_hashes);

    let new_artifact = second_artifacts
        .iter()
        .find(|path| !first_artifacts.contains(path))
        .expect("the edited region contributes one new artifact");
    fs::write(new_artifact, b"corrupt").expect("the cache fixture can be corrupted");

    let repaired = fixture.render_cached("repaired.mp4", &cache).await;
    assert_success(&repaired.output, PARTITIONED_FRAME_COUNT);
    assert_incremental_summary(&repaired.output, "Reused 1/2 regions and 30/60 frames");
    assert_eq!(decode_video_hashes(&repaired.path).await, second_hashes);
    assert_eq!(cache_artifacts(&cache), second_artifacts);
    assert!(
        new_artifact
            .metadata()
            .expect("the repaired artifact has metadata")
            .len()
            > b"corrupt".len() as u64,
    );
}

async fn render_fixture_twice(
    fixture: &Fixture,
    source_video: SourceVideo,
    expected_frames: usize,
    audio_start_frame: u64,
) -> VerifiedOutput {
    fixture.generate_source_video(source_video).await;
    fixture.generate_voice_over().await;

    let first = fixture.render("first.mp4").await;
    let second = fixture.render("second.mp4").await;
    assert_success(&first.output, expected_frames);
    assert_success(&second.output, expected_frames);

    let first_output = inspect_output(&first.path, expected_frames).await;
    let second_output = inspect_output(&second.path, expected_frames).await;
    assert_media_contract(&first_output, expected_frames);
    assert_media_contract(&second_output, expected_frames);
    assert_audio_begins_at_frame(&first.path, audio_start_frame).await;
    assert_audio_begins_at_frame(&second.path, audio_start_frame).await;

    VerifiedOutput {
        path: first.path,
        inspection: first_output,
    }
}

fn assert_media_contract(output: &InspectedOutput, expected_frames: usize) {
    assert_eq!(output.video_frame_hashes.len(), expected_frames);
    assert!(output.has_motion());
    assert!(!output.audio_frame_hashes.is_empty());
}

struct Fixture {
    root: PathBuf,
    screenplay: PathBuf,
}

#[derive(Clone, Copy)]
enum SourceVideo {
    Moving,
    Solid,
}

impl SourceVideo {
    fn lavfi_source(self) -> String {
        match self {
            Self::Moving => format!("testsrc2=size={WIDTH}x{HEIGHT}:rate=30:duration=1"),
            Self::Solid => format!("color=c=0x17406d:size={WIDTH}x{HEIGHT}:rate=30:duration=1"),
        }
    }
}

impl Fixture {
    fn materialize(root: &Path, screenplay_fixture: &str) -> Self {
        let repository = repository();
        let screenplay = root.join("film.html");
        copy_fixture(&repository, screenplay_fixture, &screenplay);

        Self {
            root: root.to_owned(),
            screenplay,
        }
    }

    fn write(root: &Path, source: &str) -> Self {
        let screenplay = root.join("film.html");
        fs::write(&screenplay, source).expect("the screenplay fixture is writable");
        Self {
            root: root.to_owned(),
            screenplay,
        }
    }

    fn replace_screenplay(&self, source: &str) {
        fs::write(&self.screenplay, source).expect("the screenplay fixture is writable");
    }

    async fn generate_source_video(&self, video: SourceVideo) {
        let source = video.lavfi_source();
        let output = run_process(
            Command::new(required_path("ONMARK_FFMPEG"))
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
                .arg(self.root.join("source.mp4")),
        )
        .await;
        assert_process_success("source generation", &output);
    }

    async fn generate_voice_over(&self) {
        let output = run_process(
            Command::new(required_path("ONMARK_FFMPEG"))
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
                .arg(self.root.join("voice.m4a")),
        )
        .await;
        assert_process_success("voice-over generation", &output);
    }

    async fn generate_general_audio(&self) {
        self.generate_audio("music.wav", 220, "2").await;
        self.generate_audio("effect.wav", 880, "0.25").await;
    }

    async fn generate_audio(&self, filename: &str, frequency: u32, duration: &str) {
        let source = format!("sine=frequency={frequency}:sample_rate=48000:duration={duration}");
        let output = run_process(
            Command::new(required_path("ONMARK_FFMPEG"))
                .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i", &source])
                .args(["-ac", "2", "-c:a", "pcm_s16le", "-y"])
                .arg(self.root.join(filename)),
        )
        .await;
        assert_process_success("general-audio generation", &output);
    }

    async fn render(&self, name: &str) -> RenderAttempt {
        let path = self.root.join(name);
        let output = self.render_to(&path).await;
        RenderAttempt { path, output }
    }

    async fn render_to(&self, output: &Path) -> Output {
        let mut command = self.render_command(output);
        for (flag, variable) in [
            ("--browser", "ONMARK_HEADLESS_SHELL"),
            ("--bundler", "ONMARK_BUNDLER"),
            ("--ffmpeg", "ONMARK_FFMPEG"),
            ("--ffprobe", "ONMARK_FFPROBE"),
        ] {
            if let Some(path) = env::var_os(variable) {
                command.arg(flag).arg(path);
            }
        }
        run_process(&mut command).await
    }

    async fn render_cached(&self, name: &str, cache: &Path) -> RenderAttempt {
        let path = self.root.join(name);
        let mut command = self.render_command(&path);
        command
            .env("ONMARK_FRAME_CACHE", cache)
            .env("ONMARK_CAPTURE_ENVIRONMENT_SEED", CACHE_ENVIRONMENT);
        for (flag, variable) in [
            ("--bundler", "ONMARK_BUNDLER"),
            ("--ffmpeg", "ONMARK_FFMPEG"),
            ("--ffprobe", "ONMARK_FFPROBE"),
        ] {
            if let Some(path) = env::var_os(variable) {
                command.arg(flag).arg(path);
            }
        }
        let output = run_process(&mut command).await;
        RenderAttempt { path, output }
    }

    fn render_command(&self, output: &Path) -> Command {
        let mut command = Command::new(required_path("ONMARK_CLI"));
        command
            .arg("render")
            .arg(&self.screenplay)
            .arg("--output")
            .arg(output)
            .arg("--width")
            .arg(WIDTH.to_string())
            .arg("--height")
            .arg(HEIGHT.to_string());
        command
    }
}

struct RenderAttempt {
    path: PathBuf,
    output: Output,
}

struct VerifiedOutput {
    path: PathBuf,
    inspection: InspectedOutput,
}

struct InspectedOutput {
    video_frame_hashes: Vec<String>,
    audio_frame_hashes: Vec<String>,
}

impl InspectedOutput {
    fn has_motion(&self) -> bool {
        let Some(first) = self.video_frame_hashes.first() else {
            return false;
        };
        self.video_frame_hashes.iter().any(|hash| hash != first)
    }

    fn has_motion_before(&self, end: usize) -> bool {
        let Some(first) = self.video_frame_hashes.first() else {
            return false;
        };
        self.video_frame_hashes
            .iter()
            .take(end)
            .any(|hash| hash != first)
    }
}

async fn inspect_output(path: &Path, expected_frames: usize) -> InspectedOutput {
    probe_video_stream(path, expected_frames).await;
    probe_audio_stream(path).await;

    InspectedOutput {
        video_frame_hashes: decode_video_hashes(path).await,
        audio_frame_hashes: decode_audio_hashes(path).await,
    }
}

async fn probe_video_stream(path: &Path, expected_frames: usize) {
    let output = run_process(
        Command::new(required_path("ONMARK_FFPROBE"))
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-count_frames",
                "-show_entries",
                "stream=codec_name,width,height,avg_frame_rate,nb_read_frames",
                "-of",
                "json",
                "--",
            ])
            .arg(path),
    )
    .await;
    assert_process_success("output probing", &output);
    let response: VideoProbeResponse =
        serde_json::from_slice(&output.stdout).expect("ffprobe emits valid JSON");
    let [stream]: [VideoStream; 1] = response
        .streams
        .try_into()
        .expect("ffprobe must report exactly one video stream");
    assert_eq!(stream.codec_name, "h264");
    assert_eq!(stream.width, WIDTH);
    assert_eq!(stream.height, HEIGHT);
    assert_eq!(stream.avg_frame_rate, "30/1");
    assert_eq!(stream.nb_read_frames, expected_frames.to_string());
}

async fn probe_audio_stream(path: &Path) {
    let output = run_process(
        Command::new(required_path("ONMARK_FFPROBE"))
            .args([
                "-v",
                "error",
                "-select_streams",
                "a:0",
                "-show_entries",
                "stream=codec_name,sample_rate,channels",
                "-of",
                "json",
                "--",
            ])
            .arg(path),
    )
    .await;
    assert_process_success("audio output probing", &output);
    let response: AudioProbeResponse =
        serde_json::from_slice(&output.stdout).expect("ffprobe emits valid JSON");
    let [stream]: [AudioStream; 1] = response
        .streams
        .try_into()
        .expect("ffprobe must report exactly one audio stream");
    assert_eq!(stream.codec_name, "aac");
    assert_eq!(stream.sample_rate, "48000");
    assert_eq!(stream.channels, 2);
}

async fn decode_video_hashes(path: &Path) -> Vec<String> {
    let output = run_process(
        Command::new(required_path("ONMARK_FFMPEG"))
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(path)
            .args(["-map", "0:v:0", "-f", "framemd5", "-"]),
    )
    .await;
    assert_process_success("frame decoding", &output);

    String::from_utf8(output.stdout)
        .expect("framemd5 output is UTF-8")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(frame_hash)
        .collect()
}

async fn decode_audio_hashes(path: &Path) -> Vec<String> {
    let output = run_process(
        Command::new(required_path("ONMARK_FFMPEG"))
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(path)
            .args(["-map", "0:a:0", "-f", "framemd5", "-"]),
    )
    .await;
    assert_process_success("audio decoding", &output);

    String::from_utf8(output.stdout)
        .expect("framemd5 output is UTF-8")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(frame_hash)
        .collect()
}

async fn assert_audio_begins_at_frame(path: &Path, frame: u64) {
    let output = run_process(
        Command::new(required_path("ONMARK_FFMPEG"))
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(path)
            .args([
                "-map",
                "0:a:0",
                "-f",
                "s16le",
                "-acodec",
                "pcm_s16le",
                "-ar",
            ])
            .arg(AUDIO_SAMPLE_RATE.to_string())
            .args(["-ac", "1", "-"]),
    )
    .await;
    assert_process_success("audio sample decoding", &output);

    let actual = first_audible_sample(&output.stdout)
        .expect("the fixture output contains audible PCM samples");
    let actual = u64::try_from(actual).expect("the bounded fixture length fits in u64")
        * MICROS_PER_SECOND
        / AUDIO_SAMPLE_RATE;
    let expected = frame * MICROS_PER_SECOND / FRAMES_PER_SECOND;

    assert!(
        actual.abs_diff(expected) <= AUDIO_START_TOLERANCE_MICROS,
        "audio starts at {actual}µs instead of frame {frame} ({expected}µs)",
    );
}

fn first_audible_sample(pcm: &[u8]) -> Option<usize> {
    let mut samples = pcm.chunks_exact(2);
    let audible = samples.position(|sample| {
        i16::from_le_bytes([sample[0], sample[1]]).unsigned_abs() >= AUDIBLE_SAMPLE_THRESHOLD
    });
    assert!(
        samples.remainder().is_empty(),
        "decoded PCM is frame-aligned"
    );
    audible
}

#[test]
fn finds_the_first_audible_pcm_sample() {
    let pcm = [0_i16, 255, -256, 1_024]
        .into_iter()
        .flat_map(i16::to_le_bytes)
        .collect::<Vec<_>>();

    assert_eq!(first_audible_sample(&pcm), Some(2));
}

fn frame_hash(record: &str) -> String {
    record
        .rsplit_once(',')
        .expect("every framemd5 record contains a hash")
        .1
        .trim()
        .to_owned()
}

async fn run_process(command: &mut Command) -> Output {
    command.kill_on_drop(true);
    timeout(PROCESS_DEADLINE, command.output())
        .await
        .expect("the real-process conformance deadline is bounded")
        .expect("the conformance process starts")
}

fn assert_success(output: &Output, expected_frames: usize) {
    assert_process_success("CLI rendering", output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&format!("Rendered {expected_frames} frames")));
    let capture_mode = if cfg!(target_os = "linux") {
        "beginFrame"
    } else {
        "screenshot"
    };
    assert!(stdout.contains(&format!("with {capture_mode} capture")));
    let graphics_backend = if cfg!(target_os = "macos") {
        "Metal"
    } else {
        "SwiftShader"
    };
    assert!(stdout.contains(&format!("on {graphics_backend}")));
    assert!(stdout.contains("Timing: prepare "));
    assert!(stdout.contains(", bundle "));
    assert!(stdout.contains(", plan "));
    assert!(stdout.contains(", capture "));
    assert!(stdout.contains(", assemble "));
    assert!(stdout.contains(", total "));
}

fn assert_incremental_summary(output: &Output, expected: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(expected),
        "CLI output does not contain {expected:?}:\n{stdout}",
    );
}

fn assert_process_success(operation: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{operation} failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

fn file_digest(path: &Path) -> [u8; 32] {
    let bytes = fs::read(path).expect("the bounded conformance output is readable");
    Sha256::digest(bytes).into()
}

fn cache_artifacts(directory: &Path) -> Vec<PathBuf> {
    let mut artifacts = fs::read_dir(directory)
        .expect("the frame cache is readable")
        .map(|entry| entry.expect("the cache entry is readable").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "onmark-frames")
        })
        .collect::<Vec<_>>();
    artifacts.sort();
    artifacts
}

fn incremental_film(closing: &str) -> String {
    format!(
        r#"<!doctype html>
<om-film>
  <style>
    html, body, om-film, om-scene, om-shot {{
      height: 100%;
      margin: 0;
      width: 100%;
    }}
    body {{ background: black; color: white; }}
    om-title {{
      display: grid;
      font: 700 42px sans-serif;
      inset: 0;
      place-items: center;
      position: fixed;
    }}
  </style>
  <om-scene>
    <om-shot duration="1s"><om-title>Opening</om-title></om-shot>
    <om-shot duration="1s"><om-title>{closing}</om-title></om-shot>
  </om-scene>
</om-film>
"#,
    )
}

fn copy_fixture(repository: &Path, source: &str, destination: &Path) {
    fs::copy(repository.join("conformance").join(source), destination)
        .expect("the conformance fixture is copied");
}

fn required_path(variable: &str) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{variable} must name an executable"))
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("cli is nested at crates/cli")
        .to_owned()
}

#[derive(Debug, Deserialize)]
struct VideoProbeResponse {
    streams: Vec<VideoStream>,
}

#[derive(Debug, Deserialize)]
struct AudioProbeResponse {
    streams: Vec<AudioStream>,
}

#[derive(Debug, Deserialize)]
struct VideoStream {
    codec_name: String,
    width: u32,
    height: u32,
    avg_frame_rate: String,
    nb_read_frames: String,
}

#[derive(Debug, Deserialize)]
struct AudioStream {
    codec_name: String,
    sample_rate: String,
    channels: u32,
}
