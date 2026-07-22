//! Portable, versioned request for capture of one already-planned render unit.
//!
//! The worker receives solved browser facts and frozen bytes; it never parses
//! source, probes assets, or replans partitions.

use std::collections::BTreeSet;
use std::path::Path;

use onmark_core::model::FrozenAssetId;
use onmark_core::protocol::{BrowserPlan, BrowserVideo, BundleManifest};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::unit_root::AssetSource;
use crate::{
    CaptureEnvironmentId, ExecutableUnit, FrameArtifactId, RenderProfile, UnitRoot, UnitRootError,
    UnitRootLimits, VisualExecutionPlan,
};

/// Version of the worker capture-request contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct WorkerCaptureVersion(u16);

impl WorkerCaptureVersion {
    /// Only worker capture-request version accepted by this build.
    pub const CURRENT: Self = Self(1);

    /// Returns the stable integer representation.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl<'de> Deserialize<'de> for WorkerCaptureVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let version = u16::deserialize(deserializer)?;
        if version == Self::CURRENT.get() {
            return Ok(Self::CURRENT);
        }
        Err(D::Error::custom(
            "unsupported worker capture request version",
        ))
    }
}

/// Immutable visual work handed from one composition process to one worker.
///
/// The request contains solved browser facts and the deployment identity under
/// which their pixels may be reused. It deliberately has no screenplay, source
/// path, probe metadata, or audio mix: a capture worker must not recompile
/// authored input, and final audio stays with the assembler.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WorkerCaptureRequest {
    version: WorkerCaptureVersion,
    capture_environment: CaptureEnvironmentId,
    bundle: BundleManifest,
    browser_plan: BrowserPlan,
    profile: RenderProfile,
    visual_execution: VisualExecutionPlan,
}

impl WorkerCaptureRequest {
    /// Fixed request filename beneath every portable worker-input root.
    pub const FILE_NAME: &'static str = "request.json";
    /// Fixed directory containing immutable presentation payload files.
    pub const BUNDLE_DIRECTORY: &'static str = "bundle";
    /// Maximum retained JSON bytes accepted at a worker request boundary.
    pub const MAX_JSON_BYTES: u64 = 16 * 1024 * 1024;

    pub(crate) fn new(
        capture_environment: CaptureEnvironmentId,
        bundle: BundleManifest,
        browser_plan: BrowserPlan,
        profile: RenderProfile,
        visual_execution: VisualExecutionPlan,
    ) -> Self {
        Self {
            version: WorkerCaptureVersion::CURRENT,
            capture_environment,
            bundle,
            browser_plan,
            profile,
            visual_execution,
        }
    }

    /// Returns the request contract version.
    #[must_use]
    pub const fn version(&self) -> WorkerCaptureVersion {
        self.version
    }

    /// Returns the locked environment required to reuse captured pixels.
    #[must_use]
    pub const fn capture_environment(&self) -> CaptureEnvironmentId {
        self.capture_environment
    }

    /// Returns the immutable presentation bundle needed by this worker.
    #[must_use]
    pub const fn bundle(&self) -> &BundleManifest {
        &self.bundle
    }

    /// Returns browser facts the worker must execute exactly once.
    #[must_use]
    pub const fn browser_plan(&self) -> &BrowserPlan {
        &self.browser_plan
    }

    /// Returns the pixel-affecting output facts for this worker.
    #[must_use]
    pub const fn profile(&self) -> RenderProfile {
        self.profile
    }

    /// Returns the admitted browser/native visual path.
    #[must_use]
    pub const fn visual_execution(&self) -> &VisualExecutionPlan {
        &self.visual_execution
    }

    /// Returns the deployment-independent address of this capture result.
    ///
    /// The address commits to the solved browser plan, immutable bundle,
    /// render profile, and locked capture environment. Storage location stays
    /// deployment-owned, so this request can move between local and remote
    /// workers without changing its identity.
    #[must_use]
    pub fn artifact_id(&self) -> FrameArtifactId {
        FrameArtifactId::from_facts(
            &self.browser_plan,
            self.bundle.bundle_id(),
            self.profile,
            self.capture_environment,
        )
    }

