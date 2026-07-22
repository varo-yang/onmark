//! ffprobe process and normalization conformance over bounded fixtures.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use onmark_core::compiler;
use onmark_core::model::{
    AssetMetadata, AssetRef, AudioChannelLayout, AudioSampleRate, Duration as MediaDuration,
    FrameRate, FrozenAsset, FrozenAssetId, SourceId, Timebase, VideoColorProfile, VideoTiming,
};
use onmark_media::{Ffprobe, InvalidFfprobe, ProbeError};

static FIXTURE_PROCESS_LOCK: Mutex<()> = Mutex::new(());
const RESPONSIVE_FIXTURE_TIMEOUT: Duration = Duration::from_secs(5);

/// One serialized owner of the fixture executable and its process deadline.
///
/// The fixture deliberately includes an unresponsive probe. Keeping its
/// process tests isolated prevents that deadline case from changing unrelated
/// fixture assertions on a constrained CI worker.
struct FixtureProbe {
    ffprobe: Ffprobe,
    _process_lock: MutexGuard<'static, ()>,
}

impl FixtureProbe {
    fn probe(&self, path: &Path) -> Result<AssetMetadata, ProbeError> {
        self.ffprobe.probe(path)
    }
}

#[test]
fn normalizes_exact_duration_from_ffprobe() {
    let ffprobe = responsive_fixture_probe(4_096);
    let metadata = ffprobe
        .probe(Path::new("valid.mp4"))
        .expect("the fixture response is valid");

    assert_eq!(
        metadata.duration(),
        MediaDuration::from_nanos(2_500_000_000)
    );
    let video = metadata
        .video_metadata()
        .expect("the fixture contains a video stream");
    assert_eq!(video.codec(), "h264");
    assert_eq!(video.pixel_format(), "yuv420p");
    assert_eq!(video.dimensions().width(), 1_920);
    assert_eq!(video.dimensions().height(), 1_080);
    assert_eq!(video.color_profile(), Some(VideoColorProfile::Bt709Limited),);
    assert_eq!(video.duration(), MediaDuration::from_nanos(2_000_000_000));
    assert_eq!(
        video.timing(),
        VideoTiming::Constant(FrameRate::new(30, 1).expect("30 fps is valid")),
    );
    assert!(!metadata.has_audio_stream());
}

#[test]
fn does_not_admit_a_partial_source_color_profile() {
    let ffprobe = responsive_fixture_probe(4_096);
    let metadata = ffprobe
        .probe(Path::new("partial-color.mp4"))
        .expect("partial color facts do not invalidate the browser media path");

    assert_eq!(
        metadata
            .video_metadata()
            .expect("the fixture contains a video stream")
            .color_profile(),
        None,
    );
}

#[test]
fn records_audio_presence_independently_of_visual_metadata() {
    let ffprobe = responsive_fixture_probe(4_096);
    let audio = ffprobe
        .probe(Path::new("audio.mp3"))
        .expect("the fixture contains no video stream");
    let audiovisual = ffprobe
        .probe(Path::new("audiovisual.mp4"))
        .expect("the fixture contains both required tracks");
    let metadata_only = ffprobe
        .probe(Path::new("metadata-only.bin"))
        .expect("the fixture contains no media track");

    assert!(audio.has_audio_stream());
    assert!(audio.video_metadata().is_none());
    assert_eq!(
        audio
            .audio_metadata()
            .expect("the audio fixture has an audio stream")
            .duration(),
        MediaDuration::from_nanos(2_500_000_000),
    );
    assert_eq!(
        audio
            .audio_metadata()
            .expect("the audio fixture has an audio stream")
            .sample_rate(),
        AudioSampleRate::new(48_000).expect("48 kHz is valid"),
    );
    assert_eq!(
        audio
            .audio_metadata()
            .expect("the audio fixture has an audio stream")
            .channel_layout(),
        AudioChannelLayout::Mono,
    );
    assert!(audiovisual.has_audio_stream());
    assert!(audiovisual.video_metadata().is_some());
    assert_eq!(
        audiovisual
            .audio_metadata()
            .expect("the audiovisual fixture has an audio stream")
            .duration(),
        MediaDuration::from_nanos(1_500_000_000),
    );
    assert_eq!(
        audiovisual
            .audio_metadata()
            .expect("the audiovisual fixture has an audio stream")
            .sample_rate(),
        AudioSampleRate::new(44_100).expect("44.1 kHz is valid"),
    );
    assert_eq!(
        audiovisual
            .audio_metadata()
            .expect("the audiovisual fixture has an audio stream")
            .channel_layout(),
        AudioChannelLayout::Stereo,
    );
    assert!(!metadata_only.has_audio_stream());
    assert!(metadata_only.video_metadata().is_none());
}

