//! Typed construction of the media-bearing Gate-three conformance film.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use onmark_core::compiler;
use onmark_core::diagnostics::Diagnostics;
use onmark_core::model::{
    AssetRef, FrameRate, FrozenAsset, FrozenAssetId, PresentationTemporalCapability, SourceId,
    Timebase,
};
use onmark_core::protocol::BundleManifest;
use onmark_core::render_graph::{PartitionPlan, RenderGraph};
use onmark_core::timeline::TimelineIr;
use onmark_media::Ffprobe;
use onmark_render::{
    BrowserLimits, CaptureEnvironmentId, EncodeLimits, ExecutableUnit, Ffmpeg, FrameArtifact,
    MaterializedAsset, RenderExecutor, RenderProfile, RenderUnit, UnitRootLimits,
    WorkerCaptureRequest,
};
use sha2::{Digest as _, Sha256};

use super::aws::RemoteEnvironment;
use super::media;

const SOURCE: &str = include_str!("../../../../conformance/cli/gate-two.onmark");
const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const FRAME_RATE: u32 = 30;
const EXPECTED_FRAMES: u64 = 60;
const PROCESS_TIMEOUT: Duration = Duration::from_mins(3);
const UNIT_FILES: usize = 16;
const UNIT_BYTES: u64 = 64 * 1024 * 1024;

pub(super) struct ConformanceFilm {
    bundle: PathBuf,
    assets: BTreeMap<FrozenAssetId, PathBuf>,
    partitions: PartitionPlan,
    partition_units: Vec<RenderUnit>,
    whole_unit: RenderUnit,
}

impl ConformanceFilm {
    pub(super) async fn build(workspace: &Path, environment: &RemoteEnvironment) -> Self {
        let media = media::generate(workspace, environment).await;
        let probe = Ffprobe::new(
            environment.ffprobe(),
            PROCESS_TIMEOUT,
            Ffprobe::MAX_OUTPUT_BYTES,
        )
        .expect("the conformance probe limits are bounded");
        let catalog = freeze_catalog(&media, &probe);
        let timeline = solve(&catalog.facts);
        let partitions =
            RenderGraph::from_timeline(&timeline, PresentationTemporalCapability::RandomAccess)
                .expect("the solved fixture has complete render ownership")
                .into_partition();
        assert_eq!(partitions.units().len(), 2);

        let bundle = bundle_directory();
        let manifest = bundle_manifest(&bundle);
        let profile = RenderProfile::new(WIDTH, HEIGHT).expect("the fixture profile is valid");
        let materialized = materialized_assets(&catalog);
        let partition_units =
            render_units(&timeline, &partitions, &manifest, profile, &materialized);
        let whole_unit = RenderUnit::whole_film(&timeline, manifest, profile, materialized)
            .expect("the complete fixture forms one render unit");

        Self {
            bundle,
            assets: catalog.paths,
            partitions,
            partition_units,
            whole_unit,
        }
    }

    pub(super) fn capture_cases(&self, capture_environment: CaptureEnvironmentId) -> CaptureCases {
        let [first, second] = self.partition_units.as_slice() else {
            panic!("the conformance film must contain exactly two partitions");
        };
        let whole = CaptureCase::stage(
            "whole",
            self.whole_unit.worker_capture_request(capture_environment),
            &self.bundle,
            &self.assets,
        );
        let partitions = [
            CaptureCase::stage(
                "partition-0",
                first.worker_capture_request(capture_environment),
                &self.bundle,
                &self.assets,
            ),
            CaptureCase::stage(
                "partition-1",
                second.worker_capture_request(capture_environment),
                &self.bundle,
                &self.assets,
            ),
        ];

        CaptureCases { whole, partitions }
    }

    pub(super) async fn assemble(
        &self,
        artifacts: &[FrameArtifact],
        environment: &RemoteEnvironment,
        output: &Path,
    ) {
        let units: Vec<_> = self
            .partition_units
            .iter()
            .cloned()
            .map(|unit| {
                ExecutableUnit::materialize(unit, &self.bundle, unit_root_limits())
                    .expect("the assembler materializes each verified partition")
            })
            .collect();
        let browser_limits = BrowserLimits::new(PROCESS_TIMEOUT, 8 * 1024 * 1024)
            .expect("the unused browser boundary remains bounded");
        let encode_limits =
            EncodeLimits::new(PROCESS_TIMEOUT, EXPECTED_FRAMES, UNIT_BYTES, 64 * 1024)
                .expect("the conformance encoder limits are bounded");
        let ffmpeg = Ffmpeg::new(environment.ffmpeg(), encode_limits)
            .expect("the conformance FFmpeg path is non-empty");
        let executor = RenderExecutor::new("unused-during-assembly", browser_limits, ffmpeg);

        let video = executor
            .assemble_frame_artifacts(
                &self.partitions,
                &units,
                artifacts,
                environment.capture_environment(),
                output,
            )
            .await
            .expect("remote artifacts assemble through the shared encoder and audio path");
        assert_eq!(video.frames(), EXPECTED_FRAMES);
    }
}

pub(super) struct CaptureCases {
    whole: CaptureCase,
    partitions: [CaptureCase; 2],
}

impl CaptureCases {
    pub(super) fn into_parts(self) -> (CaptureCase, [CaptureCase; 2]) {
        (self.whole, self.partitions)
    }
}

pub(super) struct CaptureCase {
    name: Box<str>,
    root: tempfile::TempDir,
    request: WorkerCaptureRequest,
    files: Vec<PathBuf>,
}

