//! Bounded desktop reuse of verified, contract-addressed frame artifacts.
//!
//! The release launcher supplies host facts; this module adds native capture
//! policy, owns cache locking, and keeps cache hits and misses on one assembler
//! path. A full cache remains correct by declining new publications.

use std::env;
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use onmark_render::{
    BrowserCaptureMode, BrowserGraphicsBackend, CaptureEnvironmentId, ExecutableUnit,
    FrameArtifact, FrameArtifactError, FrameArtifactId, FrameArtifactLimits,
    InvalidCaptureEnvironmentId, RenderError, RenderExecutor,
};
use sha2::{Digest as _, Sha256};
use tempfile::TempDir;
use tokio::task::JoinError;

const CACHE_DIRECTORY: &str = "ONMARK_FRAME_CACHE";
const ENVIRONMENT_SEED: &str = "ONMARK_CAPTURE_ENVIRONMENT_SEED";
const IDENTITY_DOMAIN: &[u8] = b"onmark-local-capture-environment-v1\0";
const CACHE_LOCK: &str = ".cache.lock";
const ARTIFACT_EXTENSION: &str = "onmark-frames";
const MAX_CACHE_ARTIFACTS: usize = 10_000;
const MAX_CACHE_BYTES: u64 = 32 * 1024 * 1024 * 1024;

/// One captured sequence whose private misses live through final assembly.
pub(super) struct CapturedArtifacts {
    artifacts: Vec<FrameArtifact>,
    reuse: ArtifactReuse,
    _staging: TempDir,
}

impl CapturedArtifacts {
    pub(super) fn as_slice(&self) -> &[FrameArtifact] {
        &self.artifacts
    }

    pub(super) const fn reuse(&self) -> ArtifactReuse {
        self.reuse
    }
}

/// Verified cache work completed before any browser capture begins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ArtifactReuse {
    regions: usize,
    reused_regions: usize,
    reused_frames: u64,
}

/// Whether launcher-owned host identity permits cross-process artifact reuse.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CacheAdmission {
    Persistent,
    Ephemeral,
}

impl ArtifactReuse {
    fn from_hits(artifacts: &[Option<FrameArtifact>]) -> Result<Self, ArtifactCacheError> {
        let mut reused_regions = 0;
        let mut reused_frames = 0_u64;

        for artifact in artifacts.iter().flatten() {
            reused_regions += 1;
            reused_frames = reused_frames
                .checked_add(artifact.frames())
                .ok_or(ArtifactCacheError::FrameAccounting)?;
        }

        Ok(Self {
            regions: artifacts.len(),
            reused_regions,
            reused_frames,
        })
    }

    pub(super) const fn regions(self) -> usize {
        self.regions
    }

    pub(super) const fn reused_regions(self) -> usize {
        self.reused_regions
    }

    pub(super) const fn reused_frames(self) -> u64 {
        self.reused_frames
    }
}

/// Local artifact store selected before any browser work begins.
pub(super) struct ArtifactCache {
    directory: Option<PathBuf>,
    environment: CaptureEnvironmentId,
}

impl ArtifactCache {
    pub(super) fn from_environment(
        admission: CacheAdmission,
        capture_mode: BrowserCaptureMode,
        graphics_backend: BrowserGraphicsBackend,
    ) -> Result<Self, ArtifactCacheError> {
        if admission == CacheAdmission::Ephemeral {
            return Ok(Self {
                directory: None,
                environment: ephemeral_environment(capture_mode, graphics_backend),
            });
        }
        match (env::var_os(CACHE_DIRECTORY), env::var_os(ENVIRONMENT_SEED)) {
            (Some(directory), Some(seed)) => {
                let seed = seed
                    .into_string()
                    .map_err(|_| ArtifactCacheError::NonUtf8EnvironmentSeed)?;
                let seed = CaptureEnvironmentId::parse(&seed)
                    .map_err(ArtifactCacheError::InvalidEnvironmentSeed)?;
                Ok(Self {
                    directory: Some(PathBuf::from(directory)),
                    environment: capture_environment(
                        seed.as_sha256(),
                        capture_mode,
                        graphics_backend,
                    ),
                })
            }
            (None, None) => Ok(Self {
                directory: None,
                environment: ephemeral_environment(capture_mode, graphics_backend),
            }),
            _ => Err(ArtifactCacheError::IncompleteEnvironment),
        }
    }

    pub(super) const fn environment(&self) -> CaptureEnvironmentId {
        self.environment
    }

