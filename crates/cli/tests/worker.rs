use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use onmark_core::compiler;
use onmark_core::model::{FrameRate, SourceId, Timebase};
use onmark_core::protocol::BundleManifest;
use onmark_core::render_graph::{PartitionPlan, RenderGraph};
use onmark_core::timeline::TimelineIr;
use onmark_render::{
    BrowserLimits, EncodeLimits, ExecutableUnit, Ffmpeg, FrameArtifact, FrameArtifactLimits,
    RenderExecutor, RenderProfile, RenderUnit, UnitRootLimits, WorkerCaptureRequest,
};
use sha2::{Digest as _, Sha256};
use tempfile::tempdir;
use tokio::process::Command;
use tokio::time::timeout;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const FRAME_COUNT: u64 = 60;
const PROCESS_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::test]
#[ignore = "requires ONMARK_CHROME and a built @onmark/runtime package"]
async fn captures_a_portable_request_in_a_separate_worker_process() {
    let directory = tempdir().expect("the worker fixture directory is available");
    let input = directory.path().join("input");
    let artifact = directory.path().join("worker.onmark-frames");
    stage_input(&input, &static_request());

    capture_with_worker(&input, &artifact).await;
    capture_with_worker(&input, &artifact).await;

    let artifact = FrameArtifact::open(artifact, artifact_limits(FRAME_COUNT / 2))
        .await
        .expect("the separate worker publishes or reuses one valid frame artifact");
    assert_eq!(artifact.frames(), FRAME_COUNT / 2);
    artifact
        .verify()
        .await
        .expect("the separate worker artifact payload verifies");
}

#[tokio::test]
#[ignore = "requires ONMARK_CHROME, ONMARK_FFMPEG, and a built @onmark/runtime package"]
async fn assembles_two_independent_worker_processes_equivalently_to_one_local_film() {
    let directory = tempdir().expect("the worker fixture directory is available");
    let timeline = two_shot_timeline();
    let partitions = RenderGraph::from_timeline(&timeline).into_partition();
    assert_eq!(
        partitions.units().len(),
        2,
        "the worker fixture must form two independent partitions",
    );
    let profile = render_profile();
    let manifest = bundle_manifest();
    let units = partition_units(&timeline, &partitions, &manifest, profile);
    let requests: Vec<_> = units
        .iter()
        .map(RenderUnit::worker_capture_request)
        .collect();
    let assembly_units = materialize_units(units);
    let artifacts = capture_partition_artifacts(directory.path(), &requests).await;
    let baseline = materialize_whole_film(&timeline, &manifest, profile);
    let whole_output = directory.path().join("whole.mp4");
    let assembled_output = directory.path().join("assembled.mp4");
    let executor = executor();

    let whole = executor
        .render(baseline, &whole_output)
        .await
        .expect("the one-process whole-film baseline renders");
    let assembled = executor
        .assemble_frame_artifacts(&partitions, &assembly_units, &artifacts, &assembled_output)
        .await
        .expect("two worker artifacts assemble through the shared encoder");

    assert_eq!(whole.frames(), FRAME_COUNT);
    assert_eq!(assembled.frames(), FRAME_COUNT);
    assert_eq!(
        decoded_video_hash(&whole_output).await,
        decoded_video_hash(&assembled_output).await,
    );
}

#[tokio::test]
#[ignore = "requires ONMARK_CHROME and a built @onmark/runtime package"]
async fn partition_workers_match_the_whole_film_raw_rgba_sequence() {
    let directory = tempdir().expect("the worker fixture directory is available");
    let timeline = two_shot_timeline();
    let partition_plan = RenderGraph::from_timeline(&timeline).into_partition();
    let profile = render_profile();
    let manifest = bundle_manifest();
    let units = partition_units(&timeline, &partition_plan, &manifest, profile);
    let worker_requests: Vec<_> = units
        .iter()
        .map(RenderUnit::worker_capture_request)
        .collect();
    let whole_unit = whole_film_unit(&timeline, &manifest, profile);
    let whole_request = whole_unit.worker_capture_request();
    let whole_artifact = capture_worker_artifact(directory.path(), "whole", &whole_request).await;
    let partition_artifacts = capture_partition_artifacts(directory.path(), &worker_requests).await;

    FrameArtifact::verify_raw_rgba_equivalence(
        std::slice::from_ref(&whole_artifact),
        &partition_artifacts,
    )
    .await
    .expect("partition workers must capture the whole-film raw RGBA sequence");
}

