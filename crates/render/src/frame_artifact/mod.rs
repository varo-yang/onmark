mod format;
mod reader;
mod writer;

use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use format::{FrameArtifactDescriptor, Header};
use reader::open_verified;

pub(crate) use reader::FrameArtifactReader;
pub(crate) use writer::FrameArtifactWriter;

use crate::ExecutableUnit;

const MAX_FRAMES: u64 = 10_000_000;
const MAX_BYTES: u64 = 1 << 40;
const MAX_FRAME_BYTES: usize = 512 * 1024 * 1024;

/// Retained-storage limits for one immutable worker frame artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameArtifactLimits {
    frames: u64,
    bytes: u64,
    frame_bytes: usize,
}

impl FrameArtifactLimits {
    /// Creates one bounded worker-artifact policy.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidFrameArtifactLimits`] when a bound is zero or exceeds
    /// the fixed Gate-three safety envelope.
    pub fn new(
        max_frames: u64,
        max_bytes: u64,
        max_frame_bytes: usize,
    ) -> Result<Self, InvalidFrameArtifactLimits> {
        if max_frames == 0 {
            return Err(InvalidFrameArtifactLimits::ZeroFrames);
        }
        if max_frames > MAX_FRAMES {
            return Err(InvalidFrameArtifactLimits::TooManyFrames);
        }
        if max_bytes == 0 {
            return Err(InvalidFrameArtifactLimits::ZeroBytes);
        }
        if max_bytes > MAX_BYTES {
            return Err(InvalidFrameArtifactLimits::TooManyBytes);
        }
        if max_frame_bytes == 0 {
            return Err(InvalidFrameArtifactLimits::ZeroFrameBytes);
        }
        if max_frame_bytes > MAX_FRAME_BYTES {
            return Err(InvalidFrameArtifactLimits::FrameBytesTooLarge);
        }
        let frame_bytes = u64::try_from(max_frame_bytes)
            .map_err(|_| InvalidFrameArtifactLimits::FrameBytesTooLarge)?;
        if frame_bytes > max_bytes {
            return Err(InvalidFrameArtifactLimits::FrameBytesExceedArtifact);
        }
        Ok(Self {
            frames: max_frames,
            bytes: max_bytes,
            frame_bytes: max_frame_bytes,
        })
    }

    pub(crate) const fn max_frames(self) -> u64 {
        self.frames
    }

    const fn max_bytes(self) -> u64 {
        self.bytes
    }

    const fn max_frame_bytes(self) -> usize {
        self.frame_bytes
    }
}

/// Reason a worker-artifact policy cannot bound retained output safely.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidFrameArtifactLimits {
    /// No captured frames may be retained.
    ZeroFrames,
    /// The requested frame count exceeds the fixed safety ceiling.
    TooManyFrames,
    /// No encoded PNG bytes may be retained.
    ZeroBytes,
    /// The requested byte budget exceeds one tebibyte.
    TooManyBytes,
    /// No one captured PNG may be retained.
    ZeroFrameBytes,
    /// One captured PNG exceeds the fixed in-memory safety ceiling.
    FrameBytesTooLarge,
    /// One captured PNG would exceed the total artifact budget.
    FrameBytesExceedArtifact,
}

impl fmt::Display for InvalidFrameArtifactLimits {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroFrames => "frame artifact frame limit must be positive",
            Self::TooManyFrames => "frame artifact frame limit exceeds the safety ceiling",
            Self::ZeroBytes => "frame artifact byte limit must be positive",
            Self::TooManyBytes => "frame artifact byte limit exceeds the safety ceiling",
            Self::ZeroFrameBytes => "frame artifact per-frame byte limit must be positive",
            Self::FrameBytesTooLarge => {
                "frame artifact per-frame byte limit exceeds the safety ceiling"
            }
            Self::FrameBytesExceedArtifact => {
                "frame artifact per-frame byte limit exceeds the total byte limit"
            }
        })
    }
}

impl Error for InvalidFrameArtifactLimits {}

/// Immutable, ordered PNG output published by one completed worker unit.
///
/// The artifact is one file rather than a directory of frame objects: its
/// fixed header binds the artifact to the input unit, and its length-prefixed
/// payload lets an assembler verify and stream one frame at a time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameArtifact {
    path: PathBuf,
    header: Header,
    limits: FrameArtifactLimits,
}

