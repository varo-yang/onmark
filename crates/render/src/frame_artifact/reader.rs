//! Bounded sequential validation of an immutable frame artifact.
//!
//! Records are consumed in order and checked against both per-frame and whole-
//! artifact identities; no untrusted length controls an unbounded allocation.

use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use tokio::fs::{self, File};
use tokio::io::AsyncReadExt as _;

use super::format::{FRAME_LENGTH_BYTES, HEADER_BYTES, Header, RAW_RGBA_HASH_BYTES};
use super::{FrameArtifact, FrameArtifactError, FrameArtifactErrorKind, FrameArtifactLimits};
use crate::{CapturedFrame, EncodedPng, RawRgbaHash};

const PNG_HASH_BUFFER_BYTES: usize = 8 * 1024;

/// One bounded, sequential reader for an artifact payload.
pub(crate) struct FrameArtifactReader {
    file: File,
    path: PathBuf,
    header: Header,
    max_frame_bytes: usize,
    frames_read: u64,
    payload_bytes_read: u64,
    digest: Sha256,
}

/// One sequential fingerprint view spanning adjacent immutable artifacts.
pub(super) struct FrameArtifactFingerprintSequence<'a> {
    artifacts: std::slice::Iter<'a, FrameArtifact>,
    reader: Option<FrameArtifactReader>,
}

impl<'a> FrameArtifactFingerprintSequence<'a> {
    pub(super) fn new(artifacts: &'a [FrameArtifact]) -> Self {
        Self {
            artifacts: artifacts.iter(),
            reader: None,
        }
    }

    pub(super) async fn next_fingerprint(
        &mut self,
    ) -> Result<Option<RawRgbaHash>, FrameArtifactError> {
        loop {
            if let Some(reader) = self.reader.as_mut() {
                if let Some(fingerprint) = reader.next_verified_fingerprint().await? {
                    return Ok(Some(fingerprint));
                }
                self.reader = None;
                continue;
            }

            let Some(artifact) = self.artifacts.next() else {
                return Ok(None);
            };
            self.reader = Some(artifact.reader().await?);
        }
    }
}

impl FrameArtifactReader {
    pub(super) fn new(file: File, header: Header, path: PathBuf, max_frame_bytes: usize) -> Self {
        Self {
            file,
            path,
            header,
            max_frame_bytes,
            frames_read: 0,
            payload_bytes_read: 0,
            digest: Sha256::new(),
        }
    }

    /// Reads exactly one retained frame and verifies the final record on EOF.
    pub(crate) async fn next_frame(&mut self) -> Result<Option<CapturedFrame>, FrameArtifactError> {
        let Some(record) = self.next_record().await? else {
            return Ok(None);
        };
        let png = self.read_png(record.frame_len).await?;
        let fingerprint = self.read_fingerprint().await?;
        self.finish_record(&record)?;

        Ok(Some(CapturedFrame::recorded(png, fingerprint)))
    }

    /// Reads one fingerprint while hashing its PNG payload without retaining it.
    pub(super) async fn next_fingerprint(
        &mut self,
    ) -> Result<Option<RawRgbaHash>, FrameArtifactError> {
        let Some(record) = self.next_record().await? else {
            return Ok(None);
        };
        self.hash_png(record.frame_len).await?;
        let fingerprint = self.read_fingerprint().await?;
        self.finish_record(&record)?;

        Ok(Some(fingerprint))
    }

    async fn next_verified_fingerprint(
        &mut self,
    ) -> Result<Option<RawRgbaHash>, FrameArtifactError> {
        let Some(frame) = self.next_frame().await? else {
            return Ok(None);
        };
        let fingerprint = frame
            .png()
            .decode_rgba(self.header.descriptor.profile)
            .map_err(|source| FrameArtifactError::pixels(&self.path, source))?
            .fingerprint();
        if fingerprint != frame.raw_rgba_hash() {
            return Err(FrameArtifactError::invalid(
                &self.path,
                "frame artifact raw-RGBA fingerprint does not match its PNG pixels",
            ));
        }
        Ok(Some(fingerprint))
    }

    async fn next_record(&mut self) -> Result<Option<FrameRecord>, FrameArtifactError> {
        if self.frames_read == self.header.frames {
            return Ok(None);
        }

        let mut length = [0; std::mem::size_of::<u64>()];
        self.read_exact(&mut length, "failed to read frame artifact record length")
            .await?;
        let frame_bytes = u64::from_be_bytes(length);
        if frame_bytes == 0 {
            return Err(FrameArtifactError::invalid(
                &self.path,
                "frame artifact contains an empty PNG frame",
            ));
        }
        let frame_len = usize::try_from(frame_bytes).map_err(|_| {
            FrameArtifactError::invalid(
                &self.path,
                "frame artifact frame size exceeds this process address space",
            )
        })?;
        if frame_len > self.max_frame_bytes {
            return Err(FrameArtifactError::new(
                FrameArtifactErrorKind::FrameByteLimit,
                &self.path,
                "frame artifact PNG exceeds the configured per-frame byte limit",
            ));
        }
        let payload_bytes = self
            .payload_bytes_read
            .checked_add(FRAME_LENGTH_BYTES)
            .and_then(|bytes| bytes.checked_add(frame_bytes))
            .and_then(|bytes| bytes.checked_add(RAW_RGBA_HASH_BYTES))
            .ok_or_else(|| {
                FrameArtifactError::invalid(
                    &self.path,
                    "frame artifact payload exceeds its accounting domain",
                )
            })?;
        if payload_bytes > self.header.payload_bytes {
            return Err(FrameArtifactError::invalid(
                &self.path,
                "frame artifact record exceeds its declared payload",
            ));
        }

        self.digest.update(length);
        Ok(Some(FrameRecord {
            frame_len,
            payload_bytes,
        }))
    }

