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
// One 48 kHz AAC packet spans roughly 21.3 ms; leave room for packet rounding.
const AAC_PACKET_TIMESTAMP_TOLERANCE_MICROS: u64 = 25_000;
const GATE_ONE_FRAME_COUNT: usize = 45;
const GATE_TWO_FRAME_COUNT: usize = 60;
const PROCESS_DEADLINE: Duration = Duration::from_mins(3);

#[tokio::test]
#[ignore = "requires ONMARK_CLI, ONMARK_FFMPEG, ONMARK_FFPROBE, and Gate-one tools on PATH"]
async fn renders_one_screenplay_reliably_across_real_processes() {
    let directory = tempdir().expect("the conformance workspace is available");
    let fixture = Fixture::materialize(directory.path(), "cli/gate-one.onmark");
    let first = render_fixture_twice(&fixture, GATE_ONE_FRAME_COUNT, 15).await;

    let original_digest = file_digest(&first.path);
    let rejected = fixture.render_to(&first.path).await;
    assert_eq!(rejected.status.code(), Some(2));
    assert!(rejected.stdout.is_empty());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("already exists"));
    assert_eq!(file_digest(&first.path), original_digest);
}

#[tokio::test]
#[ignore = "requires ONMARK_CLI, ONMARK_FFMPEG, ONMARK_FFPROBE, and Gate-two tools on PATH"]
async fn assembles_two_partitioned_units_across_real_processes() {
    let directory = tempdir().expect("the conformance workspace is available");
    let fixture = Fixture::materialize(directory.path(), "cli/gate-two.onmark");

    render_fixture_twice(&fixture, GATE_TWO_FRAME_COUNT, 30).await;
}

async fn render_fixture_twice(
    fixture: &Fixture,
    expected_frames: usize,
    audio_start_frame: u64,
) -> RenderedOutput {
    fixture.generate_source_video().await;
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

    first
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

impl Fixture {
    fn materialize(root: &Path, screenplay_fixture: &str) -> Self {
        let repository = repository();
        let screenplay = root.join("film.onmark");
        copy_fixture(&repository, screenplay_fixture, &screenplay);
        copy_fixture(
            &repository,
            "browser/video-presentation.ts",
            &root.join("presentation.ts"),
        );
        copy_fixture(
            &repository,
            "browser/video-presentation.css",
            &root.join("video-presentation.css"),
        );

        Self {
            root: root.to_owned(),
            screenplay,
        }
    }

    async fn generate_source_video(&self) {
        let source = format!("testsrc2=size={WIDTH}x{HEIGHT}:rate=30:duration=1");
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

    async fn render(&self, name: &str) -> RenderedOutput {
        let path = self.root.join(name);
        let output = self.render_to(&path).await;
        RenderedOutput { path, output }
    }

    async fn render_to(&self, output: &Path) -> Output {
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
}

struct RenderedOutput {
    path: PathBuf,
    output: Output,
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
        Command::new(required_path("ONMARK_FFPROBE"))
            .args(["-v", "error", "-select_streams", "a:0"])
            .arg(path)
            .args([
                "-show_entries",
                "packet=pts_time",
                "-show_packets",
                "-of",
                "json",
            ]),
    )
    .await;
    assert_process_success("audio timestamp probing", &output);

    let response: AudioPacketProbe =
        serde_json::from_slice(&output.stdout).expect("ffprobe emits valid JSON");
    let packet = response
        .packets
        .first()
        .expect("the output audio stream has a first packet");
    let actual = timestamp_micros(&packet.pts_time);
    let expected = frame * MICROS_PER_SECOND / FRAMES_PER_SECOND;

    // AAC priming can move the first packet by one encoded audio frame, while
    // raw PCM output drops its timestamp entirely.
    assert!(
        actual.abs_diff(expected) <= AAC_PACKET_TIMESTAMP_TOLERANCE_MICROS,
        "audio starts at {actual}µs instead of frame {frame} ({expected}µs)",
    );
}

fn timestamp_micros(timestamp: &str) -> u64 {
    let (seconds, fraction) = timestamp.split_once('.').unwrap_or((timestamp, ""));
    let seconds = seconds
        .parse::<u64>()
        .expect("the fixture packet timestamp is non-negative seconds");
    let mut micros = 0_u64;
    let mut digits = 0_u32;

    for digit in fraction.bytes().take(6) {
        assert!(digit.is_ascii_digit());
        micros = micros * 10 + u64::from(digit - b'0');
        digits += 1;
    }
    for _ in digits..6 {
        micros *= 10;
    }

    seconds * MICROS_PER_SECOND + micros
}

#[test]
fn parses_ffprobe_packet_timestamps_without_floating_point() {
    assert_eq!(timestamp_micros("0.978000"), 978_000);
    assert_eq!(timestamp_micros("1.2"), 1_200_000);
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
struct AudioPacketProbe {
    packets: Vec<AudioPacket>,
}

#[derive(Debug, Deserialize)]
struct AudioPacket {
    pts_time: String,
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