impl FrameArtifact {
    /// Opens one completed worker artifact and validates its fixed envelope.
    ///
    /// # Errors
    ///
    /// Returns [`FrameArtifactError`] when the header, declared extent, or
    /// retained-storage limits are invalid. Call [`Self::verify`] to read and
    /// checksum every payload record before a reuse decision.
    pub async fn open(
        path: impl Into<PathBuf>,
        limits: FrameArtifactLimits,
    ) -> Result<Self, FrameArtifactError> {
        let path = path.into();
        let (_, header) = open_verified(&path, limits).await?;

        Ok(Self::from_header(path, header, limits))
    }

    /// Returns the immutable artifact path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the exact frame interval published by this artifact.
    #[must_use]
    pub const fn output(&self) -> onmark_core::model::FrameInterval {
        self.header.descriptor.output
    }

    /// Returns the number of encoded PNG frames in output order.
    #[must_use]
    pub const fn frames(&self) -> u64 {
        self.header.frames
    }

    pub(crate) fn matches_unit(&self, unit: &ExecutableUnit) -> bool {
        self.header.descriptor == FrameArtifactDescriptor::from_unit(unit)
    }

    pub(crate) async fn writer_for_unit(
        unit: &ExecutableUnit,
        output: &Path,
        limits: FrameArtifactLimits,
    ) -> Result<FrameArtifactWriter, FrameArtifactError> {
        FrameArtifactWriter::create(output, FrameArtifactDescriptor::from_unit(unit), limits).await
    }

    pub(crate) async fn reuse_for_unit(
        unit: &ExecutableUnit,
        output: &Path,
        limits: FrameArtifactLimits,
    ) -> Result<Self, FrameArtifactError> {
        let artifact = Self::open(output, limits).await?;
        if !artifact.matches_unit(unit) {
            return Err(Self::identity_mismatch(output));
        }
        artifact.verify().await?;
        Ok(artifact)
    }

    pub(crate) fn identity_mismatch(path: &Path) -> FrameArtifactError {
        FrameArtifactError::new(
            FrameArtifactErrorKind::IdentityMismatch,
            path,
            "frame artifact belongs to a different render unit",
        )
    }

    /// Verifies every length-prefixed PNG record and the final payload checksum.
    ///
    /// This reads the artifact without launching a browser or encoder, which is
    /// useful to a worker retry before it reuses an immutable publication.
    ///
    /// # Errors
    ///
    /// Returns [`FrameArtifactError`] when the artifact changes, truncates, or
    /// fails its declared checksum.
    pub async fn verify(&self) -> Result<(), FrameArtifactError> {
        let mut reader = self.reader().await?;
        while reader.next_frame().await?.is_some() {}
        Ok(())
    }

    pub(crate) async fn reader(&self) -> Result<FrameArtifactReader, FrameArtifactError> {
        let (file, header) = open_verified(&self.path, self.limits).await?;
        if header != self.header {
            return Err(FrameArtifactError::invalid(
                &self.path,
                "frame artifact changed after validation",
            ));
        }

        Ok(FrameArtifactReader::new(
            file,
            header,
            self.path.clone(),
            self.limits.max_frame_bytes(),
        ))
    }

    pub(in crate::frame_artifact) fn from_header(
        path: PathBuf,
        header: Header,
        limits: FrameArtifactLimits,
    ) -> Self {
        Self {
            path,
            header,
            limits,
        }
    }
}

/// Stable category for a frame-artifact boundary failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum FrameArtifactErrorKind {
    /// A destination artifact already exists.
    OutputExists,
    /// A source artifact could not be inspected or opened.
    Input,
    /// Artifact staging, writing, or publication failed.
    Output,
    /// Header or payload bytes violate the artifact contract.
    InvalidArtifact,
    /// The artifact exceeds the configured frame count.
    FrameLimit,
    /// The artifact exceeds the configured byte budget.
    ByteLimit,
    /// One PNG exceeds the configured retained-memory bound.
    FrameByteLimit,
    /// Capture ended before every planned output frame was retained.
    Incomplete,
    /// An artifact belongs to a different planned unit.
    IdentityMismatch,
}

/// Typed failure from the worker-frame artifact boundary.
#[derive(Debug)]
pub struct FrameArtifactError {
    kind: FrameArtifactErrorKind,
    path: PathBuf,
    message: Box<str>,
    source: Option<io::Error>,
}

impl FrameArtifactError {
    /// Returns the stable failure category.
    #[must_use]
    pub const fn kind(&self) -> FrameArtifactErrorKind {
        self.kind
    }

