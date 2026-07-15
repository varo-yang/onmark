use std::collections::BTreeSet;
use std::path::Path;

use onmark_core::model::FrozenAssetId;
use onmark_core::protocol::{BrowserPlan, BrowserVideo, BundleManifest};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::unit_root::AssetSource;
use crate::{
    CaptureEnvironmentId, ExecutableUnit, RenderProfile, UnitRoot, UnitRootError, UnitRootLimits,
};

const BUNDLE_DIRECTORY: &str = "bundle";

/// Version of the worker capture-request contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct WorkerCaptureVersion(u16);

impl WorkerCaptureVersion {
    /// Capture request with an explicit locked-environment identity.
    pub const V2: Self = Self(2);

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
        if version == Self::V2.get() {
            return Ok(Self::V2);
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
}

impl WorkerCaptureRequest {
    /// Creates one portable request for one already-composed render unit.
    #[must_use]
    pub fn new(
        capture_environment: CaptureEnvironmentId,
        bundle: BundleManifest,
        browser_plan: BrowserPlan,
        profile: RenderProfile,
    ) -> Self {
        Self {
            version: WorkerCaptureVersion::V2,
            capture_environment,
            bundle,
            browser_plan,
            profile,
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
        &self,
        input_root: &Path,
        limits: UnitRootLimits,
    ) -> Result<ExecutableUnit, UnitRootError> {
        let assets = self
            .asset_ids()
            .into_iter()
            .map(|id| AssetSource::new(id, input_root.join(BundleManifest::asset_path(id))));
        let root = UnitRoot::materialize_sources(
            &input_root.join(BUNDLE_DIRECTORY),
            &self.bundle,
            assets,
            limits,
        )?;

        Ok(ExecutableUnit::from_worker_root(
            self.browser_plan.clone(),
            self.bundle.bundle_id(),
            self.profile,
            root,
        ))
    }

    fn asset_ids(&self) -> BTreeSet<FrozenAssetId> {
        self.browser_plan
            .videos()
            .iter()
            .map(BrowserVideo::asset_identity)
            .collect()
    }
}

impl<'de> Deserialize<'de> for WorkerCaptureRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WorkerCaptureRequestWire::deserialize(deserializer)?;
        Ok(Self::new(
            wire.capture_environment,
            wire.bundle,
            wire.browser_plan,
            wire.profile,
        ))
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
}

#[cfg(test)]
mod tests {
    use super::WorkerCaptureVersion;

    #[test]
    fn rejects_the_pre_environment_request_version() {
        assert!(serde_json::from_str::<WorkerCaptureVersion>("1").is_err());
        assert_eq!(
            serde_json::from_str::<WorkerCaptureVersion>("2")
                .expect("the environment-bound version parses"),
            WorkerCaptureVersion::V2,
        );
    }
}