async fn capture_partition_artifacts(
    workspace: &Path,
    requests: &[WorkerCaptureRequest],
) -> Vec<FrameArtifact> {
    let mut artifacts = Vec::with_capacity(requests.len());

    for (index, request) in requests.iter().enumerate() {
        let name = format!("partition-{index}");
        let artifact = capture_worker_artifact(workspace, &name, request).await;
        artifacts.push(artifact);
    }

    artifacts
}

async fn capture_worker_artifact(
    workspace: &Path,
    name: &str,
    request: &WorkerCaptureRequest,
) -> FrameArtifact {
    let input = workspace.join(format!("{name}-input"));
    let output = workspace.join(format!("{name}.onmark-frames"));
    stage_input(&input, request);
    capture_with_worker(&input, &output).await;
    let artifact = FrameArtifact::open(output, artifact_limits(request_frame_count(request)))
        .await
        .expect("the worker publishes a verified artifact");
    artifact
        .verify()
        .await
        .expect("the worker artifact checksum is valid before conformance");
    artifact
}

fn request_frame_count(request: &WorkerCaptureRequest) -> u64 {
    let output = request.browser_plan().output();
    output
        .end()
        .get()
        .checked_sub(output.start().get())
        .expect("the portable worker request has an ordered output interval")
}

async fn capture_with_worker(input: &Path, output: &Path) {
    let child = Command::new(env!("CARGO_BIN_EXE_onmark"))
        .args([
            "worker",
            "capture",
            "--input",
            input.to_str().expect("the fixture path is UTF-8"),
            "--output",
            output.to_str().expect("the fixture path is UTF-8"),
            "--browser",
            chrome().to_str().expect("the browser path is UTF-8"),
        ])
        .output();
    let output = timeout(PROCESS_TIMEOUT, child)
        .await
        .expect("the worker process must finish before its deadline")
        .expect("the worker command starts");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr),
    );
}

fn partition_units(
    timeline: &TimelineIr,
    partitions: &PartitionPlan,
    manifest: &BundleManifest,
    profile: RenderProfile,
) -> Vec<RenderUnit> {
    partitions
        .units()
        .iter()
        .map(|partition| {
            RenderUnit::from_partition(timeline, partition, manifest.clone(), profile, [])
                .expect("each static partition forms a render unit")
        })
        .collect()
}

fn materialize_units(units: Vec<RenderUnit>) -> Vec<ExecutableUnit> {
    let bundle = bundle();

    units
        .into_iter()
        .map(|unit| {
            ExecutableUnit::materialize(unit, &bundle, unit_root_limits())
                .expect("the fixture partition materializes")
        })
        .collect()
}

fn materialize_whole_film(
    timeline: &TimelineIr,
    manifest: &BundleManifest,
    profile: RenderProfile,
) -> ExecutableUnit {
    let unit = whole_film_unit(timeline, manifest, profile);
    let bundle = bundle();
    ExecutableUnit::materialize(unit, &bundle, unit_root_limits())
        .expect("the fixture whole film materializes")
}

fn whole_film_unit(
    timeline: &TimelineIr,
    manifest: &BundleManifest,
    profile: RenderProfile,
) -> RenderUnit {
    RenderUnit::whole_film(timeline, manifest.clone(), profile, [])
        .expect("the fixture whole film forms one render unit")
}

fn stage_input(input: &Path, request: &WorkerCaptureRequest) {
    fs::create_dir(input).expect("the worker input root is writable");
    let bundle_destination = input.join("bundle");
    fs::create_dir(&bundle_destination).expect("the worker bundle directory is writable");
    let source = bundle();
    for file in request.bundle().files() {
        let destination = bundle_destination.join(file.path());
        let parent = destination
            .parent()
            .expect("a checked-in bundle path always has a parent");
        fs::create_dir_all(parent).expect("the worker bundle path is writable");
        fs::copy(source.join(file.path()), destination)
            .expect("the worker input retains every bundle payload");
    }
    fs::write(
        input.join("request.json"),
        serde_json::to_vec(request).expect("the worker request serializes"),
    )
    .expect("the worker request is writable");
}

