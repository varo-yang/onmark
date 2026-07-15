//! Immutable worker handoff containing ordered captured frames and provenance.
//!
//! The artifact is an intermediate visual fact, not a separately encoded video;
//! final assembly therefore retains one continuous encoder and audio mix.

mod format;
mod reader;
mod writer;

use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use format::{FrameArtifactDescriptor, Header};
pub use format::{FrameArtifactId, InvalidFrameArtifactId};
use reader::{FrameArtifactFingerprintSequence, open_verified};

pub(crate) use reader::FrameArtifactReader;
pub(crate) use writer::FrameArtifactWriter;

use crate::{CaptureEnvironmentId, ExecutableUnit};

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

/// Immutable PNG output and raw-pixel fingerprints from one completed worker
/// unit.
///
/// The artifact is one file rather than a directory of frame objects: its
/// fixed header binds the artifact to its input unit and locked capture
/// environment, and its length-prefixed payload lets an assembler verify and
/// stream one frame at a time.
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

    /// Returns the locked capture environment bound into this artifact.
    #[must_use]
    pub const fn capture_environment(&self) -> CaptureEnvironmentId {
        self.header.descriptor.capture_environment
    }

    /// Returns the stable identity of this artifact's visual contract.
    #[must_use]
    pub fn id(&self) -> FrameArtifactId {
        self.header.descriptor.id()
    }

    pub(crate) fn matches_capture(
        &self,
        unit: &ExecutableUnit,
        capture_environment: CaptureEnvironmentId,
    ) -> bool {
        self.header.descriptor == FrameArtifactDescriptor::from_unit(unit, capture_environment)
    }

    pub(crate) async fn writer_for_capture(
        unit: &ExecutableUnit,
        capture_environment: CaptureEnvironmentId,
        output: &Path,
        limits: FrameArtifactLimits,
    ) -> Result<FrameArtifactWriter, FrameArtifactError> {
        FrameArtifactWriter::create(
            output,
            FrameArtifactDescriptor::from_unit(unit, capture_environment),
            limits,
        )
        .await
    }

    pub(crate) async fn reuse_for_capture(
        unit: &ExecutableUnit,
        capture_environment: CaptureEnvironmentId,
        output: &Path,
        limits: FrameArtifactLimits,
    ) -> Result<Self, FrameArtifactError> {
        let artifact = Self::open(output, limits).await?;
        if !artifact.matches_capture(unit, capture_environment) {
            return Err(Self::identity_mismatch(output));
        }
        artifact.verify().await?;
        Ok(artifact)
    }

    pub(crate) fn identity_mismatch(path: &Path) -> FrameArtifactError {
        FrameArtifactError::new(
            FrameArtifactErrorKind::IdentityMismatch,
            path,
            "frame artifact belongs to a different render unit or capture environment",
        )
    }

    /// Verifies every artifact record and the final payload checksum.
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
        while reader.next_fingerprint().await?.is_some() {}
        Ok(())
    }

    /// Verifies that two same-environment artifact sequences have equal
    /// raw-RGBA fingerprints.
    ///
    /// This is a bounded equivalence check: it streams each PNG through a
    /// fixed hash buffer and retains only one fingerprint from each sequence.
    /// It deliberately compares canonical pixels rather than PNG compression
    /// bytes.
    ///
    /// # Errors
    ///
    /// Returns [`FrameArtifactError`] when either artifact sequence is invalid,
    /// their capture environments differ, or their ordered raw-RGBA frame
    /// fingerprints differ.
    pub async fn verify_raw_rgba_equivalence(
        expected: &[Self],
        actual: &[Self],
    ) -> Result<(), FrameArtifactError> {
        let path = sequence_path(expected, actual);
        ensure_one_capture_environment(expected, actual, path)?;
        let mut expected_frames = FrameArtifactFingerprintSequence::new(expected);
        let mut actual_frames = FrameArtifactFingerprintSequence::new(actual);
        let mut position = 0_u128;

        while let Some(expected_fingerprint) = expected_frames.next_fingerprint().await? {
            let Some(actual_fingerprint) = actual_frames.next_fingerprint().await? else {
                return Err(actual_ends(path, position));
            };
            if expected_fingerprint != actual_fingerprint {
                return Err(fingerprint_differs(path, position));
            }
            position += 1;
        }
        if actual_frames.next_fingerprint().await?.is_some() {
            return Err(actual_continues(path, position));
        }
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
    /// An artifact belongs to a different planned unit or capture environment.
    IdentityMismatch,
    /// Two completed artifact sequences have different canonical pixels.
    RawRgbaMismatch,
    /// Artifact sequences were captured in different locked environments.
    CaptureEnvironmentMismatch,
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

fn sequence_path<'a>(expected: &'a [FrameArtifact], actual: &'a [FrameArtifact]) -> &'a Path {
    expected
        .first()
        .or_else(|| actual.first())
        .map_or_else(|| Path::new("."), FrameArtifact::path)
}

fn ensure_one_capture_environment(
    expected: &[FrameArtifact],
    actual: &[FrameArtifact],
    path: &Path,
) -> Result<(), FrameArtifactError> {
    let mut environments = expected
        .iter()
        .chain(actual)
        .map(FrameArtifact::capture_environment);
    let Some(environment) = environments.next() else {
        return Ok(());
    };

    for candidate in environments {
        if candidate != environment {
            return Err(FrameArtifactError::new(
                FrameArtifactErrorKind::CaptureEnvironmentMismatch,
                path,
                "raw-RGBA artifact comparison requires one capture environment",
            ));
        }
    }
    Ok(())
}

fn fingerprint_differs(path: &Path, position: u128) -> FrameArtifactError {
    FrameArtifactError::new(
        FrameArtifactErrorKind::RawRgbaMismatch,
        path,
        format!("raw-RGBA frame fingerprint differs at position {position}"),
    )
}

fn actual_ends(path: &Path, position: u128) -> FrameArtifactError {
    FrameArtifactError::new(
        FrameArtifactErrorKind::RawRgbaMismatch,
        path,
        format!("actual raw-RGBA frame sequence ends at position {position}"),
    )
}

fn actual_continues(path: &Path, position: u128) -> FrameArtifactError {
    FrameArtifactError::new(
        FrameArtifactErrorKind::RawRgbaMismatch,
        path,
        format!("actual raw-RGBA frame sequence has an extra frame at position {position}"),
    )
}

#[cfg(test)]
mod tests {
    use onmark_core::model::{FrameIndex, FrameInterval, FrameRate};
    use std::path::Path;
    use tempfile::tempdir;

    use super::format::{FrameArtifactDescriptor, Header};
    use super::{
        FrameArtifact, FrameArtifactErrorKind, FrameArtifactId, FrameArtifactLimits,
        FrameArtifactWriter,
    };
    use crate::{CaptureEnvironmentId, CapturedFrame, EncodedPng, RawRgbaHash, RenderProfile};

    #[test]
    fn preserves_the_canonical_frame_artifact_id_spelling_at_the_wire_boundary() {
        let spelling = "sha256:abababababababababababababababababababababababababababababababab";
        let id = FrameArtifactId::parse(spelling).expect("the fixture identity is canonical");
        let encoded = serde_json::to_string(&id).expect("the identity serializes as one string");
        let decoded: FrameArtifactId =
            serde_json::from_str(&encoded).expect("the identity parses from its wire spelling");

        assert_eq!(id.to_string(), spelling);
        assert_eq!(encoded, format!("\"{spelling}\""));
        assert_eq!(decoded, id);
        assert!(FrameArtifactId::parse("sha256:AB").is_err());
        assert!(FrameArtifactId::parse("sha512:ab").is_err());
    }

    #[tokio::test]
    async fn publishes_one_verified_frame_without_exposing_its_staging_file() {
        let directory = tempdir().expect("the fixture directory is available");
        let path = directory.path().join("worker.onmark-frames");
        let mut writer = FrameArtifactWriter::create(&path, descriptor(), limits())
            .await
            .expect("the artifact writer can stage one frame");
        writer
            .write_frame(&captured_frame())
            .await
            .expect("the frame fits the artifact limits");

        writer
            .finish()
            .await
            .expect("the completed artifact publishes atomically");

        let reopened = FrameArtifact::open(&path, limits())
            .await
            .expect("the published artifact opens through its fixed header");

        assert_eq!(reopened.path(), path);
        assert_eq!(reopened.output().start().get(), 4);
        assert_eq!(reopened.frames(), 1);
        assert_eq!(reopened.capture_environment(), capture_environment());
        reopened
            .verify()
            .await
            .expect("the published artifact verifies its payload");
    }

    #[tokio::test]
    async fn gives_the_same_identity_to_the_request_and_completed_artifact() {
        let directory = tempdir().expect("the fixture directory is available");
        let path = directory.path().join("worker.onmark-frames");
        let descriptor = descriptor();
        let expected = descriptor.id();
        let mut writer = FrameArtifactWriter::create(&path, descriptor, limits())
            .await
            .expect("the artifact writer can stage one frame");
        writer
            .write_frame(&captured_frame())
            .await
            .expect("the frame fits the artifact limits");

        let artifact = writer
            .finish()
            .await
            .expect("the completed artifact publishes atomically");

        assert_eq!(artifact.id(), expected);
    }

    #[tokio::test]
    async fn preserves_the_captured_raw_rgba_hash_with_each_frame() {
        let directory = tempdir().expect("the fixture directory is available");
        let path = directory.path().join("worker.onmark-frames");
        let expected = RawRgbaHash::from_bytes([7; RawRgbaHash::BYTE_LENGTH]);
        let mut writer = FrameArtifactWriter::create(&path, descriptor(), limits())
            .await
            .expect("the artifact writer can stage one frame");
        writer
            .write_frame(&CapturedFrame::recorded(EncodedPng::new(vec![1]), expected))
            .await
            .expect("the frame fits the artifact limits");
        let artifact = writer
            .finish()
            .await
            .expect("the completed artifact publishes atomically");
        let mut reader = artifact
            .reader()
            .await
            .expect("the completed artifact opens for streaming");
        let frame = reader
            .next_frame()
            .await
            .expect("the artifact frame reads")
            .expect("the artifact contains one frame");

        assert_eq!(frame.raw_rgba_hash(), expected);
    }

    #[tokio::test]
    async fn compares_ordered_raw_rgba_fingerprints_across_artifacts() {
        let directory = tempdir().expect("the fixture directory is available");
        let expected = artifact(
            &directory.path().join("expected.onmark-frames"),
            &[[1; RawRgbaHash::BYTE_LENGTH], [2; RawRgbaHash::BYTE_LENGTH]],
        )
        .await;
        let matching_first = artifact(
            &directory.path().join("matching-first.onmark-frames"),
            &[[1; RawRgbaHash::BYTE_LENGTH]],
        )
        .await;
        let matching_second = artifact(
            &directory.path().join("matching-second.onmark-frames"),
            &[[2; RawRgbaHash::BYTE_LENGTH]],
        )
        .await;
        let different = artifact(
            &directory.path().join("different.onmark-frames"),
            &[[3; RawRgbaHash::BYTE_LENGTH]],
        )
        .await;

        FrameArtifact::verify_raw_rgba_equivalence(
            std::slice::from_ref(&expected),
            &[matching_first.clone(), matching_second],
        )
        .await
        .expect("matching raw RGBA frames must compare equally");
        let error = FrameArtifact::verify_raw_rgba_equivalence(
            std::slice::from_ref(&expected),
            &[matching_first, different],
        )
        .await
        .expect_err("different raw RGBA frames must not compare equally");

        assert_eq!(error.kind(), FrameArtifactErrorKind::RawRgbaMismatch);
    }

    #[tokio::test]
    async fn rejects_raw_rgba_sequences_with_different_lengths() {
        let directory = tempdir().expect("the fixture directory is available");
        let complete = artifact(
            &directory.path().join("complete.onmark-frames"),
            &[[1; RawRgbaHash::BYTE_LENGTH], [2; RawRgbaHash::BYTE_LENGTH]],
        )
        .await;
        let prefix = artifact(
            &directory.path().join("prefix.onmark-frames"),
            &[[1; RawRgbaHash::BYTE_LENGTH]],
        )
        .await;

        let short = FrameArtifact::verify_raw_rgba_equivalence(
            std::slice::from_ref(&complete),
            std::slice::from_ref(&prefix),
        )
        .await
        .expect_err("a shorter raw RGBA sequence must not compare equally");
        let long = FrameArtifact::verify_raw_rgba_equivalence(
            std::slice::from_ref(&prefix),
            std::slice::from_ref(&complete),
        )
        .await
        .expect_err("a longer raw RGBA sequence must not compare equally");

        assert_eq!(short.kind(), FrameArtifactErrorKind::RawRgbaMismatch);
        assert!(short.to_string().contains("ends"));
        assert_eq!(long.kind(), FrameArtifactErrorKind::RawRgbaMismatch);
        assert!(long.to_string().contains("extra"));
    }

    #[tokio::test]
    async fn rejects_raw_rgba_comparison_across_capture_environments() {
        let directory = tempdir().expect("the fixture directory is available");
        let expected = artifact_in_environment(
            &directory.path().join("expected.onmark-frames"),
            &[[1; RawRgbaHash::BYTE_LENGTH]],
            CaptureEnvironmentId::from_sha256([7; CaptureEnvironmentId::BYTE_LENGTH]),
        )
        .await;
        let actual = artifact_in_environment(
            &directory.path().join("actual.onmark-frames"),
            &[[1; RawRgbaHash::BYTE_LENGTH]],
            CaptureEnvironmentId::from_sha256([8; CaptureEnvironmentId::BYTE_LENGTH]),
        )
        .await;

        let error = FrameArtifact::verify_raw_rgba_equivalence(
            std::slice::from_ref(&expected),
            std::slice::from_ref(&actual),
        )
        .await
        .expect_err("raw RGBA conformance requires one locked environment");

        assert_eq!(
            error.kind(),
            FrameArtifactErrorKind::CaptureEnvironmentMismatch
        );
    }

    #[tokio::test]
    async fn rejects_an_artifact_with_a_tampered_payload_checksum() {
        let directory = tempdir().expect("the fixture directory is available");
        let path = directory.path().join("worker.onmark-frames");
        let header = Header {
            descriptor: descriptor(),
            frames: 1,
            payload_bytes: 41,
            digest: [0; 32],
        };
        let mut bytes = header.encode().to_vec();
        bytes.extend_from_slice(&1_u64.to_be_bytes());
        bytes.push(1);
        bytes.extend_from_slice(&[0; 32]);
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
    async fn rejects_the_previous_header_layout_version() {
        let directory = tempdir().expect("the fixture directory is available");
        let path = directory.path().join("worker.onmark-frames");
        let mut header = Header {
            descriptor: descriptor(),
            frames: 1,
            payload_bytes: 41,
            digest: [0; 32],
        }
        .encode();
        header[8..10].copy_from_slice(&2_u16.to_be_bytes());
        tokio::fs::write(&path, header)
            .await
            .expect("the old-version fixture is writable");

        let error = FrameArtifact::open(&path, limits())
            .await
            .expect_err("the previous frame artifact layout must not decode as V3");

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
        descriptor_with_frames(1)
    }

    fn descriptor_with_frames(frames: u64) -> FrameArtifactDescriptor {
        descriptor_with_environment(frames, capture_environment())
    }

    fn descriptor_with_environment(
        frames: u64,
        capture_environment: CaptureEnvironmentId,
    ) -> FrameArtifactDescriptor {
        FrameArtifactDescriptor {
            output: FrameInterval::new(FrameIndex::new(4), FrameIndex::new(4 + frames))
                .expect("the fixture interval is ordered"),
            frame_rate: FrameRate::new(30, 1).expect("the fixture rate is valid"),
            profile: RenderProfile::new(320, 180).expect("the fixture profile is valid"),
            capture_environment,
            visual_plan_digest: [1; 32],
        }
    }

    fn capture_environment() -> CaptureEnvironmentId {
        CaptureEnvironmentId::from_sha256([7; CaptureEnvironmentId::BYTE_LENGTH])
    }

    fn captured_frame() -> CapturedFrame {
        CapturedFrame::recorded(
            EncodedPng::new(vec![1]),
            RawRgbaHash::from_bytes([1; RawRgbaHash::BYTE_LENGTH]),
        )
    }

    async fn artifact(
        path: &Path,
        raw_rgba_hashes: &[[u8; RawRgbaHash::BYTE_LENGTH]],
    ) -> FrameArtifact {
        artifact_in_environment(path, raw_rgba_hashes, capture_environment()).await
    }

    async fn artifact_in_environment(
        path: &Path,
        raw_rgba_hashes: &[[u8; RawRgbaHash::BYTE_LENGTH]],
        capture_environment: CaptureEnvironmentId,
    ) -> FrameArtifact {
        let frames = u64::try_from(raw_rgba_hashes.len())
            .expect("the fixture frame count fits the artifact domain");
        let mut writer = FrameArtifactWriter::create(
            path,
            descriptor_with_environment(frames, capture_environment),
            limits(),
        )
        .await
        .expect("the artifact writer can stage fixture frames");
        for raw_rgba_hash in raw_rgba_hashes {
            writer
                .write_frame(&CapturedFrame::recorded(
                    EncodedPng::new(vec![1]),
                    RawRgbaHash::from_bytes(*raw_rgba_hash),
                ))
                .await
                .expect("the frame fits the artifact limits");
        }
        writer
            .finish()
            .await
            .expect("the artifact publishes atomically")
    }
}