    async fn read_png(&mut self, frame_len: usize) -> Result<EncodedPng, FrameArtifactError> {
        let mut bytes = vec![0; frame_len];
        self.read_exact(&mut bytes, "failed to read frame artifact PNG payload")
            .await?;
        self.digest.update(&bytes);
        Ok(EncodedPng::new(bytes))
    }

    async fn hash_png(&mut self, frame_len: usize) -> Result<(), FrameArtifactError> {
        let mut remaining = frame_len;
        let mut buffer = [0; PNG_HASH_BUFFER_BYTES];

        while remaining > 0 {
            let length = remaining.min(buffer.len());
            self.read_exact(
                &mut buffer[..length],
                "failed to read frame artifact PNG payload",
            )
            .await?;
            self.digest.update(&buffer[..length]);
            remaining -= length;
        }
        Ok(())
    }

    async fn read_fingerprint(&mut self) -> Result<RawRgbaHash, FrameArtifactError> {
        let mut bytes = [0; RawRgbaHash::BYTE_LENGTH];
        self.read_exact(
            &mut bytes,
            "failed to read frame artifact raw-RGBA fingerprint",
        )
        .await?;
        self.digest.update(bytes);
        Ok(RawRgbaHash::from_bytes(bytes))
    }

    async fn read_exact(
        &mut self,
        bytes: &mut [u8],
        message: &'static str,
    ) -> Result<(), FrameArtifactError> {
        self.file.read_exact(bytes).await.map_err(|source| {
            FrameArtifactError::io(
                FrameArtifactErrorKind::InvalidArtifact,
                &self.path,
                message,
                source,
            )
        })?;
        Ok(())
    }

    fn finish_record(&mut self, record: &FrameRecord) -> Result<(), FrameArtifactError> {
        self.frames_read += 1;
        self.payload_bytes_read = record.payload_bytes;
        if self.frames_read == self.header.frames {
            self.verify_complete()?;
        }
        Ok(())
    }

    fn verify_complete(&mut self) -> Result<(), FrameArtifactError> {
        if self.payload_bytes_read != self.header.payload_bytes {
            return Err(FrameArtifactError::invalid(
                &self.path,
                "frame artifact payload ends before its declared byte count",
            ));
        }
        let digest: [u8; 32] = std::mem::take(&mut self.digest).finalize().into();
        if digest != self.header.digest {
            return Err(FrameArtifactError::invalid(
                &self.path,
                "frame artifact payload checksum does not match",
            ));
        }
        Ok(())
    }
}

/// One decoded record after its declared payload and fingerprints agree.
struct FrameRecord {
    frame_len: usize,
    payload_bytes: u64,
}

pub(super) async fn open_verified(
    path: &Path,
    limits: FrameArtifactLimits,
) -> Result<(File, Header), FrameArtifactError> {
    let metadata = fs::symlink_metadata(path).await.map_err(|source| {
        FrameArtifactError::io(
            FrameArtifactErrorKind::Input,
            path,
            "failed to inspect frame artifact",
            source,
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(FrameArtifactError::invalid(
            path,
            "frame artifact must be a regular file, not a symlink",
        ));
    }

    let mut file = File::open(path).await.map_err(|source| {
        FrameArtifactError::io(
            FrameArtifactErrorKind::Input,
            path,
            "failed to open frame artifact",
            source,
        )
    })?;
    let mut bytes = [0; HEADER_BYTES];
    file.read_exact(&mut bytes).await.map_err(|source| {
        FrameArtifactError::io(
            FrameArtifactErrorKind::InvalidArtifact,
            path,
            "failed to read frame artifact header",
            source,
        )
    })?;
    let header = Header::decode(path, bytes)?;
    header.validate(path, limits)?;
    let expected_size = u64::try_from(HEADER_BYTES)
        .expect("the fixed frame artifact header fits in u64")
        .checked_add(header.payload_bytes)
        .ok_or_else(|| {
            FrameArtifactError::invalid(
                path,
                "frame artifact file size exceeds its accounting domain",
            )
        })?;
    if metadata.len() != expected_size {
        return Err(FrameArtifactError::invalid(
            path,
            "frame artifact file size does not match its header",
        ));
    }

    Ok((file, header))
}