    /// Returns required frozen visual assets in deterministic identity order.
    ///
    /// A remote adapter uses these identities to materialize the exact bytes
    /// named by the browser plan before handing this request to the renderer.
    #[must_use]
    pub fn required_asset_ids(&self) -> impl ExactSizeIterator<Item = FrozenAssetId> {
        asset_ids(&self.browser_plan).into_iter()
    }

    /// Materializes verified worker-local inputs from the portable layout.
    ///
    /// `input_root/bundle` must contain the exact bundle payload described by
    /// this request. Frozen browser assets must appear beneath `input_root` at
    /// their [`BundleManifest::asset_path`] locations. The request never
    /// contains host paths, so another process can resolve the same immutable
    /// facts through a different local cache or object-store download.
    ///
    /// # Errors
    ///
    /// Returns [`UnitRootError`] when bundle or asset bytes, limits, or local
    /// filesystem operations violate the verified private-root contract.
    pub fn materialize(
        self,
        input_root: &Path,
        limits: UnitRootLimits,
    ) -> Result<ExecutableUnit, UnitRootError> {
        self.materialize_with_root_parent(input_root, None, limits)
    }

    /// Materializes the portable layout into a caller-owned private parent.
    ///
    /// Deployment adapters use this form to keep downloaded inputs, copied
    /// unit bytes, and captured output under one audited scratch-disk policy.
    ///
    /// # Errors
    ///
    /// Returns [`UnitRootError`] when bundle or asset bytes, resource limits,
    /// or filesystem operations violate the verified private-root contract.
    pub fn materialize_in(
        self,
        input_root: &Path,
        private_root_parent: &Path,
        limits: UnitRootLimits,
    ) -> Result<ExecutableUnit, UnitRootError> {
        self.materialize_with_root_parent(input_root, Some(private_root_parent), limits)
    }

    fn materialize_with_root_parent(
        self,
        input_root: &Path,
        private_root_parent: Option<&Path>,
        limits: UnitRootLimits,
    ) -> Result<ExecutableUnit, UnitRootError> {
        let Self {
            bundle: manifest,
            browser_plan,
            profile,
            visual_execution,
            ..
        } = self;
        let assets = asset_ids(&browser_plan)
            .into_iter()
            .map(|id| AssetSource::new(id, input_root.join(BundleManifest::asset_path(id))));
        let bundle_directory = input_root.join(Self::BUNDLE_DIRECTORY);
        let root = match private_root_parent {
            Some(parent) => UnitRoot::materialize_sources_in(
                parent,
                &bundle_directory,
                &manifest,
                assets,
                limits,
            )?,
            None => UnitRoot::materialize_sources(&bundle_directory, &manifest, assets, limits)?,
        };

        Ok(ExecutableUnit::from_worker_root(
            browser_plan,
            manifest.bundle_id(),
            profile,
            visual_execution,
            root,
        ))
    }
}

fn asset_ids(plan: &BrowserPlan) -> BTreeSet<FrozenAssetId> {
    plan.videos()
        .iter()
        .map(BrowserVideo::asset_identity)
        .collect()
}

impl<'de> Deserialize<'de> for WorkerCaptureRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WorkerCaptureRequestWire::deserialize(deserializer)?;
        let request = Self::new(
            wire.capture_environment,
            wire.bundle,
            wire.browser_plan,
            wire.profile,
            wire.visual_execution,
        );
        request
            .visual_execution
            .validate(
                request.bundle.visual_capability(),
                &request.browser_plan,
                request.profile,
            )
            .map_err(D::Error::custom)?;
        Ok(request)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WorkerCaptureRequestWire {
    #[serde(rename = "version")]
    _version: WorkerCaptureVersion,
    capture_environment: CaptureEnvironmentId,
    bundle: BundleManifest,
    browser_plan: BrowserPlan,
    profile: RenderProfile,
    visual_execution: VisualExecutionPlan,
}

#[cfg(test)]
mod tests {
    use super::WorkerCaptureVersion;

    #[test]
    fn accepts_only_the_current_request_version() {
        assert!(serde_json::from_str::<WorkerCaptureVersion>("2").is_err());
        assert_eq!(
            serde_json::from_str::<WorkerCaptureVersion>("1")
                .expect("the current worker request version parses"),
            WorkerCaptureVersion::CURRENT,
        );
    }
}