fn static_request() -> WorkerCaptureRequest {
    let timeline = one_shot_timeline();
    let manifest = bundle_manifest();
    RenderUnit::whole_film(&timeline, manifest, render_profile(), [])
        .expect("the static fixture forms one render unit")
        .worker_capture_request()
}

fn one_shot_timeline() -> TimelineIr {
    solve_timeline(
        r#"<film><scene><shot duration="1s"><title>Gate three</title></shot></scene></film>"#,
    )
}

fn two_shot_timeline() -> TimelineIr {
    solve_timeline(
        r#"<film><scene><shot duration="1s"><title>Opening</title></shot><shot duration="1s"><title>Closing</title></shot></scene></film>"#,
    )
}

fn solve_timeline(source: &str) -> TimelineIr {
    let (document, diagnostics) = compiler::parse(SourceId::new(0), source).into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::bind(document).into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::resolve(film.expect("the fixture binds")).into_parts();
    assert!(diagnostics.is_empty());
    let report = compiler::solve(
        film.expect("the fixture resolves"),
        &BTreeMap::new(),
        Timebase::new(FrameRate::new(30, 1).expect("the fixture frame rate is valid")),
    )
    .expect("the static fixture has no integration failures");
    let (timeline, diagnostics) = report.into_parts();
    assert!(diagnostics.is_empty());
    timeline.expect("the fixture solves")
}

async fn decoded_video_hash(path: &Path) -> [u8; 32] {
    let child = Command::new(ffmpeg())
        .args([
            "-nostdin",
            "-v",
            "error",
            "-i",
            path.to_str().expect("the fixture path is UTF-8"),
            "-map",
            "0:v:0",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
        .output();
    let output = timeout(PROCESS_TIMEOUT, child)
        .await
        .expect("FFmpeg must decode the fixture before its deadline")
        .expect("FFmpeg must start to decode the fixture");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr),
    );
    Sha256::digest(&output.stdout).into()
}

fn artifact_limits(max_frames: u64) -> FrameArtifactLimits {
    FrameArtifactLimits::new(max_frames, 64 * 1024 * 1024, 8 * 1024 * 1024)
        .expect("the fixture artifact limits are bounded")
}

fn render_profile() -> RenderProfile {
    RenderProfile::new(WIDTH, HEIGHT).expect("the fixture profile is valid")
}

fn unit_root_limits() -> UnitRootLimits {
    UnitRootLimits::new(4, 64 * 1024 * 1024).expect("the fixture unit limits are bounded")
}

fn executor() -> RenderExecutor {
    let browser_limits = BrowserLimits::new(PROCESS_TIMEOUT, 8 * 1024 * 1024)
        .expect("the fixture browser limits are bounded");
    let encode_limits =
        EncodeLimits::new(PROCESS_TIMEOUT, FRAME_COUNT, 64 * 1024 * 1024, 64 * 1024)
            .expect("the fixture encoder limits are bounded");
    let ffmpeg = Ffmpeg::new(ffmpeg(), encode_limits).expect("the fixture FFmpeg path is valid");

    RenderExecutor::new(chrome(), browser_limits, ffmpeg)
}

fn bundle_manifest() -> BundleManifest {
    serde_json::from_str(
        &fs::read_to_string(bundle().join("manifest.json"))
            .expect("the checked-in bundle manifest is readable"),
    )
    .expect("the checked-in bundle manifest is valid")
}

fn bundle() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("conformance/protocol/bundle-v1")
}

fn chrome() -> PathBuf {
    required_path("ONMARK_CHROME")
}

fn ffmpeg() -> PathBuf {
    required_path("ONMARK_FFMPEG")
}

fn required_path(variable: &str) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{variable} must name an executable"))
}
