//! Private unit-root materialization and hostile-input boundary tests.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use onmark_core::compiler;
use onmark_core::model::{
    AssetMetadata, AssetRef, Duration, FrameRate, FrozenAsset, FrozenAssetId,
    PresentationTemporalCapability, PresentationVisualCapability, SourceId, Timebase,
    VideoDimensions, VideoMetadata, VideoTiming,
};
use onmark_core::protocol::{BundleFile, BundleManifest};
use onmark_render::{
    CaptureEnvironmentId, InvalidUnitRootLimits, MaterializedAsset, RenderProfile, RenderUnit,
    UnitRoot, UnitRootErrorKind, UnitRootLimits,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use tempfile::tempdir;

#[test]
fn materializes_verified_bundle_and_asset_bytes() {
    let fixture = Fixture::new();
    let root = UnitRoot::materialize(
        fixture.bundle.path(),
        &fixture.manifest,
        [&fixture.asset],
        limits(8, 4_096),
    )
    .expect("verified inputs form one private unit root");

    assert_eq!(read(&root.path().join("index.html")), b"page");
    assert_eq!(
        read(&root.path().join(fixture.asset.unit_relative_path())),
        b"video",
    );
    assert!(root.path().join(BundleManifest::FILE_NAME).is_file());
    assert_eq!(root.entry_url().scheme(), "file");

    let path = root.path().to_owned();
    drop(root);
    assert!(!path.exists());
}

#[test]
fn materializes_the_checked_in_bundle_contract() {
    materialize_conformance_bundle("bundle-v1");
}

fn materialize_conformance_bundle(version: &str) {
    let root = conformance_bundle(version);
    let source = fs::read_to_string(root.join(BundleManifest::FILE_NAME))
        .expect("the conformance manifest is readable");
    let manifest = serde_json::from_str::<BundleManifest>(&source)
        .expect("the conformance manifest satisfies the Rust wire contract");
    let materialized = UnitRoot::materialize(&root, &manifest, [], limits(8, 1024 * 1024))
        .expect("the checked-in bundle must materialize across the native boundary");
    let entry = materialized
        .entry_url()
        .to_file_path()
        .expect("the executable entry is a local file");

    assert!(read(&entry).starts_with(b"<!doctype html>"));
}

#[test]
fn materializes_one_portable_worker_capture_request_inside_its_private_parent() {
    let fixture = Fixture::new();
    let unit = RenderUnit::whole_film(
        &static_timeline(),
        fixture.manifest.clone(),
        render_profile(),
        [],
    )
    .expect("the static fixture forms one render unit");
    let request = unit.worker_capture_request(capture_environment());
    let expected_plan = request.browser_plan().clone();
    let expected_profile = request.profile();
    let input = tempdir().expect("the worker input root is available");
    let private = tempdir().expect("the worker private parent is available");
    stage_worker_bundle(input.path(), &fixture);

    let executable = request
        .materialize_in(input.path(), private.path(), limits(4, 4_096))
        .expect("the worker request materializes into a private executable root");
    let entry = executable
        .entry_url()
        .to_file_path()
        .expect("the worker executable entry remains local");

    assert_eq!(read(&entry), b"page");
    assert!(entry.starts_with(private.path()));
    assert_eq!(executable.browser_plan(), &expected_plan);
    assert_eq!(executable.profile(), expected_profile);
}

#[test]
fn materializes_worker_video_bytes_from_the_frozen_identity_layout() {
    let fixture = Fixture::new();
    let unit = RenderUnit::whole_film(
        &video_timeline(fixture.asset.frozen().clone()),
        fixture.manifest.clone(),
        render_profile(),
        [fixture.asset.clone()],
    )
    .expect("the video fixture forms one render unit");
    let request = unit.worker_capture_request(capture_environment());
    let input = tempdir().expect("the worker input root is available");
    stage_worker_bundle(input.path(), &fixture);
    let source = input
        .path()
        .join(BundleManifest::asset_path(fixture.asset.id()));
    let parent = source
        .parent()
        .expect("a canonical asset path has a parent directory");
    fs::create_dir_all(parent).expect("the worker asset directory is writable");
    fs::copy(fixture.asset.local_path(), &source).expect("the frozen worker asset copies");

    let executable = request
        .materialize(input.path(), limits(8, 4_096))
        .expect("the worker request verifies and materializes its video bytes");
    let entry = executable
        .entry_url()
        .to_file_path()
        .expect("the fixture worker entry remains local");
    let root = entry
        .parent()
        .expect("the fixture worker entry remains below its root");

    assert_eq!(
        read(&root.join(BundleManifest::asset_path(fixture.asset.id()))),
        b"video",
    );
}

#[test]
fn rejects_payload_or_bundle_identity_drift() {
    let fixture = Fixture::new();
    fs::write(fixture.bundle.path().join("index.html"), b"changed")
        .expect("the fixture bundle is writable");
    let error = materialize(&fixture).expect_err("changed payload bytes must be rejected");
    assert_eq!(error.kind(), UnitRootErrorKind::SizeMismatch);

    let fixture = Fixture::new();
    fs::write(fixture.bundle.path().join("index.html"), b"fake")
        .expect("the fixture bundle is writable");
    let error = materialize(&fixture).expect_err("changed payload identity must be rejected");
    assert_eq!(error.kind(), UnitRootErrorKind::DigestMismatch);

    let invalid = BundleManifest::new(
        PresentationTemporalCapability::Sequential,
        PresentationVisualCapability::BrowserComposite,
        digest(b"wrong identity"),
        fixture.manifest.files().to_vec(),
    )
    .expect("the deliberately wrong identity is well formed");
    let error = UnitRoot::materialize(
        fixture.bundle.path(),
        &invalid,
        [&fixture.asset],
        limits(8, 4_096),
    )
    .expect_err("the canonical identity must be checked before payload IO");
    assert_eq!(error.kind(), UnitRootErrorKind::BundleIdentity);
}

#[test]
fn bounds_files_and_retained_bytes() {
    let fixture = Fixture::new();
    let file_error = UnitRoot::materialize(
        fixture.bundle.path(),
        &fixture.manifest,
        [&fixture.asset],
        limits(2, 4_096),
    )
    .expect_err("manifest, payload, and asset exceed two files");
    assert_eq!(file_error.kind(), UnitRootErrorKind::FileLimit);

    let byte_error = UnitRoot::materialize(
        fixture.bundle.path(),
        &fixture.manifest,
        [&fixture.asset],
        limits(8, 4),
    )
    .expect_err("the manifest alone exceeds four bytes");
    assert_eq!(byte_error.kind(), UnitRootErrorKind::ByteLimit);

    let duplicate_error = UnitRoot::materialize(
        fixture.bundle.path(),
        &fixture.manifest,
        [&fixture.asset, &fixture.asset],
        limits(8, 4_096),
    )
    .expect_err("one frozen asset identity may be materialized only once");
    assert_eq!(duplicate_error.kind(), UnitRootErrorKind::DuplicateAsset);
}

#[test]
fn rejects_empty_or_unbounded_limits() {
    assert_eq!(
        UnitRootLimits::new(0, 1),
        Err(InvalidUnitRootLimits::ZeroFiles),
    );
    assert_eq!(
        UnitRootLimits::new(100_001, 1),
        Err(InvalidUnitRootLimits::TooManyFiles),
    );
    assert_eq!(
        UnitRootLimits::new(1, 0),
        Err(InvalidUnitRootLimits::ZeroBytes),
    );
    assert_eq!(
        UnitRootLimits::new(1, (1 << 40) + 1),
        Err(InvalidUnitRootLimits::TooManyBytes),
    );
}

#[test]
fn rejects_file_limits_before_bundle_identity_work() {
    let bundle = tempdir().expect("the fixture bundle directory is available");
    let files = vec![
        BundleFile::new("index.html", 1, digest(b"index")).expect("index is valid"),
        BundleFile::new("presentation.js", 1, digest(b"script")).expect("script is valid"),
    ];
    let manifest = BundleManifest::new(
        PresentationTemporalCapability::Sequential,
        PresentationVisualCapability::BrowserComposite,
        digest(b"wrong identity"),
        files,
    )
    .expect("the deliberately wrong identity is well formed");

    let error = UnitRoot::materialize(bundle.path(), &manifest, [], limits(2, 4_096))
        .expect_err("the file bound must reject work before identity hashing");
    assert_eq!(error.kind(), UnitRootErrorKind::FileLimit);
}

#[cfg(unix)]
#[test]
fn rejects_bundle_symlinks() {
    use std::os::unix::fs::symlink;

    let fixture = Fixture::new();
    let target = fixture.bundle.path().join("target.html");
    fs::rename(fixture.bundle.path().join("index.html"), &target)
        .expect("the fixture payload can move");
    symlink(&target, fixture.bundle.path().join("index.html"))
        .expect("the fixture symlink can be created");

    let error = materialize(&fixture).expect_err("bundle symlinks cannot cross the boundary");
    assert_eq!(error.kind(), UnitRootErrorKind::InvalidSource);
}

struct Fixture {
    bundle: tempfile::TempDir,
    manifest: BundleManifest,
    asset: MaterializedAsset,
}

impl Fixture {
    fn new() -> Self {
        let bundle = tempdir().expect("the fixture bundle directory is available");
        fs::write(bundle.path().join("index.html"), b"page")
            .expect("the fixture bundle is writable");
        let files = vec![
            BundleFile::new("index.html", 4, digest(b"page"))
                .expect("the fixture bundle file is valid"),
        ];
        let manifest = manifest(files);

        let asset_file = bundle.path().join("source-video");
        fs::write(&asset_file, b"video").expect("the fixture asset is writable");
        let frozen = FrozenAsset::new(
            frozen_id(b"video"),
            AssetMetadata::video(
                Duration::from_nanos(1_000_000_000),
                VideoMetadata::new(
                    Duration::from_nanos(1_000_000_000),
                    VideoDimensions::new(1_920, 1_080).expect("fixture dimensions are positive"),
                    "h264",
                    "yuv420p",
                    VideoTiming::Constant(frame_rate()),
                )
                .expect("the fixture video metadata is normalized"),
            ),
        );
        let asset =
            MaterializedAsset::new(frozen, asset_file).expect("the fixture asset path is present");

        Self {
            bundle,
            manifest,
            asset,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleIdentity<'a> {
    version: u16,
    entry_point: &'a str,
    temporal_capability: &'a str,
    visual_capability: &'a str,
    files: &'a [BundleFile],
}

fn manifest(files: Vec<BundleFile>) -> BundleManifest {
    let identity = BundleIdentity {
        version: 1,
        entry_point: "index.html",
        temporal_capability: PresentationTemporalCapability::Sequential.as_str(),
        visual_capability: PresentationVisualCapability::BrowserComposite.as_str(),
        files: &files,
    };
    let identity = serde_json::to_vec(&identity).expect("the fixture identity serializes");
    BundleManifest::new(
        PresentationTemporalCapability::Sequential,
        PresentationVisualCapability::BrowserComposite,
        digest(&identity),
        files,
    )
    .expect("the fixture manifest is canonical")
}

fn materialize(fixture: &Fixture) -> Result<UnitRoot, onmark_render::UnitRootError> {
    UnitRoot::materialize(
        fixture.bundle.path(),
        &fixture.manifest,
        [&fixture.asset],
        limits(8, 4_096),
    )
}

fn read(path: &Path) -> Vec<u8> {
    fs::read(path).expect("the materialized fixture is readable")
}

fn conformance_bundle(version: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("conformance/protocol")
        .join(version)
}

fn static_timeline() -> onmark_core::timeline::TimelineIr {
    solve_timeline(
        r#"<film><scene><shot duration="1s"><title>Frame</title></shot></scene></film>"#,
        &BTreeMap::new(),
    )
}

fn video_timeline(frozen: FrozenAsset) -> onmark_core::timeline::TimelineIr {
    let assets = BTreeMap::from([(
        AssetRef::parse("opening.mp4").expect("the fixture asset reference is valid"),
        frozen,
    )]);
    solve_timeline(
        r#"<film><scene><shot><video src="opening.mp4" /></shot></scene></film>"#,
        &assets,
    )
}

fn solve_timeline(
    source: &str,
    assets: &BTreeMap<AssetRef, FrozenAsset>,
) -> onmark_core::timeline::TimelineIr {
    let parsed = compiler::parse(SourceId::new(0), source);
    let (document, diagnostics) = parsed.into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::bind(document).into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::resolve(film.expect("the fixture binds")).into_parts();
    assert!(diagnostics.is_empty());
    let report = compiler::solve(
        film.expect("the fixture resolves"),
        assets,
        Timebase::new(frame_rate()),
    )
    .expect("the fixture has complete frozen metadata");
    let (timeline, diagnostics) = report.into_parts();
    assert!(diagnostics.is_empty());
    timeline.expect("the fixture solves")
}

fn stage_worker_bundle(input: &Path, fixture: &Fixture) {
    let bundle = input.join("bundle");
    fs::create_dir(&bundle).expect("the worker bundle directory is writable");
    fs::copy(
        fixture.bundle.path().join("index.html"),
        bundle.join("index.html"),
    )
    .expect("the bundle payload copies into the worker layout");
}

fn render_profile() -> RenderProfile {
    RenderProfile::new(320, 180).expect("the fixture render profile is valid")
}

fn capture_environment() -> CaptureEnvironmentId {
    CaptureEnvironmentId::from_sha256([7; CaptureEnvironmentId::BYTE_LENGTH])
}

fn frame_rate() -> FrameRate {
    FrameRate::new(30, 1).expect("the fixture frame rate is valid")
}

fn frozen_id(bytes: &[u8]) -> FrozenAssetId {
    FrozenAssetId::from_sha256(Sha256::digest(bytes).into())
}

fn digest(bytes: &[u8]) -> String {
    let mut encoded = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        use std::fmt::Write as _;
        write!(encoded, "{byte:02x}").expect("writing to a string cannot fail");
    }
    encoded
}

fn limits(files: usize, bytes: u64) -> UnitRootLimits {
    UnitRootLimits::new(files, bytes).expect("the fixture limits are bounded")
}