    pub(super) async fn capture(
        &self,
        executor: &RenderExecutor,
        units: &[ExecutableUnit],
        limits: FrameArtifactLimits,
    ) -> Result<CapturedArtifacts, ArtifactCacheError> {
        let staging = self.staging_directory().await?;
        let mut artifacts = match &self.directory {
            Some(directory) => self.cache_hits(directory, units, limits).await?,
            None => empty_artifacts(units.len()),
        };
        let reuse = ArtifactReuse::from_hits(&artifacts)?;
        let misses = missing_units(units, &artifacts, staging.path(), self.environment);
        let references = misses
            .iter()
            .map(|miss| &units[miss.index])
            .collect::<Vec<_>>();
        let destinations = misses
            .iter()
            .map(|miss| miss.path.clone())
            .collect::<Vec<_>>();
        let captured = executor
            .capture_frame_artifacts(&references, self.environment, &destinations, limits)
            .await
            .map_err(ArtifactCacheError::Render)?;

        let captured = match &self.directory {
            Some(directory) => {
                self.publish(directory, units, misses, captured, limits)
                    .await?
            }
            None => misses
                .into_iter()
                .zip(captured)
                .map(|(miss, artifact)| (miss.index, artifact))
                .collect(),
        };
        for (index, artifact) in captured {
            artifacts[index] = Some(artifact);
        }

        Ok(CapturedArtifacts {
            artifacts: artifacts
                .into_iter()
                .map(|artifact| artifact.expect("every cache miss is filled by the capture batch"))
                .collect(),
            reuse,
            _staging: staging,
        })
    }

    async fn staging_directory(&self) -> Result<TempDir, ArtifactCacheError> {
        match &self.directory {
            Some(directory) => {
                tokio::fs::create_dir_all(directory)
                    .await
                    .map_err(|source| ArtifactCacheError::Directory {
                        path: directory.clone(),
                        source,
                    })?;
                tempfile::Builder::new()
                    .prefix(".capture-")
                    .tempdir_in(directory)
                    .map_err(ArtifactCacheError::Staging)
            }
            None => tempfile::tempdir().map_err(ArtifactCacheError::Staging),
        }
    }

    async fn cache_hits(
        &self,
        directory: &Path,
        units: &[ExecutableUnit],
        limits: FrameArtifactLimits,
    ) -> Result<Vec<Option<FrameArtifact>>, ArtifactCacheError> {
        let mut artifacts = Vec::with_capacity(units.len());
        for unit in units {
            let id = unit.frame_artifact_id(self.environment);
            artifacts.push(load_cache_entry(directory, id, limits).await?);
        }
        Ok(artifacts)
    }

    async fn publish(
        &self,
        directory: &Path,
        units: &[ExecutableUnit],
        misses: Vec<MissingArtifact>,
        captured: Vec<FrameArtifact>,
        limits: FrameArtifactLimits,
    ) -> Result<Vec<(usize, FrameArtifact)>, ArtifactCacheError> {
        let _lease = CacheLease::acquire(directory).await?;
        let mut usage = cache_usage(directory)?;
        let mut published = Vec::with_capacity(captured.len());
        for (miss, captured) in misses.into_iter().zip(captured) {
            let id = units[miss.index].frame_artifact_id(self.environment);
            let path = artifact_path(directory, id);
            match inspect_cache_entry(&path, id, limits).await? {
                CacheEntry::Valid(existing) => {
                    published.push((miss.index, existing));
                    continue;
                }
                CacheEntry::Corrupt => usage.remove(remove_corrupt(&path)?),
                CacheEntry::Missing => {}
            }

            let bytes = captured
                .path()
                .metadata()
                .map_err(|source| ArtifactCacheError::Inspect {
                    path: captured.path().to_owned(),
                    source,
                })?
                .len();
            if !usage.admits(bytes) {
                published.push((miss.index, captured));
                continue;
            }

            fs::hard_link(captured.path(), &path).map_err(|source| {
                ArtifactCacheError::Publish {
                    path: path.clone(),
                    source,
                }
            })?;
            let artifact = FrameArtifact::open(path, limits)
                .await
                .map_err(ArtifactCacheError::Artifact)?;
            usage.add(bytes);
            published.push((miss.index, artifact));
        }
        Ok(published)
    }
}

struct MissingArtifact {
    index: usize,
    path: PathBuf,
}

fn missing_units(
    units: &[ExecutableUnit],
    artifacts: &[Option<FrameArtifact>],
    staging: &Path,
    environment: CaptureEnvironmentId,
) -> Vec<MissingArtifact> {
    let mut misses = Vec::new();
    for (index, (unit, artifact)) in units.iter().zip(artifacts).enumerate() {
        if artifact.is_none() {
            misses.push(MissingArtifact {
                index,
                path: artifact_path(staging, unit.frame_artifact_id(environment)),
            });
        }
    }
    misses
}