#[test]
fn selects_default_media_streams_and_ignores_attached_pictures() {
    let ffprobe = responsive_fixture_probe(8_192);
    let metadata = ffprobe
        .probe(Path::new("stream-selection.mp4"))
        .expect("the default media streams are usable");

    let video = metadata
        .video_metadata()
        .expect("the fixture has a selected video stream");
    assert_eq!(video.duration(), MediaDuration::from_nanos(2_000_000_000));
    assert_eq!(
        video.timing(),
        VideoTiming::Constant(FrameRate::new(30, 1).expect("30 fps is valid")),
    );

    let audio = metadata
        .audio_metadata()
        .expect("the fixture has a selected audio stream");
    assert_eq!(
        audio.sample_rate(),
        AudioSampleRate::new(48_000).expect("48 kHz is valid"),
    );
    assert_eq!(audio.channel_layout(), AudioChannelLayout::Stereo);
}

#[test]
fn derives_artifact_duration_from_streams_when_format_duration_is_absent() {
    let ffprobe = responsive_fixture_probe(4_096);
    let metadata = ffprobe
        .probe(Path::new("stream-duration-only.mp4"))
        .expect("stream durations are sufficient when format duration is absent");

    assert_eq!(
        metadata.duration(),
        MediaDuration::from_nanos(2_000_000_000)
    );
    assert_eq!(
        metadata
            .audio_metadata()
            .expect("the fixture has an audio stream")
            .duration(),
        MediaDuration::from_nanos(1_500_000_000),
    );
    assert_eq!(
        metadata
            .video_metadata()
            .expect("the fixture has a video stream")
            .duration(),
        MediaDuration::from_nanos(2_000_000_000),
    );
}

#[test]
fn derives_video_duration_from_the_container_when_the_stream_omits_it() {
    let ffprobe = responsive_fixture_probe(4_096);
    let metadata = ffprobe
        .probe(Path::new("format-duration-only.mp4"))
        .expect("the container duration can bound a video stream");

    assert_eq!(
        metadata
            .video_metadata()
            .expect("the fixture has a video stream")
            .duration(),
        MediaDuration::from_nanos(2_500_000_000),
    );
}

#[test]
fn distinguishes_variable_and_still_video_timing() {
    let ffprobe = responsive_fixture_probe(4_096);
    let variable = ffprobe
        .probe(Path::new("variable.mp4"))
        .expect("the fixture reports conflicting stream frame rates");
    let still = ffprobe
        .probe(Path::new("still.mp4"))
        .expect("the fixture contains one video frame");

    assert_eq!(
        variable
            .video_metadata()
            .expect("the fixture has video")
            .timing(),
        VideoTiming::Variable,
    );
    assert_eq!(
        still
            .video_metadata()
            .expect("the fixture has video")
            .timing(),
        VideoTiming::Still,
    );
}

#[test]
fn frozen_identity_and_probed_metadata_drive_timeline_solving() {
    let ffprobe = responsive_fixture_probe(4_096);
    let metadata = ffprobe
        .probe(Path::new("valid.mp4"))
        .expect("the fixture response is valid");
    let asset = AssetRef::parse("valid.mp4").expect("the fixture asset reference is valid");
    let frozen = FrozenAsset::new(FrozenAssetId::from_sha256([1; 32]), metadata);
    let assets = BTreeMap::from([(asset, frozen)]);
    let parsed = compiler::parse(
        SourceId::new(0),
        r#"<film><scene><shot><video src="valid.mp4" /></shot></scene></film>"#,
    );
    let (document, diagnostics) = parsed.into_parts();
    assert!(diagnostics.is_empty());
    let bound = compiler::bind(document);
    let (film, diagnostics) = bound.into_parts();
    assert!(diagnostics.is_empty());
    let resolved = compiler::resolve(film.expect("the fixture contains one film"));
    let (film, diagnostics) = resolved.into_parts();
    assert!(diagnostics.is_empty());
    let rate = FrameRate::new(30, 1).expect("30 fps is valid");
    let solved = compiler::solve(
        film.expect("the fixture resolves"),
        &assets,
        Timebase::new(rate),
    )
    .expect("the probed asset metadata is complete");

    assert!(solved.diagnostics().is_empty());
    assert_eq!(
        solved
            .timeline()
            .expect("the fixture solves")
            .interval()
            .end()
            .get(),
        60,
    );
}

#[test]
fn rejects_probe_limits_outside_the_safety_ceiling() {
    assert_eq!(
        Ffprobe::new("", Duration::from_secs(1), 1).expect_err("an executable path is required"),
        InvalidFfprobe::EmptyExecutable,
    );
    assert_eq!(
        Ffprobe::new("ffprobe", Duration::ZERO, 1).expect_err("zero cannot bound lifetime"),
        InvalidFfprobe::ZeroTimeout,
    );
    assert_eq!(
        Ffprobe::new("ffprobe", Ffprobe::MAX_TIMEOUT + Duration::from_nanos(1), 1,)
            .expect_err("process lifetime has a fixed ceiling"),
        InvalidFfprobe::TimeoutTooLong,
    );
    assert_eq!(
        Ffprobe::new("ffprobe", Duration::from_secs(1), 0)
            .expect_err("zero bytes cannot carry probe output"),
        InvalidFfprobe::ZeroOutputLimit,
    );
    assert_eq!(
        Ffprobe::new(
            "ffprobe",
            Duration::from_secs(1),
            Ffprobe::MAX_OUTPUT_BYTES + 1,
        )
        .expect_err("the fixed capture ceiling is one MiB"),
        InvalidFfprobe::OutputLimitTooLarge,
    );

    Ffprobe::new("ffprobe", Ffprobe::MAX_TIMEOUT, Ffprobe::MAX_OUTPUT_BYTES)
        .expect("the published safety ceilings are inclusive");
}

