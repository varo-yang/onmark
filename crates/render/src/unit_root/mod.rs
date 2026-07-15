//! Private, verified filesystem root presented to one browser render unit.
//!
//! Ownership of the temporary directory keeps every returned path valid and
//! removes the complete root when execution ends.

mod error;
mod materializer;

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use onmark_core::model::{FrameIndex, FrozenAssetId};
use onmark_core::protocol::{BrowserPlan, BundleManifest};
use tempfile::TempDir;
use url::Url;

pub use error::{UnitRootError, UnitRootErrorKind};

use crate::encoder::AudioInput;
use crate::{AudioPlan, MaterializedAsset, RenderProfile, RenderUnit};

const MAX_FILES: usize = BundleManifest::MAX_FILES + 1;
const MAX_BYTES: u64 = 1 << 40;

/// One worker-local source for frozen bytes entering a private unit root.
///
/// Composition uses [`MaterializedAsset`] because it still needs metadata.
/// Materialization needs only an immutable identity and a local byte source,
/// which keeps the worker handoff from reconstructing probe facts it does not
/// consume.
#[derive(Clone, Debug)]
pub(crate) struct AssetSource {
    id: FrozenAssetId,
    local_path: PathBuf,
}

impl AssetSource {
    pub(crate) fn new(id: FrozenAssetId, local_path: impl Into<PathBuf>) -> Self {
        Self {
            id,
            local_path: local_path.into(),
        }
    }

    pub(crate) const fn id(&self) -> FrozenAssetId {
        self.id
    }

    pub(crate) fn local_path(&self) -> &Path {
        &self.local_path
    }

    pub(crate) fn unit_relative_path(&self) -> String {
        BundleManifest::asset_path(self.id)
    }
}

impl From<&MaterializedAsset> for AssetSource {
    fn from(asset: &MaterializedAsset) -> Self {
        Self::new(asset.id(), asset.local_path())
    }
}

/// Explicit retained-storage limits for one private execution root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnitRootLimits {
    max_files: usize,
    max_bytes: u64,
}

impl UnitRootLimits {
    /// Creates one bounded unit-root policy.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidUnitRootLimits`] when a bound is zero or exceeds the
    /// fixed local-render safety envelope.
    pub const fn new(max_files: usize, max_bytes: u64) -> Result<Self, InvalidUnitRootLimits> {
        if max_files == 0 {
            return Err(InvalidUnitRootLimits::ZeroFiles);
        }
        if max_files > MAX_FILES {
            return Err(InvalidUnitRootLimits::TooManyFiles);
        }
        if max_bytes == 0 {
            return Err(InvalidUnitRootLimits::ZeroBytes);
        }
        if max_bytes > MAX_BYTES {
            return Err(InvalidUnitRootLimits::TooManyBytes);
        }
        Ok(Self {
            max_files,
            max_bytes,
        })
    }

    const fn max_files(self) -> usize {
        self.max_files
    }

    const fn max_bytes(self) -> u64 {
        self.max_bytes
    }
}

/// Reason unit-root resource limits cannot bound local materialization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidUnitRootLimits {
    /// No retained file may be created.
    ZeroFiles,
    /// The requested file count exceeds the fixed safety ceiling.
    TooManyFiles,
    /// No retained payload bytes may be written.
    ZeroBytes,
    /// The requested byte budget exceeds one tebibyte.
    TooManyBytes,
}

impl fmt::Display for InvalidUnitRootLimits {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroFiles => "unit root file limit must be positive",
            Self::TooManyFiles => "unit root file limit exceeds the safety ceiling",
            Self::ZeroBytes => "unit root byte limit must be positive",
            Self::TooManyBytes => "unit root byte limit exceeds the safety ceiling",
        })
    }
}

impl Error for InvalidUnitRootLimits {}

/// One render unit whose bundle and assets occupy a verified private root.
#[derive(Debug)]
pub struct ExecutableUnit {
    browser_plan: BrowserPlan,
    bundle_id: Box<str>,
    profile: RenderProfile,
    audio: AudioPlan,
    root: UnitRoot,
}

impl ExecutableUnit {
    /// Consumes materialization requirements into browser-executable inputs.
    ///
    /// # Errors
    ///
    /// Returns [`UnitRootError`] when bundle or asset bytes, resource limits,
    /// or filesystem operations violate the unit contract.
    pub fn materialize(
        unit: RenderUnit,
        bundle_directory: &Path,
        limits: UnitRootLimits,
    ) -> Result<Self, UnitRootError> {
        let bundle_id = unit.bundle_manifest().bundle_id().into();
        let root = UnitRoot::materialize(
            bundle_directory,
            unit.bundle_manifest(),
            unit.materialized_assets(),
            limits,
        )?;
        let profile = unit.profile();
        let (browser_plan, audio) = unit.into_execution_plans();

        Ok(Self {
            browser_plan,
            bundle_id,
            profile,
            audio,
            root,
        })
    }