fn empty_artifacts(length: usize) -> Vec<Option<FrameArtifact>> {
    std::iter::repeat_with(|| None).take(length).collect()
}

async fn load_cache_entry(
    directory: &Path,
    expected: FrameArtifactId,
    limits: FrameArtifactLimits,
) -> Result<Option<FrameArtifact>, ArtifactCacheError> {
    let path = artifact_path(directory, expected);
    match inspect_cache_entry(&path, expected, limits).await? {
        CacheEntry::Missing => Ok(None),
        CacheEntry::Valid(artifact) => Ok(Some(artifact)),
        CacheEntry::Corrupt => {
            let _lease = CacheLease::acquire(directory).await?;
            repair_cache_entry(directory, expected, limits).await
        }
    }
}

async fn repair_cache_entry(
    directory: &Path,
    expected: FrameArtifactId,
    limits: FrameArtifactLimits,
) -> Result<Option<FrameArtifact>, ArtifactCacheError> {
    let path = artifact_path(directory, expected);
    match inspect_cache_entry(&path, expected, limits).await? {
        CacheEntry::Missing => Ok(None),
        CacheEntry::Valid(artifact) => Ok(Some(artifact)),
        CacheEntry::Corrupt => {
            remove_corrupt(&path)?;
            Ok(None)
        }
    }
}

enum CacheEntry {
    Missing,
    Valid(FrameArtifact),
    Corrupt,
}

async fn inspect_cache_entry(
    path: &Path,
    expected: FrameArtifactId,
    limits: FrameArtifactLimits,
) -> Result<CacheEntry, ArtifactCacheError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(CacheEntry::Missing);
        }
        Err(source) => {
            return Err(ArtifactCacheError::Inspect {
                path: path.to_owned(),
                source,
            });
        }
    };
    if !metadata.is_file() {
        return Ok(CacheEntry::Corrupt);
    }

    let artifact = match FrameArtifact::open(path, limits).await {
        Ok(artifact) if artifact.id() == expected => artifact,
        Ok(_) | Err(_) => return Ok(CacheEntry::Corrupt),
    };
    if artifact.verify().await.is_err() {
        return Ok(CacheEntry::Corrupt);
    }
    Ok(CacheEntry::Valid(artifact))
}

fn remove_corrupt(path: &Path) -> Result<CacheUsage, ArtifactCacheError> {
    let metadata = path
        .symlink_metadata()
        .map_err(|source| ArtifactCacheError::Inspect {
            path: path.to_owned(),
            source,
        })?;
    let removed = CacheUsage::from_metadata(&metadata);
    let result = if metadata.is_dir() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|source| ArtifactCacheError::Remove {
        path: path.to_owned(),
        source,
    })?;
    Ok(removed)
}

fn artifact_path(directory: &Path, id: FrameArtifactId) -> PathBuf {
    let mut name = String::with_capacity(64 + ARTIFACT_EXTENSION.len() + 1);
    for byte in id.as_sha256() {
        use std::fmt::Write as _;
        write!(name, "{byte:02x}").expect("writing into a String cannot fail");
    }
    name.push('.');
    name.push_str(ARTIFACT_EXTENSION);
    directory.join(name)
}

#[derive(Clone, Copy, Default)]
struct CacheUsage {
    artifacts: usize,
    bytes: u64,
}

impl CacheUsage {
    const fn one_artifact(bytes: u64) -> Self {
        Self {
            artifacts: 1,
            bytes,
        }
    }

    fn from_metadata(metadata: &fs::Metadata) -> Self {
        if metadata.is_file() {
            return Self::one_artifact(metadata.len());
        }
        Self::default()
    }

    fn admits(self, bytes: u64) -> bool {
        self.artifacts < MAX_CACHE_ARTIFACTS && bytes <= MAX_CACHE_BYTES.saturating_sub(self.bytes)
    }

    fn add(&mut self, bytes: u64) {
        self.artifacts = self.artifacts.saturating_add(1);
        self.bytes = self.bytes.saturating_add(bytes);
    }

    fn remove(&mut self, removed: Self) {
        self.artifacts = self.artifacts.saturating_sub(removed.artifacts);
        self.bytes = self.bytes.saturating_sub(removed.bytes);
    }
}