#[test]
fn translates_process_and_response_failures() {
    let ffprobe = responsive_fixture_probe(4_096);

    assert!(matches!(
        probe_error(&ffprobe, "failed.mp4"),
        ProbeError::Failed(_)
    ));
    assert!(matches!(
        probe_error(&ffprobe, "invalid-json.mp4"),
        ProbeError::InvalidResponse(_)
    ));
    assert!(matches!(
        probe_error(&ffprobe, "missing-duration.mp4"),
        ProbeError::MissingDuration(_)
    ));
    assert!(matches!(
        probe_error(&ffprobe, "invalid-duration.mp4"),
        ProbeError::InvalidDuration(_)
    ));
    assert!(matches!(
        probe_error(&ffprobe, "empty-video.mp4"),
        ProbeError::InvalidVideo(_)
    ));
    assert!(matches!(
        probe_error(&ffprobe, "invalid-video-duration.mp4"),
        ProbeError::InvalidVideo(_)
    ));
    assert!(matches!(
        probe_error(&ffprobe, "missing-video-width.mp4"),
        ProbeError::InvalidVideo(_)
    ));
    assert!(matches!(
        probe_error(&ffprobe, "zero-video-height.mp4"),
        ProbeError::InvalidVideo(_)
    ));
    assert!(matches!(
        probe_error(&ffprobe, "missing-audio-sample-rate.mp3"),
        ProbeError::InvalidAudio(_)
    ));
    assert!(matches!(
        probe_error(&ffprobe, "missing-audio-channels.mp3"),
        ProbeError::InvalidAudio(_)
    ));
    assert!(matches!(
        probe_error(&ffprobe, "surround.mp3"),
        ProbeError::InvalidAudio(_)
    ));
    assert!(matches!(
        probe_error(&ffprobe, "excessive-channels.mp3"),
        ProbeError::InvalidAudio(_)
    ));
    assert!(matches!(
        probe_error(&ffprobe, "invalid-audio-duration.mp3"),
        ProbeError::InvalidAudio(_)
    ));
}

#[test]
fn failed_probes_retain_the_artifact_and_stderr_tail() {
    let ffprobe = responsive_fixture_probe(64);
    let path = Path::new("failed-tail.mp4");
    let error = ffprobe
        .probe(path)
        .expect_err("the fixture exits after a long stderr preamble");
    let message = error.to_string();

    assert_eq!(error.path(), path);
    assert!(matches!(error, ProbeError::Failed(_)));
    assert!(message.contains("final probe failure"));
    assert!(message.contains("[truncated]"));
}

#[test]
fn translates_an_unavailable_executable() {
    let ffprobe = Ffprobe::new(fixture("does-not-exist"), Duration::from_secs(1), 4_096)
        .expect("the fixture probe limits are valid");
    let error = ffprobe
        .probe(Path::new("valid.mp4"))
        .expect_err("the configured executable does not exist");

    assert!(matches!(&error, ProbeError::Spawn(_)));
    assert!(error.source().is_some());
}

#[test]
fn terminates_a_probe_that_exceeds_its_deadline() {
    let ffprobe = fixture_probe(Duration::from_millis(30), 4_096);
    let error = ffprobe
        .probe(Path::new("slow.mp4"))
        .expect_err("the fixture runs beyond its deadline");

    assert!(matches!(error, ProbeError::Timeout(_)));
}

#[test]
fn drains_but_does_not_retain_output_past_the_limit() {
    let ffprobe = responsive_fixture_probe(64);
    for path in ["large.mp4", "large-stderr.mp4"] {
        let error = ffprobe
            .probe(Path::new(path))
            .expect_err("the fixture exceeds its capture limit");

        assert!(matches!(error, ProbeError::OutputLimit(_)));
    }
}

fn fixture_probe(timeout: Duration, output_limit: usize) -> FixtureProbe {
    let process_lock = fixture_process_lock();
    let ffprobe = Ffprobe::new(fixture("ffprobe"), timeout, output_limit)
        .expect("the fixture probe limits are valid");

    FixtureProbe {
        ffprobe,
        _process_lock: process_lock,
    }
}

fn responsive_fixture_probe(output_limit: usize) -> FixtureProbe {
    fixture_probe(RESPONSIVE_FIXTURE_TIMEOUT, output_limit)
}

fn fixture_process_lock() -> MutexGuard<'static, ()> {
    match FIXTURE_PROCESS_LOCK.lock() {
        Ok(lock) => lock,
        // A failing assertion must not conceal later independent fixture tests.
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn probe_error(ffprobe: &FixtureProbe, path: &str) -> ProbeError {
    ffprobe
        .probe(Path::new(path))
        .expect_err("the fixture response is intentionally invalid")
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}