    /// Returns the source or destination artifact path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn new(
        kind: FrameArtifactErrorKind,
        path: &Path,
        message: impl Into<Box<str>>,
    ) -> Self {
        Self {
            kind,
            path: path.to_owned(),
            message: message.into(),
            source: None,
        }
    }

    pub(super) fn invalid(path: &Path, message: &'static str) -> Self {
        Self::new(FrameArtifactErrorKind::InvalidArtifact, path, message)
    }

    pub(super) fn io(
        kind: FrameArtifactErrorKind,
        path: &Path,
        message: impl Into<Box<str>>,
        source: io::Error,
    ) -> Self {
        Self {
            kind,
            path: path.to_owned(),
            message: message.into(),
            source: Some(source),
        }
    }
}

impl fmt::Display for FrameArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path.display(), self.message)
    }
}

impl Error for FrameArtifactError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref().map(|source| source as _)
    }
}

#[cfg(test)]
mod tests {
    use onmark_core::model::{FrameIndex, FrameInterval, FrameRate};
    use tempfile::tempdir;

    use super::format::{FrameArtifactDescriptor, Header};
    use super::{FrameArtifact, FrameArtifactErrorKind, FrameArtifactLimits, FrameArtifactWriter};
    use crate::{EncodedPng, RenderProfile};

    #[tokio::test]
    async fn publishes_one_verified_frame_without_exposing_its_staging_file() {
        let directory = tempdir().expect("the fixture directory is available");
        let path = directory.path().join("worker.onmark-frames");
        let mut writer = FrameArtifactWriter::create(&path, descriptor(), limits())
            .await
            .expect("the artifact writer can stage one frame");
        writer
            .write_frame(&EncodedPng::new(vec![1]))
            .await
            .expect("the frame fits the artifact limits");

        let artifact = writer
            .finish()
            .await
            .expect("the completed artifact publishes atomically");

        assert_eq!(artifact.path(), path);
        assert_eq!(artifact.output().start().get(), 4);
        assert_eq!(artifact.frames(), 1);
        artifact
            .verify()
            .await
            .expect("the published artifact verifies its payload");
    }

    #[tokio::test]
    async fn rejects_an_artifact_with_a_tampered_payload_checksum() {
        let directory = tempdir().expect("the fixture directory is available");
        let path = directory.path().join("worker.onmark-frames");
        let header = Header {
            descriptor: descriptor(),
            frames: 1,
            payload_bytes: 9,
            digest: [0; 32],
        };
        let mut bytes = header.encode().to_vec();
        bytes.extend_from_slice(&1_u64.to_be_bytes());
        bytes.push(1);
        tokio::fs::write(&path, bytes)
            .await
            .expect("the tampered artifact fixture is writable");

        let artifact = FrameArtifact::open(&path, limits())
            .await
            .expect("the header can be opened before payload verification");
        let error = artifact
            .verify()
            .await
            .expect_err("the checksum mismatch must reject the artifact");

        assert_eq!(error.kind(), FrameArtifactErrorKind::InvalidArtifact);
    }

    #[tokio::test]
    async fn rejects_a_header_that_cannot_name_every_declared_frame() {
        let directory = tempdir().expect("the fixture directory is available");
        let path = directory.path().join("worker.onmark-frames");
        let header = Header {
            descriptor: descriptor(),
            frames: 1,
            payload_bytes: 0,
            digest: [0; 32],
        };
        tokio::fs::write(&path, header.encode())
            .await
            .expect("the malformed artifact fixture is writable");

        let error = FrameArtifact::open(&path, limits())
            .await
            .expect_err("a frame needs a length and at least one byte");

        assert_eq!(error.kind(), FrameArtifactErrorKind::InvalidArtifact);
    }

    #[tokio::test]
    async fn drops_an_incomplete_staging_artifact_without_publication() {
        let directory = tempdir().expect("the fixture directory is available");
        let path = directory.path().join("worker.onmark-frames");
        let writer = FrameArtifactWriter::create(&path, descriptor(), limits())
            .await
            .expect("the artifact writer can reserve a staging file");

        let error = writer
            .finish()
            .await
            .expect_err("an artifact without its planned frame cannot publish");

        assert_eq!(error.kind(), FrameArtifactErrorKind::Incomplete);
        assert!(!path.exists());
    }

    fn limits() -> FrameArtifactLimits {
        FrameArtifactLimits::new(2, 128, 64).expect("the fixture limits are bounded")
    }

    fn descriptor() -> FrameArtifactDescriptor {
        FrameArtifactDescriptor {
            output: FrameInterval::new(FrameIndex::new(4), FrameIndex::new(5))
                .expect("the fixture interval is ordered"),
            frame_rate: FrameRate::new(30, 1).expect("the fixture rate is valid"),
            profile: RenderProfile::new(320, 180).expect("the fixture profile is valid"),
            visual_plan_digest: [1; 32],
        }
    }
}