fn cache_usage(directory: &Path) -> Result<CacheUsage, ArtifactCacheError> {
    let mut usage = CacheUsage {
        artifacts: 0,
        bytes: 0,
    };
    let entries = fs::read_dir(directory).map_err(|source| ArtifactCacheError::Inspect {
        path: directory.to_owned(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| ArtifactCacheError::Inspect {
            path: directory.to_owned(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some(ARTIFACT_EXTENSION) {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|source| ArtifactCacheError::Inspect {
                path: path.clone(),
                source,
            })?;
        if !metadata.is_file() {
            continue;
        }
        usage.artifacts = usage.artifacts.saturating_add(1);
        usage.bytes = usage.bytes.saturating_add(metadata.len());
    }
    Ok(usage)
}

/// Exclusive mutation lease for repair and publication.
///
/// Valid immutable reads never acquire this lock, and the holder never launches
/// Chromium. This keeps capture work concurrent while serializing namespace
/// changes and capacity accounting.
struct CacheLease {
    _file: File,
}

impl CacheLease {
    async fn acquire(directory: &Path) -> Result<Self, ArtifactCacheError> {
        let path = directory.join(CACHE_LOCK);
        tokio::task::spawn_blocking(move || {
            let file = OpenOptions::new()
                .create(true)
                .read(true)
                .truncate(false)
                .write(true)
                .open(&path)
                .map_err(|source| ArtifactCacheError::Lock {
                    path: path.clone(),
                    source,
                })?;
            file.lock()
                .map_err(|source| ArtifactCacheError::Lock { path, source })?;
            Ok(Self { _file: file })
        })
        .await
        .map_err(ArtifactCacheError::LockTask)?
    }
}

fn capture_environment(
    seed: &[u8; CaptureEnvironmentId::BYTE_LENGTH],
    capture_mode: BrowserCaptureMode,
    graphics_backend: BrowserGraphicsBackend,
) -> CaptureEnvironmentId {
    let mut hash = Sha256::new();
    hash.update(IDENTITY_DOMAIN);
    hash.update(seed);
    hash.update(env!("CARGO_PKG_VERSION").as_bytes());
    hash.update(capture_mode_name(capture_mode).as_bytes());
    hash.update(graphics_backend_name(graphics_backend).as_bytes());
    CaptureEnvironmentId::from_sha256(hash.finalize().into())
}

fn ephemeral_environment(
    capture_mode: BrowserCaptureMode,
    graphics_backend: BrowserGraphicsBackend,
) -> CaptureEnvironmentId {
    capture_environment(
        &[0; CaptureEnvironmentId::BYTE_LENGTH],
        capture_mode,
        graphics_backend,
    )
}

const fn capture_mode_name(capture_mode: BrowserCaptureMode) -> &'static str {
    match capture_mode {
        BrowserCaptureMode::BeginFrame => "begin-frame",
        BrowserCaptureMode::Screenshot => "screenshot",
    }
}

const fn graphics_backend_name(graphics_backend: BrowserGraphicsBackend) -> &'static str {
    match graphics_backend {
        BrowserGraphicsBackend::SwiftShader => "swiftshader",
        #[cfg(target_os = "macos")]
        BrowserGraphicsBackend::Metal => "metal",
    }
}

/// Typed local-cache failure at the CLI composition boundary.
#[derive(Debug)]
pub(super) enum ArtifactCacheError {
    IncompleteEnvironment,
    NonUtf8EnvironmentSeed,
    InvalidEnvironmentSeed(InvalidCaptureEnvironmentId),
    FrameAccounting,
    Directory { path: PathBuf, source: io::Error },
    Staging(io::Error),
    Lock { path: PathBuf, source: io::Error },
    LockTask(JoinError),
    Inspect { path: PathBuf, source: io::Error },
    Remove { path: PathBuf, source: io::Error },
    Publish { path: PathBuf, source: io::Error },
    Artifact(FrameArtifactError),
    Render(RenderError),
}

impl fmt::Display for ArtifactCacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IncompleteEnvironment => formatter.write_str(
                "local frame cache directory and capture environment must be configured together",
            ),
            Self::NonUtf8EnvironmentSeed => {
                formatter.write_str("local capture environment identity is not UTF-8")
            }
            Self::InvalidEnvironmentSeed(source) => source.fmt(formatter),
            Self::FrameAccounting => {
                formatter.write_str("reused frame count exceeds its accounting domain")
            }
            Self::Directory { path, .. } => {
                write!(
                    formatter,
                    "failed to create local frame cache {}",
                    path.display()
                )
            }
            Self::Staging(_) => {
                formatter.write_str("failed to create frame capture staging directory")
            }
            Self::Lock { path, .. } => {
                write!(
                    formatter,
                    "failed to lock local frame cache {}",
                    path.display()
                )
            }
            Self::LockTask(_) => formatter.write_str("local frame cache lock task did not finish"),
            Self::Inspect { path, .. } => {
                write!(
                    formatter,
                    "failed to inspect frame cache {}",
                    path.display()
                )
            }
            Self::Remove { path, .. } => {
                write!(
                    formatter,
                    "failed to remove corrupt frame artifact {}",
                    path.display()
                )
            }
            Self::Publish { path, .. } => {
                write!(
                    formatter,
                    "failed to publish local frame artifact {}",
                    path.display()
                )
            }
            Self::Artifact(source) => source.fmt(formatter),
            Self::Render(source) => source.fmt(formatter),
        }
    }
}

