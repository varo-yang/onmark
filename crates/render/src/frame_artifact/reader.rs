//! Bounded sequential validation of an immutable frame artifact.
//!
//! Records are consumed in order and checked against both per-frame and whole-
//! artifact identities; no untrusted length controls an unbounded allocation.

use std::path::{Path, PathBuf};

use onmark_core::protocol::{
    BrowserNodeId, BrowserVisualFinding, BrowserVisualFindings, MAX_BROWSER_VISUAL_FINDINGS,
};
use sha2::{Digest as _, Sha256};
use tokio::fs::{self, File};
use tokio::io::AsyncReadExt as _;

use super::format::{
    FRAME_LENGTH_BYTES, HEADER_BYTES, Header, RAW_RGBA_HASH_BYTES, VISUAL_FINDING_BYTES,
    VISUAL_FINDING_COUNT_BYTES, visual_issue,
};
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
            let Some(reader) = self.reader.as_mut() else {
                let Some(artifact) = self.artifacts.next() else {
                    return Ok(None);
                };
                self.reader = Some(artifact.reader().await?);
                continue;
            };
            let Some(frame) = reader.next_frame().await? else {
                self.reader = None;
                continue;
            };

            return Ok(Some(frame.raw_rgba_hash()));
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

    /// Reads one retained frame and verifies its pixels and final record.
    pub(crate) async fn next_frame(&mut self) -> Result<Option<CapturedFrame>, FrameArtifactError> {
        self.next_frame_with_visual_findings()
            .await
            .map(|record| record.map(|(frame, _)| frame))
    }

    pub(super) async fn next_frame_with_visual_findings(
        &mut self,
    ) -> Result<Option<(CapturedFrame, BrowserVisualFindings)>, FrameArtifactError> {
        let Some(record) = self.next_record().await? else {
            return Ok(None);
        };
        let png = self.read_png(record.frame_len).await?;
        let fingerprint = self.read_fingerprint().await?;
        let findings = self.read_visual_findings(&record).await?;
        let frame = CapturedFrame::recorded(png, fingerprint);
        self.verify_pixels(&frame)?;

        Ok(Some((frame, findings)))
    }

    /// Reads one recorded fingerprint while verifying the payload checksum.
    pub(super) async fn next_recorded_fingerprint(
        &mut self,
    ) -> Result<Option<RawRgbaHash>, FrameArtifactError> {
        self.next_recorded_fingerprint_with_visual_findings()
            .await
            .map(|record| record.map(|(fingerprint, _)| fingerprint))
    }

    pub(super) async fn next_recorded_fingerprint_with_visual_findings(
        &mut self,
    ) -> Result<Option<(RawRgbaHash, BrowserVisualFindings)>, FrameArtifactError> {
        let Some(record) = self.next_record().await? else {
            return Ok(None);
        };
        self.hash_png(record.frame_len).await?;
        let fingerprint = self.read_fingerprint().await?;
        let findings = self.read_visual_findings(&record).await?;

        Ok(Some((fingerprint, findings)))
    }

    fn verify_pixels(&self, frame: &CapturedFrame) -> Result<(), FrameArtifactError> {
        let actual = frame
            .png()
            .decode_rgba(self.header.descriptor.profile)
            .map_err(|source| FrameArtifactError::pixels(&self.path, source))?
            .fingerprint();
        if actual != frame.raw_rgba_hash() {
            return Err(FrameArtifactError::invalid(
                &self.path,
                "frame artifact raw-RGBA fingerprint does not match its PNG pixels",
            ));
        }
        Ok(())
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
            .and_then(|bytes| bytes.checked_add(VISUAL_FINDING_COUNT_BYTES))
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

    fn finish_record(&mut self, payload_bytes: u64) -> Result<(), FrameArtifactError> {
        self.frames_read += 1;
        self.payload_bytes_read = payload_bytes;
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

    async fn read_visual_findings(
        &mut self,
        record: &FrameRecord,
    ) -> Result<BrowserVisualFindings, FrameArtifactError> {
        let mut count = [0; std::mem::size_of::<u16>()];
        self.read_visual_exact(&mut count).await?;
        let count = usize::from(u16::from_be_bytes(count));
        if count > MAX_BROWSER_VISUAL_FINDINGS {
            return Err(FrameArtifactError::invalid(
                &self.path,
                "frame artifact visual-finding count exceeds the protocol limit",
            ));
        }
        let finding_bytes = u64::try_from(count)
            .expect("the protocol visual-finding limit fits in u64")
            * u64::try_from(VISUAL_FINDING_BYTES).expect("one visual finding fits in u64");
        let payload_bytes = record
            .payload_bytes
            .checked_add(finding_bytes)
            .ok_or_else(|| {
                FrameArtifactError::invalid(
                    &self.path,
                    "frame artifact visual evidence exceeds its accounting domain",
                )
            })?;
        if payload_bytes > self.header.payload_bytes {
            return Err(FrameArtifactError::invalid(
                &self.path,
                "frame artifact visual evidence exceeds its declared payload",
            ));
        }

        let mut findings = Vec::with_capacity(count);
        for _ in 0..count {
            let mut bytes = [0; VISUAL_FINDING_BYTES];
            self.read_visual_exact(&mut bytes).await?;
            let node_id = u32::from_be_bytes(
                bytes[..4]
                    .try_into()
                    .expect("one visual finding contains its node identity"),
            );
            let issue = visual_issue(bytes[4]).ok_or_else(|| {
                FrameArtifactError::invalid(&self.path, "frame artifact visual issue is invalid")
            })?;
            findings.push(BrowserVisualFinding::new(
                BrowserNodeId::new(node_id),
                issue,
            ));
        }
        let findings = BrowserVisualFindings::new(findings).map_err(|_| {
            FrameArtifactError::invalid(
                &self.path,
                "frame artifact visual findings are not canonical",
            )
        })?;
        self.finish_record(payload_bytes)?;
        Ok(findings)
    }

    async fn read_visual_exact(&mut self, bytes: &mut [u8]) -> Result<(), FrameArtifactError> {
        self.read_exact(bytes, "failed to read frame artifact visual evidence")
            .await?;
        self.digest.update(&*bytes);
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