impl CaptureCase {
    fn stage(
        name: &str,
        request: WorkerCaptureRequest,
        bundle_source: &Path,
        assets: &BTreeMap<FrozenAssetId, PathBuf>,
    ) -> Self {
        let root = tempfile::Builder::new()
            .prefix("onmark-remote-input-")
            .tempdir()
            .expect("the private worker input is writable");
        let mut files = vec![PathBuf::from(WorkerCaptureRequest::FILE_NAME)];
        fs::write(
            root.path().join(WorkerCaptureRequest::FILE_NAME),
            serde_json::to_vec(&request).expect("the worker request serializes"),
        )
        .expect("the worker request is writable");

        for file in request.bundle().files() {
            let relative = Path::new(WorkerCaptureRequest::BUNDLE_DIRECTORY).join(file.path());
            copy_file(
                &bundle_source.join(file.path()),
                &root.path().join(&relative),
            );
            files.push(relative);
        }
        for id in request.required_asset_ids() {
            let relative = PathBuf::from(BundleManifest::asset_path(id));
            copy_file(
                assets
                    .get(&id)
                    .expect("every requested visual asset retains its exact bytes"),
                &root.path().join(&relative),
            );
            files.push(relative);
        }
        files.sort();

        Self {
            name: name.into(),
            root,
            request,
            files,
        }
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn root(&self) -> &Path {
        self.root.path()
    }

    pub(super) const fn request(&self) -> &WorkerCaptureRequest {
        &self.request
    }

    pub(super) fn frames(&self) -> u64 {
        let output = self.request.browser_plan().output();
        output
            .end()
            .get()
            .checked_sub(output.start().get())
            .expect("a worker request has an ordered output interval")
    }

    pub(super) fn files(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(PathBuf::as_path)
    }
}

struct FrozenCatalog {
    facts: BTreeMap<AssetRef, FrozenAsset>,
    paths: BTreeMap<FrozenAssetId, PathBuf>,
}

fn materialized_assets(catalog: &FrozenCatalog) -> Vec<MaterializedAsset> {
    catalog
        .facts
        .values()
        .map(|frozen| {
            let path = catalog
                .paths
                .get(&frozen.id())
                .expect("every frozen fact retains its exact bytes");
            MaterializedAsset::new(frozen.clone(), path)
                .expect("the fixture asset path is non-empty")
        })
        .collect()
}

fn render_units(
    timeline: &TimelineIr,
    partitions: &PartitionPlan,
    manifest: &BundleManifest,
    profile: RenderProfile,
    assets: &[MaterializedAsset],
) -> Vec<RenderUnit> {
    partitions
        .units()
        .iter()
        .map(|partition| {
            RenderUnit::from_partition(
                timeline,
                partition,
                manifest.clone(),
                profile,
                assets.to_vec(),
            )
            .expect("each graph partition forms one render unit")
        })
        .collect()
}

fn freeze_catalog(media: &media::GeneratedMedia, probe: &Ffprobe) -> FrozenCatalog {
    let mut facts = BTreeMap::new();
    let mut paths = BTreeMap::new();
    for (reference, path) in [
        ("source.mp4", media.video()),
        ("voice.m4a", media.voice_over()),
    ] {
        let id = digest_file(path);
        let metadata = probe
            .probe(path)
            .unwrap_or_else(|error| panic!("failed to probe {}: {error}", path.display()));
        let frozen = FrozenAsset::new(id, metadata);
        facts.insert(
            AssetRef::parse(reference).expect("the fixture reference is canonical"),
            frozen,
        );
        paths.insert(id, path.to_owned());
    }
    FrozenCatalog { facts, paths }
}

fn solve(facts: &BTreeMap<AssetRef, FrozenAsset>) -> TimelineIr {
    let (document, diagnostics) = compiler::parse(SourceId::new(0), SOURCE).into_parts();
    require_clean(diagnostics);
    let (film, diagnostics) = compiler::bind(document).into_parts();
    require_clean(diagnostics);
    let (film, diagnostics) =
        compiler::resolve(film.expect("the conformance film binds")).into_parts();
    require_clean(diagnostics);
    let report = compiler::solve(
        film.expect("the conformance film resolves"),
        facts,
        Timebase::new(FrameRate::new(FRAME_RATE, 1).expect("the frame rate is valid")),
    )
    .expect("the conformance assets satisfy the solver");
    let (timeline, diagnostics) = report.into_parts();
    require_clean(diagnostics);
    timeline.expect("the conformance film solves")
}

fn require_clean(diagnostics: Diagnostics) {
    assert!(
        diagnostics.is_empty(),
        "the conformance fixture produced {} diagnostics",
        diagnostics.len(),
    );
}

fn digest_file(path: &Path) -> FrozenAssetId {
    let bytes = fs::read(path).expect("the bounded fixture asset is readable");
    FrozenAssetId::from_sha256(Sha256::digest(bytes).into())
}

fn bundle_manifest(directory: &Path) -> BundleManifest {
    let contents = fs::read(directory.join("manifest.json"))
        .expect("the checked-in bundle manifest is readable");
    serde_json::from_slice(&contents).expect("the checked-in bundle manifest is valid")
}

fn bundle_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("conformance/protocol/bundle-v1")
}

fn unit_root_limits() -> UnitRootLimits {
    UnitRootLimits::new(UNIT_FILES, UNIT_BYTES).expect("the conformance unit limits are bounded")
}

fn copy_file(source: &Path, destination: &Path) {
    let parent = destination
        .parent()
        .expect("a portable worker path always has a parent");
    fs::create_dir_all(parent).expect("the worker input directory is writable");
    fs::copy(source, destination).unwrap_or_else(|error| {
        panic!(
            "failed to copy {} to {}: {error}",
            source.display(),
            destination.display(),
        )
    });
}