impl Error for ArtifactCacheError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidEnvironmentSeed(source) => Some(source),
            Self::Directory { source, .. }
            | Self::Lock { source, .. }
            | Self::Inspect { source, .. }
            | Self::Remove { source, .. }
            | Self::Publish { source, .. }
            | Self::Staging(source) => Some(source),
            Self::LockTask(source) => Some(source),
            Self::Artifact(source) => Some(source),
            Self::Render(source) => Some(source),
            Self::IncompleteEnvironment | Self::NonUtf8EnvironmentSeed | Self::FrameAccounting => {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use onmark_render::{
        BrowserCaptureMode, BrowserGraphicsBackend, CaptureEnvironmentId, FrameArtifactId,
        FrameArtifactLimits,
    };
    use tempfile::tempdir;

    use super::{
        CacheEntry, artifact_path, cache_usage, capture_environment, inspect_cache_entry,
        remove_corrupt,
    };

    #[test]
    fn native_capture_policy_participates_in_environment_identity() {
        let seed = [7; CaptureEnvironmentId::BYTE_LENGTH];
        let begin_frame = capture_environment(
            &seed,
            BrowserCaptureMode::BeginFrame,
            BrowserGraphicsBackend::SwiftShader,
        );
        let screenshot = capture_environment(
            &seed,
            BrowserCaptureMode::Screenshot,
            BrowserGraphicsBackend::SwiftShader,
        );

        assert_ne!(begin_frame, screenshot);
        assert_eq!(
            begin_frame,
            capture_environment(
                &seed,
                BrowserCaptureMode::BeginFrame,
                BrowserGraphicsBackend::SwiftShader,
            ),
        );
    }

    #[test]
    fn cache_accounting_owns_only_canonical_frame_artifacts() {
        let directory = tempdir().expect("the fixture cache is available");
        let id = FrameArtifactId::parse(
            "sha256:0101010101010101010101010101010101010101010101010101010101010101",
        )
        .expect("the fixture identity is canonical");
        let artifact = artifact_path(directory.path(), id);
        std::fs::write(&artifact, b"artifact").expect("the fixture artifact is writable");
        std::fs::write(directory.path().join(".cache.lock"), b"")
            .expect("the cache lock fixture is writable");

        let mut usage = cache_usage(directory.path()).expect("the fixture cache can be counted");

        assert_eq!(usage.artifacts, 1);
        assert_eq!(usage.bytes, 8);
        usage.remove(super::CacheUsage::one_artifact(8));
        assert_eq!(usage.artifacts, 0);
        assert_eq!(usage.bytes, 0);
        assert_eq!(
            artifact
                .file_name()
                .and_then(|name| name.to_str())
                .expect("the artifact name is UTF-8"),
            "0101010101010101010101010101010101010101010101010101010101010101.onmark-frames",
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn repairs_a_broken_symlink_without_debiting_cache_usage() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().expect("the fixture cache is available");
        let id = FrameArtifactId::parse(
            "sha256:0202020202020202020202020202020202020202020202020202020202020202",
        )
        .expect("the fixture identity is canonical");
        let artifact = artifact_path(directory.path(), id);
        symlink("missing.onmark-frames", &artifact)
            .expect("the fixture broken symlink can be created");
        let limits =
            FrameArtifactLimits::new(2, 1_024, 512).expect("the fixture limits are bounded");

        let entry = inspect_cache_entry(&artifact, id, limits)
            .await
            .expect("a broken symlink is a readable cache state");

        assert!(matches!(entry, CacheEntry::Corrupt));
        let removed = remove_corrupt(&artifact).expect("the corrupt cache entry can be removed");
        assert_eq!(removed.artifacts, 0);
        assert_eq!(removed.bytes, 0);
        assert!(std::fs::symlink_metadata(&artifact).is_err());
    }
}
