use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use tokio::fs::{self, File};
use tokio::io::AsyncReadExt as _;

use super::format::{FRAME_LENGTH_BYTES, HEADER_BYTES, Header};
use super::{FrameArtifactError, FrameArtifactErrorKind, FrameArtifactLimits};
use crate::EncodedPng;

/// One bounded, sequential view into a verified artifact payload.
pub(crate) struct FrameArtifactReader {
    file: File,
    path: PathBuf,
    header: Header,
    max_frame_bytes: usize,
    frames_read: u64,
    payload_bytes_read: u64,
    digest: Sha256,
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
    pub(crate) async fn next_frame(&mut self) -> Result<Option<EncodedPng>, FrameArtifactError> {
        if self.frames_read == self.header.frames {
            return Ok(None);
        }

        let mut length = [0; std::mem::size_of::<u64>()];
        self.file.read_exact(&mut length).await.map_err(|source| {
            FrameArtifactError::io(
                FrameArtifactErrorKind::InvalidArtifact,
                &self.path,
                "failed to read frame artifact record length",
                source,
            )
        })?;
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
        let next_payload_bytes = self
            .payload_bytes_read
            .checked_add(FRAME_LENGTH_BYTES)
            .and_then(|bytes| bytes.checked_add(frame_bytes))
            .ok_or_else(|| {
                FrameArtifactError::invalid(
                    &self.path,
                    "frame artifact payload exceeds its accounting domain",
                )
            })?;
        if next_payload_bytes > self.header.payload_bytes {
            return Err(FrameArtifactError::invalid(
                &self.path,
                "frame artifact record exceeds its declared payload",
            ));
        }

        let mut bytes = vec![0; frame_len];
        self.file.read_exact(&mut bytes).await.map_err(|source| {
            FrameArtifactError::io(
                FrameArtifactErrorKind::InvalidArtifact,
                &self.path,
                "failed to read frame artifact PNG payload",
                source,
            )
        })?;
        self.digest.update(length);
        self.digest.update(&bytes);
        self.frames_read += 1;
        self.payload_bytes_read = next_payload_bytes;

        if self.frames_read == self.header.frames {
            self.verify_complete()?;
        }

        Ok(Some(EncodedPng::new(bytes)))
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
