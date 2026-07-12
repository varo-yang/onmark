use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use onmark_core::compiler;
use onmark_core::model::{AssetMetadata, AssetRef, Duration as MediaDuration};
use onmark_core::model::{FrameRate, SourceId, Timebase};
use onmark_core::protocol::{
    BrowserCommand, BrowserEvent, BrowserPlan, BrowserRequest, RequestId, WireFrame,
};
use onmark_render::{
    BrowserErrorKind, BrowserLimits, BrowserSession, EncodeLimits, EncodedPng, Ffmpeg,
    RenderExecutor,
};
use serde::Deserialize;
use tempfile::tempdir;
use tokio::process::Command;
use tokio::time::timeout;
use url::Url;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const FRAME_COUNT: u64 = 75;

#[tokio::test]
#[ignore = "requires ONMARK_CHROME"]
async fn rejects_a_page_that_never_installs_the_runtime_host() {
    let session = BrowserSession::launch(chrome(), browser_limits(Duration::from_secs(5)))
        .await
        .expect("Chrome must launch");
    let fixture = render_fixture("missing-runtime.html");

    let error = session
        .navigate(fixture.as_str())
        .await
        .expect_err("the missing host must miss its readiness deadline");
    let shutdown = session.shutdown().await;

    assert_eq!(error.kind(), BrowserErrorKind::RuntimeHost);
    shutdown.expect("Chrome must shut down after a readiness failure");
}

#[tokio::test]
#[ignore = "requires ONMARK_CHROME and a built @onmark/runtime package"]
async fn bounds_a_runtime_adapter_that_never_finishes_loading() {
    let session = BrowserSession::launch(chrome(), browser_limits(Duration::from_secs(5)))
        .await
        .expect("Chrome must launch");
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
    shutdown.expect("Chrome must shut down after a protocol timeout");
}

#[tokio::test]
#[ignore = "requires ONMARK_CHROME and a built @onmark/runtime package"]
async fn captures_stable_frames_across_the_real_browser_protocol() {
    let session = BrowserSession::launch(chrome(), browser_limits(Duration::from_secs(10)))
        .await
        .expect("Chrome must launch");
    let fixture = browser_fixture();

    let result = exercise_protocol(&session, &fixture).await;
    let shutdown = session.shutdown().await;

    result.expect("the real browser protocol must capture deterministic frames");
    shutdown.expect("Chrome must shut down cleanly");
}

#[tokio::test]
#[ignore = "requires ONMARK_CHROME, ONMARK_FFMPEG, ONMARK_FFPROBE, and built runtime"]
async fn renders_the_gate_one_plan_to_a_verified_mp4() {
    let directory = tempdir().expect("the test output directory must be available");
    let output = directory.path().join("gate-one.mp4");
    let limits = EncodeLimits::new(Duration::from_secs(30), 100, 64 * 1024 * 1024, 64 * 1024)
        .expect("the fixture encoding limits are bounded");
    let ffmpeg = Ffmpeg::new(required_path("ONMARK_FFMPEG"), limits)
        .expect("the FFmpeg executable path is present");
    let executor = RenderExecutor::new(chrome(), browser_limits(Duration::from_secs(10)), ffmpeg);

    let video = executor
        .render(gate_one_plan(), browser_fixture().as_str(), &output)
        .await
        .expect("the real local renderer must produce an MP4");

    assert_eq!(video.path(), output);
    assert_eq!(video.frames(), FRAME_COUNT);
    assert!(output.metadata().expect("the MP4 must exist").len() > 0);
    assert_video_stream(&output).await;
    assert_decodable(&output).await;
}

async fn exercise_protocol(session: &BrowserSession, fixture: &Url) -> Result<(), Box<dyn Error>> {
    session.navigate(fixture.as_str()).await?;
    let plan = gate_one_plan();

    let loaded = session
        .dispatch(&BrowserRequest::new(
            RequestId::new(1),
            BrowserCommand::Load { plan },
        ))
        .await?;
    assert_eq!(loaded.event(), &BrowserEvent::Loaded);

    let prepared = session
        .dispatch(&BrowserRequest::new(
            RequestId::new(2),
            BrowserCommand::Prepare {
                evaluation_start: frame(0),
            },
        ))
        .await?;
    assert_eq!(
        prepared.event(),
        &BrowserEvent::Prepared {
            evaluation_start: frame(0),
        },
    );
    let first = session.capture_png().await?;

    seek(session, 3, 15).await?;
    let selected = session.capture_png().await?;
    seek(session, 4, 15).await?;
    let repeated = session.capture_png().await?;

    assert_png(&first);
    assert_ne!(first, selected);
    assert_eq!(selected, repeated);
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

fn frame(index: u64) -> WireFrame {
    WireFrame::new(index).expect("fixture frames are browser-safe")
}

fn assert_png(frame: &EncodedPng) {
    assert!(frame.as_bytes().starts_with(b"\x89PNG\r\n\x1a\n"));
}

async fn assert_video_stream(output: &Path) {
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
    assert_eq!(stream.nb_read_frames, FRAME_COUNT.to_string());
}

async fn assert_decodable(output: &Path) {
    let decoded = Command::new(required_path("ONMARK_FFMPEG"))
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(output)
        .args(["-f", "null", "-"])
        .output();
    let decoded = timeout(Duration::from_secs(10), decoded)
        .await
        .expect("FFmpeg decode must finish before the conformance deadline")
        .expect("FFmpeg must decode the completed MP4");
    assert!(
        decoded.status.success(),
        "{}",
        String::from_utf8_lossy(&decoded.stderr)
    );
}

fn chrome() -> PathBuf {
    required_path("ONMARK_CHROME")
}

fn required_path(variable: &str) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{variable} must name an executable"))
}

fn browser_limits(deadline: Duration) -> BrowserLimits {
    BrowserLimits::new(WIDTH, HEIGHT, deadline, 8 * 1024 * 1024)
        .expect("the fixture browser limits are bounded")
}

fn browser_fixture() -> Url {
    let repository = repository();
    let fixture = repository.join("conformance/browser/gate-one.html");
    let runtime = repository.join("packages/runtime/dist/src/index.js");
    assert!(runtime.is_file(), "run `pnpm --dir packages/runtime build`");
    Url::from_file_path(fixture).expect("the fixture path is absolute")
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
    let asset = AssetRef::parse("opening.mp4").expect("the fixture asset is valid");
    let assets = BTreeMap::from([(
        asset,
        AssetMetadata::new(MediaDuration::from_nanos(2_500_000_000)),
    )]);
    let parsed = compiler::parse(
        SourceId::new(0),
        r#"<film><scene><shot><video src="opening.mp4" /></shot></scene></film>"#,
    );
    let (document, diagnostics) = parsed.into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::bind(document).into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::resolve(film.expect("the fixture binds")).into_parts();
    assert!(diagnostics.is_empty());
    let rate = FrameRate::new(30, 1).expect("the fixture frame rate is valid");
    let solved = compiler::solve(
        film.expect("the fixture resolves"),
        &assets,
        Timebase::new(rate),
    )
    .expect("the fixture metadata is complete");
    assert!(solved.diagnostics().is_empty());

    BrowserPlan::try_from(solved.timeline().expect("the fixture solves"))
        .expect("the fixture timeline fits the browser frame domain")
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
