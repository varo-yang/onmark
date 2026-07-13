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
const FRAME_COUNT: usize = 30;
const PROCESS_DEADLINE: Duration = Duration::from_mins(3);

#[tokio::test]
#[ignore = "requires ONMARK_CHROME, ONMARK_BUNDLER, ONMARK_FFMPEG, and ONMARK_FFPROBE"]
async fn renders_one_screenplay_deterministically_across_real_processes() {
    let directory = tempdir().expect("the conformance workspace is available");
    let fixture = Fixture::materialize(directory.path());
    fixture.generate_source_video().await;

    let first = fixture.render("first.mp4").await;
    let second = fixture.render("second.mp4").await;
    assert_success(&first.output);
    assert_success(&second.output);

    let first_video = inspect_video(&first.path).await;
    let second_video = inspect_video(&second.path).await;
    assert_eq!(first_video, second_video);
    assert_eq!(first_video.frame_hashes.len(), FRAME_COUNT);
    assert!(first_video.has_motion());

    let original_digest = file_digest(&first.path);
    let rejected = fixture.render_to(&first.path).await;
    assert_eq!(rejected.status.code(), Some(2));
    assert!(rejected.stdout.is_empty());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("already exists"));
    assert_eq!(file_digest(&first.path), original_digest);
}

struct Fixture {
    root: PathBuf,
    screenplay: PathBuf,
    presentation: PathBuf,
}

impl Fixture {
    fn materialize(root: &Path) -> Self {
        let repository = repository();
        let screenplay = root.join("gate-one.onmark");
        let presentation = root.join("presentation.ts");
        copy_fixture(&repository, "cli/gate-one.onmark", &screenplay);
        copy_fixture(&repository, "browser/video-presentation.ts", &presentation);
        copy_fixture(
            &repository,
            "browser/video-presentation.css",
            &root.join("video-presentation.css"),
        );

        Self {
            root: root.to_owned(),
            screenplay,
            presentation,
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

    async fn render(&self, name: &str) -> RenderedOutput {
        let path = self.root.join(name);
        let output = self.render_to(&path).await;
        RenderedOutput { path, output }
    }

    async fn render_to(&self, output: &Path) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_onmark"));
        command
            .arg("render")
            .arg(&self.screenplay)
            .arg("--presentation")
            .arg(&self.presentation)
            .arg("--output")
            .arg(output)
            .arg("--width")
            .arg(WIDTH.to_string())
            .arg("--height")
            .arg(HEIGHT.to_string())
            .arg("--browser")
            .arg(required_path("ONMARK_CHROME"))
            .arg("--bundler")
            .arg(required_path("ONMARK_BUNDLER"))
            .arg("--ffmpeg")
            .arg(required_path("ONMARK_FFMPEG"))
            .arg("--ffprobe")
            .arg(required_path("ONMARK_FFPROBE"));
        run_process(&mut command).await
    }
}

struct RenderedOutput {
    path: PathBuf,
    output: Output,
}

#[derive(Debug, Eq, PartialEq)]
struct InspectedVideo {
    stream: ProbeStream,
    frame_hashes: Vec<String>,
}

impl InspectedVideo {
    fn has_motion(&self) -> bool {
        let Some(first) = self.frame_hashes.first() else {
            return false;
        };
        self.frame_hashes.iter().any(|hash| hash != first)
    }
}

async fn inspect_video(path: &Path) -> InspectedVideo {
    InspectedVideo {
        stream: probe_stream(path).await,
        frame_hashes: decode_frame_hashes(path).await,
    }
}

async fn probe_stream(path: &Path) -> ProbeStream {
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
    let response: ProbeResponse =
        serde_json::from_slice(&output.stdout).expect("ffprobe emits valid JSON");
    let [stream]: [ProbeStream; 1] = response
        .streams
        .try_into()
        .expect("ffprobe must report exactly one video stream");
    assert_eq!(stream.codec_name, "h264");
    assert_eq!(stream.width, WIDTH);
    assert_eq!(stream.height, HEIGHT);
    assert_eq!(stream.avg_frame_rate, "30/1");
    assert_eq!(stream.nb_read_frames, FRAME_COUNT.to_string());
    stream
}

async fn decode_frame_hashes(path: &Path) -> Vec<String> {
    let output = run_process(
        Command::new(required_path("ONMARK_FFMPEG"))
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(path)
            .args(["-f", "framemd5", "-"]),
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

fn assert_success(output: &Output) {
    assert_process_success("CLI rendering", output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&format!("Rendered {FRAME_COUNT} frames")));
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
struct ProbeResponse {
    streams: Vec<ProbeStream>,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct ProbeStream {
    codec_name: String,
    width: u32,
    height: u32,
    avg_frame_rate: String,
    nb_read_frames: String,
}