    pub(crate) fn from_worker_root(
        browser_plan: BrowserPlan,
        bundle_id: impl Into<Box<str>>,
        profile: RenderProfile,
        root: UnitRoot,
    ) -> Self {
        Self {
            browser_plan,
            bundle_id: bundle_id.into(),
            profile,
            audio: AudioPlan::empty(),
            root,
        }
    }

    /// Returns the browser-facing projection of the verified unit.
    #[must_use]
    pub const fn browser_plan(&self) -> &BrowserPlan {
        &self.browser_plan
    }

    /// Returns pixel-affecting output facts for the verified unit.
    #[must_use]
    pub const fn profile(&self) -> RenderProfile {
        self.profile
    }

    pub(crate) fn bundle_id(&self) -> &str {
        &self.bundle_id
    }

    /// Returns audio inputs relative to one published-output origin.
    ///
    /// The caller owns the artifact's frame-zero convention. A standalone
    /// unit uses its own output start; assembled units use their plan start.
    pub(crate) fn audio_inputs_rebased_to(
        &self,
        origin: FrameIndex,
    ) -> impl ExactSizeIterator<Item = AudioInput> + '_ {
        self.audio.tracks().map(move |track| {
            let start = rebase_audio_start(track.start(), origin);
            let source = self.root.path().join(track.asset().unit_relative_path());
            AudioInput::new(source, start)
        })
    }

    /// Returns the verified presentation entry loaded by Chromium.
    #[must_use]
    pub const fn entry_url(&self) -> &Url {
        self.root.entry_url()
    }
}

fn rebase_audio_start(start: FrameIndex, origin: FrameIndex) -> FrameIndex {
    start
        .get()
        .checked_sub(origin.get())
        .map(FrameIndex::new)
        .expect("a render unit retains only audio at or after its published-output origin")
}

#[cfg(test)]
mod tests {
    use onmark_core::model::FrameIndex;

    use super::rebase_audio_start;

    #[test]
    fn rebases_audio_to_the_output_origin() {
        assert_eq!(
            rebase_audio_start(FrameIndex::new(30), FrameIndex::new(30)),
            FrameIndex::new(0),
        );
        assert_eq!(
            rebase_audio_start(FrameIndex::new(45), FrameIndex::new(30)),
            FrameIndex::new(15),
        );
    }
}

/// Private verified filesystem root retained for one local render lifetime.
#[derive(Debug)]
pub struct UnitRoot {
    directory: TempDir,
    entry_url: Url,
}

impl UnitRoot {
    /// Materializes one presentation bundle and its frozen assets.
    ///
    /// Payloads are copied rather than linked so later source-path mutation
    /// cannot change bytes already admitted into this private execution root.
    ///
    /// # Errors
    ///
    /// Returns [`UnitRootError`] when identities, source files, resource
    /// limits, or filesystem operations violate the execution contract.
    pub fn materialize<'a>(
        bundle_directory: &Path,
        manifest: &BundleManifest,
        assets: impl IntoIterator<Item = &'a MaterializedAsset>,
        limits: UnitRootLimits,
    ) -> Result<Self, UnitRootError> {
        Self::materialize_sources(
            bundle_directory,
            manifest,
            assets.into_iter().map(AssetSource::from),
            limits,
        )
    }

    pub(crate) fn materialize_sources(
        bundle_directory: &Path,
        manifest: &BundleManifest,
        assets: impl IntoIterator<Item = AssetSource>,
        limits: UnitRootLimits,
    ) -> Result<Self, UnitRootError> {
        Self::materialize_sources_at(None, bundle_directory, manifest, assets, limits)
    }

    pub(crate) fn materialize_sources_in(
        private_root_parent: &Path,
        bundle_directory: &Path,
        manifest: &BundleManifest,
        assets: impl IntoIterator<Item = AssetSource>,
        limits: UnitRootLimits,
    ) -> Result<Self, UnitRootError> {
        Self::materialize_sources_at(
            Some(private_root_parent),
            bundle_directory,
            manifest,
            assets,
            limits,
        )
    }

    fn materialize_sources_at(
        private_root_parent: Option<&Path>,
        bundle_directory: &Path,
        manifest: &BundleManifest,
        assets: impl IntoIterator<Item = AssetSource>,
        limits: UnitRootLimits,
    ) -> Result<Self, UnitRootError> {
        let directory = materializer::materialize(
            private_root_parent,
            bundle_directory,
            manifest,
            assets.into_iter(),
            limits,
        )?;
        let entry = directory.path().join(manifest.entry_point());
        let entry_url = Url::from_file_path(&entry).map_err(|()| {
            UnitRootError::without_source(
                UnitRootErrorKind::InvalidEntry,
                &entry,
                "unit-root entry cannot be represented as a file URL",
            )
        })?;

        Ok(Self {
            directory,
            entry_url,
        })
    }

    /// Returns the owned private filesystem root.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.directory.path()
    }

    /// Returns the browser URL of the verified presentation entry.
    #[must_use]
    pub const fn entry_url(&self) -> &Url {
        &self.entry_url
    }
}
