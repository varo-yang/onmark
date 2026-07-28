//! Opt-in real-process conformance for capture, partitioning, and assembly.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use onmark_core::compiler;
use onmark_core::model::{
    AssetRef, FrameRate, FrozenAsset, FrozenAssetId, PresentationTemporalCapability,
    PresentationVisualCapability, SourceId, Timebase,
};
use onmark_core::protocol::{
    BrowserCommand, BrowserEvent, BrowserMediaMode, BrowserOverlayKind, BrowserPlan,
    BrowserRequest, BundleManifest, RequestId, WireFrame,
};
use onmark_core::render_graph::{PartitionPlan, RenderGraph};
use onmark_media::{Ffprobe, SubtitleLimits, parse_webvtt};
use onmark_render::{
    AlphaMode, BrowserCaptureMode, BrowserErrorKind, BrowserGraphicsBackend, BrowserLaunchPolicy,
    BrowserLimits, BrowserSession, BrowserSessionOptions, CaptureEnvironmentId, EncodeLimits,
    EncodeProfile, EncodedPng, ExecutableUnit, Ffmpeg, FrameArtifact, FrameArtifactErrorKind,
    FrameArtifactLimits, MaterializedAsset, RawRgbaHash, RenderErrorKind, RenderExecutor,
    RenderProfile, RenderUnit, UnitRootLimits,
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
const ONE_SHOT_PROJECTION: &[&[u32]] = &[&[0]];
const TWO_SHOT_PROJECTION: &[&[u32]] = &[&[0], &[1]];
const TRANSITION_PROJECTION: &[&[u32]] = &[&[0], &[0, 1], &[1]];
const UNIT_ROOT_FILE_LIMIT: usize = 16;
const TEMPORAL_SEEK_SEQUENCE: [u64; 4] = [17, 3, 29, 17];
const MICROS_PER_SECOND: i64 = 1_000_000;
const OUTPUT_AUDIO_SAMPLE_RATE: u64 = 48_000;
const AUDIO_TIMESTAMP_TOLERANCE_MICROS: u64 = 25_000;

#[test]
fn render_executor_uses_the_environment_owned_capture_mode() {
    let limits = EncodeLimits::new(Duration::from_secs(1), 2, 2, 2)
        .expect("the fixture encoding limits are bounded");
    let ffmpeg = Ffmpeg::new("ffmpeg", limits, onmark_render::EncodeProfile::H264Mp4)
        .expect("the fixture executable path is present");
    let executor = RenderExecutor::new(
        "misleading/chrome-headless-shell",
        BrowserCaptureMode::Screenshot,
        browser_limits(Duration::from_secs(1)),
        ffmpeg,
    );

    assert_eq!(executor.capture_mode(), BrowserCaptureMode::Screenshot);
}

#[tokio::test]
async fn rejects_units_that_do_not_match_the_partition_plan_before_launching_browser() {
    let timeline = solve_timeline(
        concat!(
            "<om-film><om-scene>",
            r#"<om-shot duration="1s"></om-shot>"#,
            r#"<om-shot duration="1s"></om-shot>"#,
            "</om-scene></om-film>",
        ),
        &BTreeMap::new(),
    );
    let partitions =
        RenderGraph::from_timeline(&timeline, PresentationTemporalCapability::RandomAccess)
            .expect("the solved fixture has complete render ownership")
            .into_partition();
    let directory = tempdir().expect("the test output directory must be available");
    let output = directory.path().join("partitioned.mp4");
    let limits = EncodeLimits::new(Duration::from_secs(1), 2, 2, 2)
        .expect("the fixture encoding limits are bounded");
    let ffmpeg = Ffmpeg::new("ffmpeg", limits, onmark_render::EncodeProfile::H264Mp4)
        .expect("the fixture executable path is present");
    let executor = RenderExecutor::new(
        "browser",
        BrowserCaptureMode::Screenshot,
        browser_limits(Duration::from_secs(1)),
        ffmpeg,
    );

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
    let mut session = BrowserSession::launch(
        headless_shell(),
        browser_options(BrowserCaptureMode::BeginFrame, Duration::from_secs(5)),
    )
    .await
    .expect("headless shell must launch");
    let fixture = render_fixture("missing-runtime.html");

    let error = session
        .navigate(&fixture, &fixture_root(&fixture))
        .await
        .expect_err("the missing host must miss its readiness deadline");
    let shutdown = session.shutdown().await;

    assert_eq!(error.kind(), BrowserErrorKind::RuntimeHost);
    shutdown.expect("headless shell must shut down after a readiness failure");
}

#[tokio::test]
#[ignore = "requires ONMARK_HEADLESS_SHELL"]
async fn blocks_navigation_outside_the_private_resource_root() {
    let mut session = BrowserSession::launch(
        headless_shell(),
        browser_options(BrowserCaptureMode::BeginFrame, Duration::from_secs(5)),
    )
    .await
    .expect("headless shell must launch");
    let fixture = render_fixture("missing-runtime.html");
    let unrelated_root = tempdir().expect("the unrelated private root must be available");

    let error = session
        .navigate(&fixture, unrelated_root.path())
        .await
        .expect_err("Chromium must not read a file outside the declared private root");
    let shutdown = session.shutdown().await;

    assert_eq!(error.kind(), BrowserErrorKind::Navigation);
    shutdown.expect("headless shell must shut down after a blocked navigation");
}

#[tokio::test]
#[ignore = "requires ONMARK_HEADLESS_SHELL and a built @onmark/runtime package"]
async fn bounds_a_runtime_adapter_that_never_finishes_loading() {
    let mut session = BrowserSession::launch(
        headless_shell(),
        browser_options(BrowserCaptureMode::BeginFrame, Duration::from_secs(5)),
    )
    .await
    .expect("headless shell must launch");
    let fixture = render_fixture("stalled-runtime.html");
    session
        .navigate(&fixture, &fixture_root(&fixture))
        .await
        .expect("the stalled fixture must install its runtime host");

    let request = BrowserRequest::new(
        RequestId::new(1),
        BrowserCommand::load(browser_plan_fixture(), BrowserMediaMode::Decoded),
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
#[ignore = "requires ONMARK_PORTABLE_CHROME and a built @onmark/runtime package"]
async fn captures_stable_frames_through_the_portable_screenshot_backend() {
    let fixture = browser_fixture();
    let browser = required_path("ONMARK_PORTABLE_CHROME");
    let first = capture_portable_fingerprint(&browser, &fixture).await;
    let second = capture_portable_fingerprint(&browser, &fixture).await;

    assert_eq!(
        first, second,
        "locked portable browser sessions must capture equal RGBA",
    );
}

#[tokio::test]
#[ignore = "requires ONMARK_BUNDLER and ONMARK_HEADLESS_SHELL"]
async fn seeks_browser_animation_playheads_deterministically() {
    let directory = tempdir().expect("the experiment workspace must be available");
    let bundle = FixtureBundle::build_temporal(directory.path()).await;
    let entry = bundle.entry_url();
    let first = capture_temporal_sequence(&entry).await;
    let second = capture_temporal_sequence(&entry).await;

    assert_eq!(first, second, "independent browser processes must agree");
    assert_eq!(first[0], first[3], "repeated exact frames must agree");
    assert!(
        first.windows(2).any(|frames| frames[0] != frames[1]),
        "the experiment must contain visible temporal change",
    );
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "requires ONMARK_BUNDLER and ONMARK_PORTABLE_CHROME"]
async fn seeks_dynamic_frames_deterministically_on_metal() {
    let directory = tempdir().expect("the experiment workspace must be available");
    let bundle = FixtureBundle::build_temporal(directory.path()).await;
    let browser = required_path("ONMARK_PORTABLE_CHROME");
    let entry = bundle.entry_url();
    let software =
        capture_portable_temporal_sequence(&browser, BrowserGraphicsBackend::SwiftShader, &entry)
            .await;
    let first =
        capture_portable_temporal_sequence(&browser, BrowserGraphicsBackend::Metal, &entry).await;
    let second =
        capture_portable_temporal_sequence(&browser, BrowserGraphicsBackend::Metal, &entry).await;

    assert_eq!(first, second, "independent browser processes must agree");
    assert_ne!(
        first, software,
        "the fixture must expose backend-sensitive WebGL pixels",
    );
    assert_eq!(first[0], first[3], "repeated exact frames must agree");
    assert!(
        first.windows(2).any(|frames| frames[0] != frames[1]),
        "the experiment must contain visible temporal change",
    );
}

#[tokio::test]
#[ignore = "requires ONMARK_HEADLESS_SHELL, ONMARK_FFMPEG, and ONMARK_FFPROBE"]
async fn renders_the_browser_plan_to_a_verified_mp4() {
    let directory = tempdir().expect("the test output directory must be available");
    let source = directory.path().join("source.mp4");
    let output = directory.path().join("render.mp4");
    generate_source_video(&source, "2.5").await;
    let frozen = freeze_asset(&source).await;
    let executor = real_executor(100);
    let bundle = FixtureBundle::checked_in();
    let unit = executable_video_unit(&bundle, frozen, source);

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
#[ignore = "requires ONMARK_PORTABLE_CHROME, ONMARK_FFMPEG, and ONMARK_FFPROBE"]
async fn renders_decoded_video_through_the_portable_screenshot_backend() {
    let directory = tempdir().expect("the test output directory must be available");
    let source = directory.path().join("source.mp4");
    let output = directory.path().join("portable-video.mp4");
    generate_source_video(&source, "2.5").await;
    let frozen = freeze_asset(&source).await;
    let bundle = FixtureBundle::checked_in();
    let unit = executable_video_unit(&bundle, frozen, source);

    let executor = portable_executor(FRAME_COUNT);
    let video = executor
        .render(unit, &output)
        .await
        .expect("portable Chrome must render decoded video");

    assert_eq!(video.frames(), FRAME_COUNT);
    assert_video_stream(&output, FRAME_COUNT).await;
    assert_decodable_motion(&output).await;
}

#[tokio::test]
#[ignore = "requires ONMARK_BUNDLER, ONMARK_HEADLESS_SHELL or ONMARK_PORTABLE_CHROME, ONMARK_FFMPEG, and ONMARK_FFPROBE"]
async fn renders_and_repeats_the_production_layered_path() {
    let directory = tempdir().expect("the test output directory must be available");
    let bundle = FixtureBundle::build_layered(directory.path()).await;
    let per_frame_bundle = FixtureBundle::build_layered_per_frame(directory.path()).await;
    let source = directory.path().join("source.mp4");
    let output = directory.path().join("layered.mp4");
    let per_frame_path = directory.path().join("per-frame.onmark-frames");
    let first_path = directory.path().join("first.onmark-frames");
    let second_path = directory.path().join("second.onmark-frames");
    generate_source_video(&source, "2.5").await;
    let frozen = freeze_asset(&source).await;
    let executor = layered_executor(100);

    let rendered = executor
        .render(
            executable_video_unit(&bundle, frozen.clone(), source.clone()),
            &output,
        )
        .await
        .expect("the admitted layered path must render one MP4");
    let per_frame = executor
        .capture_frame_artifact_report(
            &executable_video_unit(&per_frame_bundle, frozen.clone(), source.clone()),
            capture_environment(),
            &per_frame_path,
            frame_artifact_limits(),
        )
        .await
        .expect("the per-frame control capture must publish");
    let per_frame_metrics = per_frame
        .metrics()
        .expect("the control performs a fresh capture");
    assert_eq!(per_frame_metrics.browser_captures(), FRAME_COUNT);
    assert!(
        per_frame_metrics.browser_capture_commands() >= FRAME_COUNT,
        "each authored capture requires at least one Chromium command",
    );
    let per_frame = per_frame.into_artifact();

    let first = executor
        .capture_frame_artifact_report(
            &executable_video_unit(&bundle, frozen.clone(), source.clone()),
            capture_environment(),
            &first_path,
            frame_artifact_limits(),
        )
        .await
        .expect("the first layered worker capture must publish");
    let placement_bounded_metrics = first
        .metrics()
        .expect("the admitted path performs a fresh capture");
    assert_eq!(placement_bounded_metrics.browser_captures(), 1);
    assert!(
        placement_bounded_metrics.browser_capture_commands()
            < per_frame_metrics.browser_capture_commands(),
        "placement-bounded capture must remove Chromium commands",
    );
    let first = first.into_artifact();
    let second = executor
        .capture_frame_artifact(
            &executable_video_unit(&bundle, frozen, source),
            capture_environment(),
            &second_path,
            frame_artifact_limits(),
        )
        .await
        .expect("the repeated layered worker capture must publish");

    FrameArtifact::verify_raw_rgba_equivalence(
        std::slice::from_ref(&per_frame),
        std::slice::from_ref(&first),
    )
    .await
    .expect("placement-bounded reuse must preserve every canonical output pixel");
    FrameArtifact::verify_raw_rgba_equivalence(
        std::slice::from_ref(&first),
        std::slice::from_ref(&second),
    )
    .await
    .expect("independent layered workers must produce equal canonical pixels");
    assert_eq!(rendered.frames(), FRAME_COUNT);
    assert_video_stream(&output, FRAME_COUNT).await;
    assert_decodable_motion(&output).await;
}

#[tokio::test]
#[ignore = "requires ONMARK_BUNDLER, ONMARK_FFMPEG, ONMARK_FFPROBE, and a supported browser"]
async fn preserves_backdrop_layout_across_whole_local_and_worker_execution() {
    let directory = tempdir().expect("the experiment workspace must be available");
    let source = repository().join("conformance/browser/backdrop-sequence.html");
    let bundle = FixtureBundle::build_from(
        directory.path(),
        "backdrop-bundle",
        &source,
        "randomAccess",
        "separableBackdrop",
        "perFrame",
        TWO_SHOT_PROJECTION,
    )
    .await;
    let fixture = BackdropFixture::materialize(directory.path(), &source, &bundle).await;
    let executor = layered_executor(TWO_UNIT_FRAME_COUNT);
    let whole_path = directory.path().join("whole.onmark-frames");
    let local_output = directory.path().join("local.mp4");
    let assembled_output = directory.path().join("assembled.mp4");

    let whole = capture_and_reuse_static(&executor, &fixture.whole_film, &whole_path).await;
    let partitions = capture_partition_artifacts(
        &executor,
        directory.path(),
        "backdrop-partition",
        &fixture.partitioned_units,
    )
    .await;
    FrameArtifact::verify_raw_rgba_equivalence(std::slice::from_ref(&whole), &partitions)
        .await
        .expect("whole and distributed backdrop capture must produce equal raw pixels");

    let local = executor
        .render_partitioned(
            &fixture.partition_plan,
            fixture.partitioned_units,
            &local_output,
        )
        .await
        .expect("local backdrop partitions must render");
    let assembled = executor
        .assemble_frame_artifacts(
            &fixture.partition_plan,
            &fixture.assembly_units,
            &partitions,
            capture_environment(),
            &assembled_output,
        )
        .await
        .expect("worker backdrop artifacts must assemble");

    assert_eq!(local.frames(), TWO_UNIT_FRAME_COUNT);
    assert_eq!(assembled.frames(), TWO_UNIT_FRAME_COUNT);
    assert_eq!(
        decoded_hashes(&local_output, "0:v:0").await,
        decoded_hashes(&assembled_output, "0:v:0").await,
        "local and distributed backdrop output must decode equally",
    );
}

#[tokio::test]
#[ignore = "requires ONMARK_BUNDLER, ONMARK_FFMPEG, ONMARK_FFPROBE, and a supported browser"]
async fn renders_multiple_backdrop_videos_in_one_shot() {
    let directory = tempdir().expect("the experiment workspace must be available");
    let source = repository().join("conformance/browser/backdrop-split.html");
    let bundle = FixtureBundle::build_from(
        directory.path(),
        "backdrop-split-bundle",
        &source,
        "randomAccess",
        "separableBackdrop",
        "perFrame",
        ONE_SHOT_PROJECTION,
    )
    .await;
    let video_path = directory.path().join("source.mp4");
    generate_source_video(&video_path, "1").await;
    let video = freeze_asset(&video_path).await;
    let assets = BTreeMap::from([(asset_ref("source.mp4"), video.clone())]);
    let screenplay = fs::read_to_string(source).expect("the backdrop screenplay is readable");
    let timeline = solve_timeline(&screenplay, &assets);
    let materialized =
        MaterializedAsset::new(video, video_path).expect("the backdrop source path is present");
    let unit = RenderUnit::whole_film(
        &timeline,
        bundle.manifest.clone(),
        render_profile(),
        [materialized],
    )
    .expect("the split-screen backdrop forms one render unit");
    assert_eq!(
        unit.visual_execution()
            .backdrop_media()
            .expect("split-screen video must use native backdrop media")
            .media()
            .len(),
        2,
    );
    let output = unit.browser_plan().output();
    let expected_frames = output.end().get() - output.start().get();
    let unit = bundle.materialize(unit);
    let executor = layered_executor(expected_frames);
    let first_path = directory.path().join("split-first.onmark-frames");
    let second_path = directory.path().join("split-second.onmark-frames");

    let first = executor
        .capture_frame_artifact(
            &unit,
            capture_environment(),
            &first_path,
            frame_artifact_limits(),
        )
        .await
        .expect("the first split-screen worker capture must publish");
    let second = executor
        .capture_frame_artifact(
            &unit,
            capture_environment(),
            &second_path,
            frame_artifact_limits(),
        )
        .await
        .expect("the repeated split-screen worker capture must publish");

    FrameArtifact::verify_raw_rgba_equivalence(
        std::slice::from_ref(&first),
        std::slice::from_ref(&second),
    )
    .await
    .expect("independent split-screen workers must produce equal raw pixels");
    assert_eq!(first.frames(), expected_frames);
}

#[tokio::test]
#[ignore = "requires ONMARK_BUNDLER, ONMARK_FFMPEG, ONMARK_FFPROBE, and a supported browser"]
async fn renders_repeated_and_held_media_equally_as_one_or_two_units() {
    let directory = tempdir().expect("the test output directory must be available");
    let bundle = FixtureBundle::build_media_continuity(directory.path()).await;
    let fixture = AudioSubtitleFixture::materialize(
        directory.path(),
        &bundle,
        "conformance/cli/media-continuity.html",
    )
    .await;
    let whole_output = directory.path().join("whole.mp4");
    let partitioned_output = directory.path().join("partitioned.mp4");
    let executor = layered_executor(TWO_UNIT_FRAME_COUNT);

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
    let whole = inspect_audio_subtitle_output(&whole_output).await;
    let partitioned = inspect_audio_subtitle_output(&partitioned_output).await;
    assert_eq!(
        whole.video_hashes, partitioned.video_hashes,
        "source repetition and final-frame hold must preserve whole-film browser pixels",
    );
    assert_eq!(
        whole.audio_hashes, partitioned.audio_hashes,
        "partitioning must not change the decoded final audio",
    );
}

#[tokio::test]
#[ignore = "requires ONMARK_BUNDLER, ONMARK_FFMPEG, ONMARK_FFPROBE, and a supported browser"]
async fn preserves_source_edits_across_local_and_worker_partition_execution() {
    let directory = tempdir().expect("the test output directory must be available");
    let bundle = FixtureBundle::build_source_edits(directory.path()).await;
    let local = AudioSubtitleFixture::materialize(
        directory.path(),
        &bundle,
        "conformance/cli/audio-subtitle.html",
    )
    .await;
    let distributed = AudioSubtitleFixture::materialize(
        directory.path(),
        &bundle,
        "conformance/cli/audio-subtitle.html",
    )
    .await;
    let local_output = directory.path().join("local-partitions.mp4");
    let assembled_output = directory.path().join("assembled-from-artifacts.mp4");
    let executor = layered_executor(TWO_UNIT_FRAME_COUNT);

    let local_artifacts = capture_partition_artifacts(
        &executor,
        directory.path(),
        "local",
        &local.partitioned_units,
    )
    .await;
    let worker_artifacts = capture_partition_artifacts(
        &executor,
        directory.path(),
        "worker",
        &distributed.partitioned_units,
    )
    .await;

    let assembled = executor
        .assemble_frame_artifacts(
            &distributed.partition_plan,
            &distributed.partitioned_units,
            &worker_artifacts,
            capture_environment(),
            &assembled_output,
        )
        .await
        .expect("the assembler must reuse worker artifacts through one encoder");
    let rendered = executor
        .render_partitioned(
            &local.partition_plan,
            local.partitioned_units,
            &local_output,
        )
        .await
        .expect("the same source edits must render through local partitions");

    FrameArtifact::verify_raw_rgba_equivalence(&local_artifacts, &worker_artifacts)
        .await
        .expect("local and worker source edits must produce the same raw pixels");
    assert_eq!(assembled.frames(), TWO_UNIT_FRAME_COUNT);
    assert_eq!(rendered.frames(), TWO_UNIT_FRAME_COUNT);
    let assembled = inspect_audio_subtitle_output(&assembled_output).await;
    let rendered = inspect_audio_subtitle_output(&local_output).await;
    assert_eq!(assembled.video_hashes, rendered.video_hashes);
    assert_eq!(assembled.audio_hashes, rendered.audio_hashes);
}

#[tokio::test]
#[ignore = "requires ONMARK_BUNDLER, ONMARK_FFMPEG, ONMARK_FFPROBE, and a supported browser"]
async fn preserves_alpha_across_whole_partitioned_and_worker_output() {
    let directory = tempdir().expect("the experiment workspace must be available");
    let source = directory.path().join("transparent.html");
    fs::write(&source, transparent_source())
        .expect("the transparent presentation source must be writable");
    let bundle = FixtureBundle::build_from(
        directory.path(),
        "transparent-bundle",
        &source,
        "randomAccess",
        "browserComposite",
        "perFrame",
        TWO_SHOT_PROJECTION,
    )
    .await;
    let local =
        StaticPartitionFixture::materialize_with_profile(&source, &bundle, transparent_profile());
    let distributed =
        StaticPartitionFixture::materialize_with_profile(&source, &bundle, transparent_profile());
    let executor = executor_with_profile(TWO_UNIT_FRAME_COUNT, EncodeProfile::ProRes4444Mov);
    let whole_output = directory.path().join("whole.mov");
    let assembled_output = directory.path().join("assembled.mov");
    let whole = executor
        .render(local.whole_film, &whole_output)
        .await
        .expect("the complete transparent film must render");
    let captured =
        capture_static_fixture(&executor, directory.path(), "transparent", &distributed).await;

    FrameArtifact::verify_raw_rgba_equivalence(
        std::slice::from_ref(&captured.whole),
        &captured.partitions,
    )
    .await
    .expect("transparent worker artifacts must reproduce whole-film pixels");
    let assembled = executor
        .assemble_frame_artifacts(
            &distributed.partition_plan,
            &distributed.partitioned_units,
            &captured.partitions,
            capture_environment(),
            &assembled_output,
        )
        .await
        .expect("transparent worker artifacts must assemble through the same profile");

    assert_eq!(whole.frames(), TWO_UNIT_FRAME_COUNT);
    assert_eq!(assembled.frames(), TWO_UNIT_FRAME_COUNT);
    assert_prores_4444_alpha(&whole_output).await;
    assert_prores_4444_alpha(&assembled_output).await;
    assert_eq!(
        decoded_hashes(&whole_output, "0:v:0").await,
        decoded_hashes(&assembled_output, "0:v:0").await,
        "local and worker assembly must decode to the same ProRes 4444 frames",
    );
}

#[tokio::test]
#[ignore = "requires ONMARK_BUNDLER, ONMARK_HEADLESS_SHELL, and ONMARK_FFMPEG"]
async fn isolates_one_authored_html_edit_to_its_render_partition() {
    let directory = tempdir().expect("the experiment workspace must be available");
    let baseline_source = directory.path().join("baseline.html");
    let edited_source = directory.path().join("edited.html");
    fs::write(&baseline_source, isolation_source("Before edit"))
        .expect("the baseline presentation source must be writable");
    fs::write(&edited_source, isolation_source("After edit"))
        .expect("the edited presentation source must be writable");

    let baseline_bundle = FixtureBundle::build_from(
        directory.path(),
        "baseline-bundle",
        &baseline_source,
        "randomAccess",
        "browserComposite",
        "placementBounded",
        TWO_SHOT_PROJECTION,
    )
    .await;
    let edited_bundle = FixtureBundle::build_from(
        directory.path(),
        "edited-bundle",
        &edited_source,
        "randomAccess",
        "browserComposite",
        "placementBounded",
        TWO_SHOT_PROJECTION,
    )
    .await;
    let baseline = StaticPartitionFixture::materialize(&baseline_source, &baseline_bundle);
    let edited = StaticPartitionFixture::materialize(&edited_source, &edited_bundle);
    let executor = real_executor(TWO_UNIT_FRAME_COUNT);

    let baseline = capture_static_fixture(&executor, directory.path(), "baseline", &baseline).await;
    let edited = capture_static_fixture(&executor, directory.path(), "edited", &edited).await;

    FrameArtifact::verify_raw_rgba_equivalence(
        std::slice::from_ref(&baseline.whole),
        &baseline.partitions,
    )
    .await
    .expect("the baseline partitions must reproduce their whole film");
    FrameArtifact::verify_raw_rgba_equivalence(
        std::slice::from_ref(&edited.whole),
        &edited.partitions,
    )
    .await
    .expect("the edited partitions must reproduce their whole film");
    FrameArtifact::verify_raw_rgba_equivalence(
        std::slice::from_ref(&baseline.partitions[0]),
        std::slice::from_ref(&edited.partitions[0]),
    )
    .await
    .expect("editing the closing shot must preserve opening pixels");

    let error = FrameArtifact::verify_raw_rgba_equivalence(
        std::slice::from_ref(&baseline.partitions[1]),
        std::slice::from_ref(&edited.partitions[1]),
    )
    .await
    .expect_err("the edited closing title must change closing pixels");
    assert_eq!(error.kind(), FrameArtifactErrorKind::RawRgbaMismatch);
}

#[tokio::test]
#[ignore = "requires ONMARK_BUNDLER, ONMARK_FFMPEG, and a supported browser"]
async fn retains_exact_motion_across_partition_evaluations() {
    let directory = tempdir().expect("the experiment workspace must be available");
    let source = directory.path().join("continuous-motion.html");
    fs::write(&source, exact_motion_partition_source())
        .expect("the continuous-motion source must be writable");
    let bundle = FixtureBundle::build_from(
        directory.path(),
        "continuous-motion-bundle",
        &source,
        "randomAccess",
        "browserComposite",
        "perFrame",
        TWO_SHOT_PROJECTION,
    )
    .await;
    let fixture = StaticPartitionFixture::materialize(&source, &bundle);
    let executor = layered_executor(TWO_UNIT_FRAME_COUNT);

    let captured =
        capture_static_fixture(&executor, directory.path(), "continuous-motion", &fixture).await;

    FrameArtifact::verify_raw_rgba_equivalence(
        std::slice::from_ref(&captured.whole),
        &captured.partitions,
    )
    .await
    .expect("partition evaluations must retain exact local motion");
}

#[tokio::test]
#[ignore = "requires ONMARK_BUNDLER, ONMARK_FFMPEG, and a supported browser"]
async fn transition_regions_match_the_whole_film_pixel_sequence() {
    let directory = tempdir().expect("the experiment workspace must be available");
    let source = directory.path().join("transition.html");
    fs::write(&source, transition_partition_source())
        .expect("the transition source must be writable");
    let bundle = FixtureBundle::build_from(
        directory.path(),
        "transition-bundle",
        &source,
        "randomAccess",
        "browserComposite",
        "perFrame",
        TRANSITION_PROJECTION,
    )
    .await;
    let fixture = StaticPartitionFixture::materialize_with_region_count(
        &source,
        &bundle,
        render_profile(),
        3,
    );
    let executor = layered_executor(45);

    let captured =
        capture_static_fixture(&executor, directory.path(), "transition", &fixture).await;

    FrameArtifact::verify_raw_rgba_equivalence(
        std::slice::from_ref(&captured.whole),
        &captured.partitions,
    )
    .await
    .expect("transition regions must reproduce their whole-film pixels");
}

#[tokio::test]
#[ignore = "requires ONMARK_BUNDLER, ONMARK_HEADLESS_SHELL, and ONMARK_FFMPEG"]
async fn shot_projection_blocks_cross_partition_css_observation() {
    let directory = tempdir().expect("the experiment workspace must be available");
    let baseline_source = directory.path().join("dependency-baseline.html");
    let edited_source = directory.path().join("dependency-edited.html");
    fs::write(&baseline_source, dependency_source("ordinary", "Closing"))
        .expect("the dependency baseline must be writable");
    fs::write(&edited_source, dependency_source("trigger", "Closing"))
        .expect("the dependency edit must be writable");

    let baseline_bundle = FixtureBundle::build_from(
        directory.path(),
        "dependency-baseline-bundle",
        &baseline_source,
        "randomAccess",
        "browserComposite",
        "placementBounded",
        TWO_SHOT_PROJECTION,
    )
    .await;
    let edited_bundle = FixtureBundle::build_from(
        directory.path(),
        "dependency-edited-bundle",
        &edited_source,
        "randomAccess",
        "browserComposite",
        "placementBounded",
        TWO_SHOT_PROJECTION,
    )
    .await;
    let baseline = StaticPartitionFixture::materialize(&baseline_source, &baseline_bundle);
    let edited = StaticPartitionFixture::materialize(&edited_source, &edited_bundle);
    let executor = real_executor(TWO_UNIT_FRAME_COUNT);

    let baseline = capture_static_partitions(
        &executor,
        directory.path(),
        "dependency-baseline",
        &baseline,
    )
    .await;
    let edited =
        capture_static_partitions(&executor, directory.path(), "dependency-edited", &edited).await;

    FrameArtifact::verify_raw_rgba_equivalence(
        std::slice::from_ref(&baseline[0]),
        std::slice::from_ref(&edited[0]),
    )
    .await
    .expect("an omitted closing shot cannot affect opening pixels through :has()");
}

async fn inspect_audio_subtitle_output(output: &Path) -> DecodedOutput {
    assert_video_stream(output, TWO_UNIT_FRAME_COUNT).await;
    let output = inspect_output(output).await;
    assert_eq!(
        output.audio_presentation_samples,
        TWO_UNIT_FRAME_COUNT * OUTPUT_AUDIO_SAMPLE_RATE / 30,
        "the AAC presentation timeline must end with the visual frame range",
    );
    assert!(
        output.has_motion(),
        "the audio-and-subtitle video must contain motion"
    );
    assert!(
        !output.audio_hashes.is_empty(),
        "the audio-and-subtitle video must retain its final audio mix",
    );
    assert_audio_starts_at(&output, 0);
    output
}

fn isolation_source(closing: &str) -> String {
    static_partition_source("", "", closing)
}

fn transparent_source() -> &'static str {
    r#"<!doctype html>
<html><head><style>
html, body, om-film, om-scene, om-shot {
  display: block; height: 100%; margin: 0; overflow: hidden; width: 100%;
}
om-shot { align-items: center; display: flex; justify-content: center; }
om-title {
  background: rgba(70, 120, 255, 0.65);
  border-radius: 48px;
  color: rgba(255, 255, 255, 0.9);
  display: block;
  font: 700 28px sans-serif;
  padding: 36px 52px;
}
</style></head><body>
<om-film><om-scene>
  <om-shot duration="1s"><om-title>Alpha one</om-title></om-shot>
  <om-shot duration="1s"><om-title>Alpha two</om-title></om-shot>
</om-scene></om-film>
</body></html>
"#
}

fn exact_motion_partition_source() -> &'static str {
    r#"<!doctype html>
<html><head><style>
html, body, om-film, om-scene, om-shot {
  display: block; height: 100%; margin: 0; overflow: hidden; width: 100%;
}
body { background: #070b15; }
om-scene {
  background: linear-gradient(110deg, #356dff, #ad4cff);
  width: calc(100% + 80px);
}
om-shot {
  align-items: center; display: flex; justify-content: center;
}
om-title { color: white; font: 700 28px sans-serif; }
</style></head><body>
<om-film><om-scene>
  <om-shot duration="1s"><om-title>Continuous</om-title></om-shot>
  <om-shot duration="1s"><om-title>Motion</om-title></om-shot>
</om-scene></om-film>
<script type="module" data-om-motion>
import {
  combineMotion,
  frameMotion,
  interpolate,
  spring,
} from "onmark/authoring";
import { gsapMotion } from "onmark/motion/gsap";

export const motion = combineMotion(
  gsapMotion({
    scene({ element, timeline }) {
      timeline.fromTo(
        element,
        { x: -80 },
        { duration: 2, ease: "none", force3D: false, x: 0 },
        0,
      );
    },
  }),
  frameMotion({
    scene(context) {
      const progress = spring(context, { damping: 20 });
      context.element.style.opacity = String(
        interpolate(progress, [0, 1], [0.5, 1]),
      );
    },
  }),
);
</script>
</body></html>
"#
}

fn transition_partition_source() -> &'static str {
    r#"<!doctype html>
<html><head><style>
html, body, om-film, om-scene, om-shot {
  display: block; height: 100%; margin: 0; overflow: hidden; width: 100%;
}
om-scene { display: block; position: relative; }
om-shot {
  align-items: center;
  display: flex;
  inset: 0;
  justify-content: center;
  position: absolute;
}
.outgoing { background: #ff593d; }
.incoming { background: #315cff; }
om-title { color: white; font: 700 28px sans-serif; }
</style></head><body>
<om-film><om-scene>
  <om-shot class="outgoing" duration="1s"><om-title>Outgoing</om-title></om-shot>
  <om-transition duration="500ms"></om-transition>
  <om-shot class="incoming" duration="1s"><om-title>Incoming</om-title></om-shot>
</om-scene></om-film>
<script type="module" data-om-motion>
import { frameMotion } from "onmark/authoring";

export const motion = frameMotion({
  transition({ incomingElement, outgoingElement, progress }) {
    outgoingElement.style.transform = `translateX(${-100 * progress}%)`;
    incomingElement.style.transform = `translateX(${100 * (1 - progress)}%)`;
  },
});
</script>
</body></html>
"#
}

fn dependency_source(closing_class: &str, closing: &str) -> String {
    static_partition_source(
        "om-scene:has(> om-shot.trigger) > om-shot:first-child om-title { color: #d02020; }\n",
        closing_class,
        closing,
    )
}

fn static_partition_source(extra_style: &str, closing_class: &str, closing: &str) -> String {
    format!(
        r#"<!doctype html>
<html><head><style>
html, body, om-film, om-scene, om-shot {{
  display: block; height: 100%; margin: 0; width: 100%;
}}
body {{ background: #f5f0e8; overflow: hidden; }}
om-shot {{
  align-items: center; display: flex; justify-content: center;
}}
om-title {{ color: #152238; font: 700 28px sans-serif; }}
{extra_style}</style></head><body>
<om-film><om-scene>
  <om-shot duration="1s"><om-title>Stable opening</om-title></om-shot>
  <om-shot class="{closing_class}" duration="1s"><om-title>{closing}</om-title></om-shot>
</om-scene></om-film>
</body></html>
"#,
    )
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
            "-x264-params",
            "colorprim=bt709:transfer=bt709:colormatrix=bt709:range=limited",
            "-color_range",
            "tv",
            "-colorspace",
            "bt709",
            "-color_trc",
            "bt709",
            "-color_primaries",
            "bt709",
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
    capture_fingerprint(&headless_shell(), BrowserCaptureMode::BeginFrame, fixture).await
}

async fn capture_portable_fingerprint(browser: &Path, fixture: &Url) -> RawRgbaHash {
    capture_fingerprint(browser, BrowserCaptureMode::Screenshot, fixture).await
}

async fn capture_fingerprint(
    browser: &Path,
    capture_mode: BrowserCaptureMode,
    fixture: &Url,
) -> RawRgbaHash {
    let mut session = BrowserSession::launch(
        browser,
        browser_options(capture_mode, Duration::from_secs(10)),
    )
    .await
    .expect("the requested browser must launch");
    let result = exercise_protocol(&mut session, fixture).await;
    let shutdown = session.shutdown().await;

    let fingerprint = result.expect("the real browser protocol must capture deterministic frames");
    shutdown.expect("the requested browser must shut down cleanly");
    fingerprint
}

async fn capture_temporal_sequence(fixture: &Url) -> Vec<RawRgbaHash> {
    capture_temporal_sequence_with(
        &headless_shell(),
        BrowserLaunchPolicy::local(),
        BrowserGraphicsBackend::SwiftShader,
        BrowserCaptureMode::BeginFrame,
        fixture,
    )
    .await
}

#[cfg(target_os = "macos")]
async fn capture_portable_temporal_sequence(
    browser: &Path,
    graphics_backend: BrowserGraphicsBackend,
    fixture: &Url,
) -> Vec<RawRgbaHash> {
    capture_temporal_sequence_with(
        browser,
        BrowserLaunchPolicy::local(),
        graphics_backend,
        BrowserCaptureMode::Screenshot,
        fixture,
    )
    .await
}

async fn capture_temporal_sequence_with(
    browser: &Path,
    launch_policy: BrowserLaunchPolicy,
    graphics_backend: BrowserGraphicsBackend,
    capture_mode: BrowserCaptureMode,
    fixture: &Url,
) -> Vec<RawRgbaHash> {
    let mut session = BrowserSession::launch(
        browser,
        BrowserSessionOptions {
            launch_policy,
            graphics_backend,
            capture_mode,
            render_profile: render_profile(),
            limits: browser_limits(Duration::from_secs(10)),
        },
    )
    .await
    .expect("the requested browser must launch");
    let result = exercise_temporal_sequence(&mut session, fixture).await;
    let shutdown = session.shutdown().await;

    let fingerprints = result.expect("the temporal experiment must capture every frame");
    shutdown.expect("the requested browser must shut down cleanly");
    fingerprints
}

async fn exercise_temporal_sequence(
    session: &mut BrowserSession,
    fixture: &Url,
) -> Result<Vec<RawRgbaHash>, Box<dyn Error>> {
    load_and_prepare(session, fixture).await?;
    let frame_rate = browser_plan_fixture().frame_rate();
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
        .capture_frame(frame(15), browser_plan_fixture().frame_rate())
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

async fn load_and_prepare(
    session: &mut BrowserSession,
    fixture: &Url,
) -> Result<(), Box<dyn Error>> {
    session.navigate(fixture, &fixture_root(fixture)).await?;
    let plan = browser_plan_fixture();
    let frame_rate = plan.frame_rate();
    let loaded = session
        .dispatch(&BrowserRequest::new(
            RequestId::new(1),
            BrowserCommand::load(plan, BrowserMediaMode::Decoded),
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
        &BrowserEvent::Prepared {
            evaluation_start,
            media_layout: onmark_core::protocol::BrowserMediaLayout::empty(),
        },
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

async fn assert_prores_4444_alpha(output: &Path) {
    let probe = Command::new(required_path("ONMARK_FFPROBE"))
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,pix_fmt",
            "-of",
            "json",
            "--",
        ])
        .arg(output)
        .output();
    let probe = timeout(Duration::from_secs(10), probe)
        .await
        .expect("alpha probing must finish before its deadline")
        .expect("ffprobe must inspect the completed MOV");
    assert!(
        probe.status.success(),
        "{}",
        String::from_utf8_lossy(&probe.stderr),
    );
    let value: serde_json::Value =
        serde_json::from_slice(&probe.stdout).expect("ffprobe must emit JSON");
    let stream = &value["streams"][0];
    assert_eq!(stream["codec_name"], "prores");
    assert_eq!(stream["pix_fmt"], "yuva444p12le");

    let decoded = Command::new(required_path("ONMARK_FFMPEG"))
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(output)
        .args(["-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output();
    let decoded = timeout(Duration::from_secs(10), decoded)
        .await
        .expect("alpha decoding must finish before its deadline")
        .expect("FFmpeg must decode the completed MOV");
    assert!(
        decoded.status.success(),
        "{}",
        String::from_utf8_lossy(&decoded.stderr),
    );
    let (mut transparent, mut translucent) = (false, false);
    for alpha in decoded.stdout.iter().skip(3).step_by(4) {
        transparent |= *alpha == 0;
        translucent |= *alpha > 0 && *alpha < u8::MAX;
    }
    assert!(transparent, "the output must retain transparent pixels");
    assert!(translucent, "the output must retain translucent pixels");
}

struct DecodedOutput {
    video_hashes: Vec<String>,
    audio_hashes: Vec<String>,
    audio_start_micros: i64,
    audio_presentation_samples: u64,
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
        audio_presentation_samples: audio_presentation_samples(output).await,
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

async fn audio_presentation_samples(output: &Path) -> u64 {
    let probe = Command::new(required_path("ONMARK_FFPROBE"))
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=duration_ts,time_base",
            "-of",
            "json",
            "--",
        ])
        .arg(output)
        .output();
    let probe = timeout(Duration::from_secs(10), probe)
        .await
        .expect("audio duration probing must finish before its deadline")
        .expect("ffprobe must inspect the output audio");
    assert!(
        probe.status.success(),
        "{}",
        String::from_utf8_lossy(&probe.stderr),
    );
    let response: AudioDurationProbe =
        serde_json::from_slice(&probe.stdout).expect("ffprobe must emit its JSON response");
    let [stream] = response.streams.as_slice() else {
        panic!("ffprobe must report exactly one audio stream");
    };
    assert_eq!(
        stream.time_base, "1/48000",
        "the final audio stream must retain the fixed output sample grid",
    );
    stream.duration_ts
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

fn browser_options(capture_mode: BrowserCaptureMode, deadline: Duration) -> BrowserSessionOptions {
    BrowserSessionOptions {
        launch_policy: BrowserLaunchPolicy::local(),
        graphics_backend: BrowserGraphicsBackend::SwiftShader,
        capture_mode,
        render_profile: render_profile(),
        limits: browser_limits(deadline),
    }
}

fn render_profile() -> RenderProfile {
    RenderProfile::new(WIDTH, HEIGHT).expect("the fixture render profile is valid")
}

fn transparent_profile() -> RenderProfile {
    render_profile().with_alpha(AlphaMode::Preserve)
}

fn capture_environment() -> CaptureEnvironmentId {
    CaptureEnvironmentId::from_sha256([7; CaptureEnvironmentId::BYTE_LENGTH])
}

fn real_executor(max_frames: u64) -> RenderExecutor {
    render_executor(headless_shell(), BrowserCaptureMode::BeginFrame, max_frames)
}

fn portable_executor(max_frames: u64) -> RenderExecutor {
    render_executor(
        required_path("ONMARK_PORTABLE_CHROME"),
        BrowserCaptureMode::Screenshot,
        max_frames,
    )
}

fn layered_executor(max_frames: u64) -> RenderExecutor {
    if let Some(browser) = env::var_os("ONMARK_HEADLESS_SHELL") {
        return render_executor(
            PathBuf::from(browser),
            BrowserCaptureMode::BeginFrame,
            max_frames,
        );
    }
    let browser = env::var_os("ONMARK_PORTABLE_CHROME")
        .map(PathBuf::from)
        .expect("ONMARK_HEADLESS_SHELL or ONMARK_PORTABLE_CHROME must name an executable");
    render_executor(browser, BrowserCaptureMode::Screenshot, max_frames)
}

fn render_executor(
    browser: PathBuf,
    capture_mode: BrowserCaptureMode,
    max_frames: u64,
) -> RenderExecutor {
    render_executor_with_profile(browser, capture_mode, max_frames, EncodeProfile::H264Mp4)
}

fn render_executor_with_profile(
    browser: PathBuf,
    capture_mode: BrowserCaptureMode,
    max_frames: u64,
    profile: EncodeProfile,
) -> RenderExecutor {
    let limits = EncodeLimits::new(
        Duration::from_secs(30),
        max_frames,
        64 * 1024 * 1024,
        64 * 1024,
    )
    .expect("the fixture encoding limits are bounded");
    let ffmpeg = Ffmpeg::new(required_path("ONMARK_FFMPEG"), limits, profile)
        .expect("the FFmpeg executable path is present");

    RenderExecutor::new(
        browser,
        capture_mode,
        browser_limits(Duration::from_secs(10)),
        ffmpeg,
    )
}

fn executor_with_profile(max_frames: u64, profile: EncodeProfile) -> RenderExecutor {
    if let Some(browser) = env::var_os("ONMARK_HEADLESS_SHELL") {
        return render_executor_with_profile(
            PathBuf::from(browser),
            BrowserCaptureMode::BeginFrame,
            max_frames,
            profile,
        );
    }
    let browser = env::var_os("ONMARK_PORTABLE_CHROME")
        .map(PathBuf::from)
        .expect("ONMARK_HEADLESS_SHELL or ONMARK_PORTABLE_CHROME must name an executable");
    render_executor_with_profile(browser, BrowserCaptureMode::Screenshot, max_frames, profile)
}

fn frame_artifact_limits() -> FrameArtifactLimits {
    FrameArtifactLimits::new(100, 64 * 1024 * 1024, 8 * 1024 * 1024)
        .expect("the fixture artifact limits are bounded")
}

fn browser_fixture() -> Url {
    let repository = repository();
    let fixture = repository.join("conformance/browser/runtime-protocol.html");
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

fn fixture_root(fixture: &Url) -> PathBuf {
    let path = fixture
        .to_file_path()
        .expect("the browser fixture must be a file URL");
    let repository = repository();

    // Checked-in fixture modules import the built runtime across repository
    // directories. Ephemeral bundles are self-contained beneath their parent.
    if path.starts_with(&repository) {
        return repository;
    }

    path.parent()
        .expect("the browser fixture must have a parent directory")
        .to_owned()
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("render is nested at crates/render")
        .to_owned()
}

fn browser_plan_fixture() -> BrowserPlan {
    BrowserPlan::from_timeline(&synthetic_timeline(), &BTreeMap::new())
        .expect("the fixture timeline fits the browser frame domain")
}

fn synthetic_timeline() -> onmark_core::timeline::TimelineIr {
    solve_timeline(
        concat!(
            "<om-film><om-scene>",
            r#"<om-shot duration="2.5s"><om-title>Opening</om-title></om-shot>"#,
            "</om-scene></om-film>",
        ),
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

fn executable_video_unit(
    bundle: &FixtureBundle,
    frozen: FrozenAsset,
    source: PathBuf,
) -> ExecutableUnit {
    let timeline = video_timeline_fixture(frozen.clone());
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

fn video_timeline_fixture(frozen: FrozenAsset) -> onmark_core::timeline::TimelineIr {
    let asset = AssetRef::parse("source.mp4").expect("the fixture asset reference is valid");
    let assets = BTreeMap::from([(asset, frozen)]);
    solve_timeline(
        concat!(
            "<om-film><om-scene><om-shot>",
            r#"<video src="source.mp4"></video>"#,
            "</om-shot></om-scene></om-film>",
        ),
        &assets,
    )
}

struct BackdropFixture {
    partition_plan: PartitionPlan,
    whole_film: ExecutableUnit,
    partitioned_units: Vec<ExecutableUnit>,
    assembly_units: Vec<ExecutableUnit>,
}

impl BackdropFixture {
    async fn materialize(workspace: &Path, source: &Path, bundle: &FixtureBundle) -> Self {
        let video_path = workspace.join("source.mp4");
        generate_source_video(&video_path, "1").await;
        let video = freeze_asset(&video_path).await;
        let assets = BTreeMap::from([(asset_ref("source.mp4"), video.clone())]);
        let source = fs::read_to_string(source).expect("the backdrop screenplay is readable");
        let timeline = solve_timeline(&source, &assets);
        let partition_plan =
            RenderGraph::from_timeline(&timeline, bundle.manifest.temporal_capability())
                .expect("the backdrop fixture has complete render ownership")
                .into_partition();
        assert_eq!(partition_plan.units().len(), 2);

        let materialized =
            MaterializedAsset::new(video, video_path).expect("the backdrop source path is present");
        let whole_film = RenderUnit::whole_film(
            &timeline,
            bundle.manifest.clone(),
            render_profile(),
            [materialized.clone()],
        )
        .expect("the complete backdrop fixture forms one unit");
        assert!(whole_film.visual_execution().backdrop_media().is_some());
        let whole_film = bundle.materialize(whole_film);
        let partitioned_units =
            Self::partition_units(&timeline, &partition_plan, bundle, materialized.clone());
        let assembly_units =
            Self::partition_units(&timeline, &partition_plan, bundle, materialized);

        Self {
            partition_plan,
            whole_film,
            partitioned_units,
            assembly_units,
        }
    }

    fn partition_units(
        timeline: &onmark_core::timeline::TimelineIr,
        partitions: &PartitionPlan,
        bundle: &FixtureBundle,
        materialized: MaterializedAsset,
    ) -> Vec<ExecutableUnit> {
        let manifests = bundle.region_manifests(partitions.units().len());
        RenderUnit::from_partitioned_bundles(
            timeline,
            partitions,
            manifests,
            render_profile(),
            [materialized],
        )
        .expect("each backdrop partition forms one unit")
        .into_iter()
        .enumerate()
        .map(|(index, unit)| {
            if index == 0 {
                assert!(unit.visual_execution().backdrop_media().is_some());
            } else {
                assert!(unit.visual_execution().backdrop_media().is_none());
                assert_eq!(
                    unit.visual_execution().capability(),
                    PresentationVisualCapability::SeparableBackdrop,
                );
            }
            bundle.materialize_region(index, unit)
        })
        .collect()
    }
}

struct AudioSubtitleFixture {
    partition_plan: PartitionPlan,
    whole_film: ExecutableUnit,
    partitioned_units: Vec<ExecutableUnit>,
}

impl AudioSubtitleFixture {
    async fn materialize(workspace: &Path, bundle: &FixtureBundle, screenplay: &str) -> Self {
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
        let source = fs::read_to_string(repository().join(screenplay))
            .expect("the audio-and-subtitle screenplay fixture is readable");
        let timeline = solve_timeline(&source, &assets);
        let timeline = compiler::import_captions(timeline, [caption_track()])
            .expect("fixture captions must enter the frame grid");
        let partition_plan =
            RenderGraph::from_timeline(&timeline, bundle.manifest.temporal_capability())
                .expect("the solved fixture has complete render ownership")
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
        let whole_film = RenderUnit::whole_film(
            &timeline,
            bundle.manifest.clone(),
            render_profile(),
            materialized_assets.clone(),
        )
        .expect("the complete fixture forms one whole-film unit");
        assert!(
            whole_film.visual_execution().layered_media().is_none(),
            "two simultaneous media placements keep the whole-film control in Chromium",
        );
        let whole_film = bundle.materialize(whole_film);
        let region_manifests = bundle.region_manifests(partition_plan.units().len());
        let partitioned_units: Vec<_> = RenderUnit::from_partitioned_bundles(
            &timeline,
            &partition_plan,
            region_manifests,
            render_profile(),
            materialized_assets,
        )
        .expect("the graph partitions form one local sequence")
        .into_iter()
        .enumerate()
        .map(|(index, unit)| bundle.materialize_region(index, unit))
        .collect();
        if bundle.manifest.visual_capability()
            == onmark_core::model::PresentationVisualCapability::SeparableOverlay
        {
            assert_native_source_edits(&partitioned_units);
        }
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

fn assert_native_source_edits(units: &[ExecutableUnit]) {
    for (index, unit) in units.iter().enumerate() {
        assert!(
            unit.visual_execution().layered_media().is_some(),
            "source-edited partition {index} must admit the native layered path",
        );
    }
}

struct StaticPartitionFixture {
    partition_plan: PartitionPlan,
    whole_film: ExecutableUnit,
    partitioned_units: Vec<ExecutableUnit>,
}

impl StaticPartitionFixture {
    fn materialize(source: &Path, bundle: &FixtureBundle) -> Self {
        Self::materialize_with_region_count(source, bundle, render_profile(), 2)
    }

    fn materialize_with_profile(
        source: &Path,
        bundle: &FixtureBundle,
        profile: RenderProfile,
    ) -> Self {
        Self::materialize_with_region_count(source, bundle, profile, 2)
    }

    fn materialize_with_region_count(
        source: &Path,
        bundle: &FixtureBundle,
        profile: RenderProfile,
        expected_regions: usize,
    ) -> Self {
        let source =
            fs::read_to_string(source).expect("the static presentation source must be readable");
        let timeline = solve_timeline(&source, &BTreeMap::new());
        let partitions =
            RenderGraph::from_timeline(&timeline, bundle.manifest.temporal_capability())
                .expect("the static fixture has complete render ownership")
                .into_partition();
        assert_eq!(
            partitions.units().len(),
            expected_regions,
            "the static fixture must produce its expected render regions",
        );

        let whole_film = RenderUnit::whole_film(&timeline, bundle.manifest.clone(), profile, [])
            .expect("the static fixture forms one whole-film unit");
        let region_manifests = bundle.region_manifests(partitions.units().len());
        let partitioned_units = RenderUnit::from_partitioned_bundles(
            &timeline,
            &partitions,
            region_manifests,
            profile,
            [],
        )
        .expect("the static fixture forms its partition units")
        .into_iter()
        .enumerate()
        .map(|(index, unit)| bundle.materialize_region(index, unit))
        .collect();

        Self {
            partition_plan: partitions,
            whole_film: bundle.materialize(whole_film),
            partitioned_units,
        }
    }
}

struct CapturedStaticFixture {
    whole: FrameArtifact,
    partitions: Vec<FrameArtifact>,
}

async fn capture_static_fixture(
    executor: &RenderExecutor,
    workspace: &Path,
    label: &str,
    fixture: &StaticPartitionFixture,
) -> CapturedStaticFixture {
    let whole_path = workspace.join(format!("{label}-whole.onmark-frames"));
    let whole = capture_and_reuse_static(executor, &fixture.whole_film, &whole_path).await;
    let partitions = capture_static_partitions(executor, workspace, label, fixture).await;

    CapturedStaticFixture { whole, partitions }
}

async fn capture_static_partitions(
    executor: &RenderExecutor,
    workspace: &Path,
    label: &str,
    fixture: &StaticPartitionFixture,
) -> Vec<FrameArtifact> {
    let mut partitions = Vec::with_capacity(fixture.partitioned_units.len());
    for (index, unit) in fixture.partitioned_units.iter().enumerate() {
        let path = workspace.join(format!("{label}-partition-{index}.onmark-frames"));
        let artifact = capture_and_reuse_static(executor, unit, &path).await;
        partitions.push(artifact);
    }
    partitions
}

async fn capture_partition_artifacts(
    executor: &RenderExecutor,
    workspace: &Path,
    label: &str,
    units: &[ExecutableUnit],
) -> Vec<FrameArtifact> {
    let mut artifacts = Vec::with_capacity(units.len());
    for (index, unit) in units.iter().enumerate() {
        let path = workspace.join(format!("{label}-{index}.onmark-frames"));
        artifacts.push(capture_and_reuse_static(executor, unit, &path).await);
    }
    artifacts
}

async fn capture_and_reuse_static(
    executor: &RenderExecutor,
    unit: &ExecutableUnit,
    path: &Path,
) -> FrameArtifact {
    let cold = executor
        .capture_frame_artifact_report(unit, capture_environment(), path, frame_artifact_limits())
        .await
        .expect("the static artifact must capture");
    assert!(
        cold.metrics().is_some(),
        "a missing artifact must execute its browser capture",
    );

    let warm = executor
        .capture_frame_artifact_report(unit, capture_environment(), path, frame_artifact_limits())
        .await
        .expect("the completed static artifact must be reusable");
    assert!(
        warm.metrics().is_none(),
        "a verified artifact must not launch another browser capture",
    );
    assert_eq!(warm.artifact().id(), cold.artifact().id());
    warm.into_artifact()
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

fn write_projection(path: &Path, regions: &[&[u32]]) {
    let regions = regions
        .iter()
        .map(|shot_indices| serde_json::json!({ "shotIndices": shot_indices }))
        .collect::<Vec<_>>();
    let projection = serde_json::json!({
        "version": 1,
        "regions": regions,
    });
    let encoded =
        serde_json::to_vec(&projection).expect("the fixture projection must encode as JSON");
    fs::write(path, encoded).expect("the fixture projection must be writable");
}

struct FixtureBundle {
    directory: PathBuf,
    manifest: BundleManifest,
}

impl FixtureBundle {
    fn checked_in() -> Self {
        let directory = repository().join("conformance/protocol/bundle-v1");
        Self::from_directory(directory)
    }

    async fn build_temporal(workspace: &Path) -> Self {
        Self::build(
            workspace,
            "temporal-bundle",
            "temporal-effects.html",
            "randomAccess",
            "browserComposite",
            "perFrame",
            ONE_SHOT_PROJECTION,
        )
        .await
    }

    async fn build_media_continuity(workspace: &Path) -> Self {
        Self::build_from(
            workspace,
            "media-continuity-bundle",
            &repository().join("conformance/cli/media-continuity.html"),
            "randomAccess",
            "browserComposite",
            "perFrame",
            TWO_SHOT_PROJECTION,
        )
        .await
    }

    async fn build_source_edits(workspace: &Path) -> Self {
        Self::build_from(
            workspace,
            "source-edit-bundle",
            &repository().join("conformance/cli/audio-subtitle.html"),
            "randomAccess",
            "separableOverlay",
            "placementBounded",
            TWO_SHOT_PROJECTION,
        )
        .await
    }

    async fn build_layered(workspace: &Path) -> Self {
        Self::build_layered_with(workspace, "layered-bundle", "placementBounded").await
    }

    async fn build_layered_per_frame(workspace: &Path) -> Self {
        Self::build_layered_with(workspace, "layered-per-frame-bundle", "perFrame").await
    }

    async fn build_layered_with(
        workspace: &Path,
        directory_name: &str,
        frame_behavior: &str,
    ) -> Self {
        Self::build(
            workspace,
            directory_name,
            "layered-presentation.html",
            "randomAccess",
            "separableOverlay",
            frame_behavior,
            ONE_SHOT_PROJECTION,
        )
        .await
    }

    async fn build(
        workspace: &Path,
        directory_name: &str,
        document_name: &str,
        temporal_capability: &str,
        visual_capability: &str,
        frame_behavior: &str,
        projection: &[&[u32]],
    ) -> Self {
        Self::build_from(
            workspace,
            directory_name,
            &repository().join("conformance/browser").join(document_name),
            temporal_capability,
            visual_capability,
            frame_behavior,
            projection,
        )
        .await
    }

    async fn build_from(
        workspace: &Path,
        directory_name: &str,
        document: &Path,
        temporal_capability: &str,
        visual_capability: &str,
        frame_behavior: &str,
        projection: &[&[u32]],
    ) -> Self {
        let directory = workspace.join(directory_name);
        let projection_path = workspace.join(format!("{directory_name}-projection.json"));
        write_projection(&projection_path, projection);
        let bundled = Command::new(required_path("ONMARK_BUNDLER"))
            .args(["--html"])
            .arg(document)
            .args(["--output"])
            .arg(&directory)
            .args(["--max-output-bytes", "2000000"])
            .args(["--projection"])
            .arg(&projection_path)
            .args(["--frame-behavior", frame_behavior])
            .args(["--temporal-capability", temporal_capability])
            .args(["--visual-capability", visual_capability])
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

        Self::from_directory(directory)
    }

    fn from_directory(directory: PathBuf) -> Self {
        let manifest = fs::read_to_string(directory.join(BundleManifest::FILE_NAME))
            .expect("the executable bundle manifest is readable");
        let manifest: BundleManifest =
            serde_json::from_str(&manifest).expect("the executable bundle manifest is valid");
        Self {
            directory,
            manifest,
        }
    }

    fn entry_url(&self) -> Url {
        Url::from_file_path(self.directory.join(BundleManifest::ENTRY_POINT))
            .expect("the fixture bundle path is absolute")
    }

    fn materialize(&self, unit: RenderUnit) -> ExecutableUnit {
        self.materialize_from(&self.directory, unit)
    }

    fn region_manifests(&self, count: usize) -> Vec<BundleManifest> {
        (0..count)
            .map(|index| self.region_manifest(index))
            .collect()
    }

    fn region_manifest(&self, index: usize) -> BundleManifest {
        let path = self.region_directory(index).join(BundleManifest::FILE_NAME);
        let manifest =
            fs::read_to_string(path).expect("the shot-scoped bundle manifest is readable");
        serde_json::from_str(&manifest).expect("the shot-scoped bundle manifest is valid")
    }

    fn materialize_region(&self, index: usize, unit: RenderUnit) -> ExecutableUnit {
        self.materialize_from(&self.region_directory(index), unit)
    }

    fn region_directory(&self, index: usize) -> PathBuf {
        self.directory
            .join(BundleManifest::REGION_DIRECTORY)
            .join(index.to_string())
    }

    fn materialize_from(&self, directory: &Path, unit: RenderUnit) -> ExecutableUnit {
        let limits = UnitRootLimits::new(UNIT_ROOT_FILE_LIMIT, 64 * 1024 * 1024)
            .expect("the fixture materialization limits are bounded");

        ExecutableUnit::materialize(unit, directory, limits)
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

#[derive(Debug, Deserialize)]
struct AudioDurationProbe {
    streams: Vec<AudioDurationStream>,
}

#[derive(Debug, Deserialize)]
struct AudioDurationStream {
    duration_ts: u64,
    time_base: String,
}

#[test]
fn parses_audio_packet_timestamps_without_floating_point() {
    assert_eq!(timestamp_micros("0.978"), 978_000);
    assert_eq!(timestamp_micros("-0.021333"), -21_333);
}
