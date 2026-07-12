use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use onmark_core::compiler;
use onmark_core::model::{
    AssetRef, Duration as MediaDuration, FrameRate, FrozenAsset, FrozenAssetId, SourceId, Timebase,
};
use onmark_media::{Ffprobe, InvalidFfprobe, ProbeError};

#[test]
fn normalizes_exact_duration_from_ffprobe() {
    let ffprobe = fixture_probe(Duration::from_secs(1), 4_096);
    let metadata = ffprobe
        .probe(Path::new("valid.mp4"))
        .expect("the fixture response is valid");

    assert_eq!(
        metadata.duration(),
        MediaDuration::from_nanos(2_500_000_000)
    );
}

#[test]
fn frozen_identity_and_probed_metadata_drive_timeline_solving() {
    let ffprobe = fixture_probe(Duration::from_secs(1), 4_096);
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
        75,
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
    let ffprobe = fixture_probe(Duration::from_secs(1), 4_096);

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
}

#[test]
fn failed_probes_retain_the_artifact_and_stderr_tail() {
    let ffprobe = fixture_probe(Duration::from_secs(1), 64);
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
    let ffprobe = fixture_probe(Duration::from_secs(1), 64);
    for path in ["large.mp4", "large-stderr.mp4"] {
        let error = ffprobe
            .probe(Path::new(path))
            .expect_err("the fixture exceeds its capture limit");

        assert!(matches!(error, ProbeError::OutputLimit(_)));
    }
}

fn fixture_probe(timeout: Duration, output_limit: usize) -> Ffprobe {
    Ffprobe::new(fixture("ffprobe"), timeout, output_limit)
        .expect("the fixture probe limits are valid")
}

fn probe_error(ffprobe: &Ffprobe, path: &str) -> ProbeError {
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
