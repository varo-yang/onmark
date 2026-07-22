//! Single-owner construction and no-clobber publication of frame artifacts.
//!
//! Payload bytes remain in a private temporary file until the final header and
//! digest are complete. Dropping the writer cannot expose a partial artifact.

use std::path::Path;

use png::{BitDepth, ColorType, Encoder};
use sha2::{Digest as _, Sha256};
use tempfile::{NamedTempFile, TempPath};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncSeekExt as _, AsyncWriteExt as _, SeekFrom};

use super::format::{
    FRAME_LENGTH_BYTES, FrameArtifactDescriptor, HEADER_BYTES, Header, RAW_RGBA_HASH_BYTES,
};
use super::{FrameArtifact, FrameArtifactError, FrameArtifactErrorKind, FrameArtifactLimits};
use crate::{CapturedFrame, EncodedPng, RawRgbaHash, RenderProfile};

/// The only mutable state while one worker owns an unpublished artifact.
pub(crate) struct FrameArtifactWriter {
    file: File,
    staging: TempPath,
    output: std::path::PathBuf,
    descriptor: FrameArtifactDescriptor,
    limits: FrameArtifactLimits,
    frames: u64,
    payload_bytes: u64,
    digest: Sha256,
}

impl FrameArtifactWriter {
    pub(super) async fn create(
        output: &Path,
        descriptor: FrameArtifactDescriptor,
        limits: FrameArtifactLimits,
    ) -> Result<Self, FrameArtifactError> {
        if output.exists() {
            return Err(FrameArtifactError::new(
                FrameArtifactErrorKind::OutputExists,
                output,
                "frame artifact output already exists",
            ));
        }

        let staging = NamedTempFile::new_in(output_parent(output)).map_err(|source| {
            FrameArtifactError::io(
                FrameArtifactErrorKind::Output,
                output,
                "failed to create frame artifact staging file",
                source,
            )
        })?;
        let staging = staging.into_temp_path();
        let mut file = OpenOptions::new()
            .write(true)
            .open(&staging)
            .await
            .map_err(|source| {
                FrameArtifactError::io(
                    FrameArtifactErrorKind::Output,
                    output,
                    "failed to open frame artifact staging file",
                    source,
                )
            })?;
        file.write_all(&[0; HEADER_BYTES]).await.map_err(|source| {
            FrameArtifactError::io(
                FrameArtifactErrorKind::Output,
                output,
                "failed to reserve frame artifact header",
                source,
            )
        })?;

        Ok(Self {
            file,
            staging,
            output: output.to_owned(),
            descriptor,
            limits,
            frames: 0,
            payload_bytes: 0,
            digest: Sha256::new(),
        })
    }

    /// Appends one bounded PNG record and its raw-RGBA fingerprint in output order.
    pub(crate) async fn write_frame(
        &mut self,
        frame: &CapturedFrame,
    ) -> Result<(), FrameArtifactError> {
        let bytes = frame.png().as_bytes();
        let raw_rgba_hash = frame.raw_rgba_hash();
        let next = self.next_record(bytes)?;
        let length = next.frame_bytes.to_be_bytes();

        write_all(
            &mut self.file,
            &self.output,
            &length,
            "failed to write frame artifact record length",
        )
        .await?;
        write_all(
            &mut self.file,
            &self.output,
            bytes,
            "failed to write frame artifact PNG payload",
        )
        .await?;
        write_all(
            &mut self.file,
            &self.output,
            raw_rgba_hash.as_bytes(),
            "failed to write frame artifact raw-RGBA fingerprint",
        )
        .await?;

        self.digest.update(length);
        self.digest.update(bytes);
        self.digest.update(raw_rgba_hash.as_bytes());
        self.frames = next.frames;
        self.payload_bytes = next.payload_bytes;
        Ok(())
    }

    /// Encodes one canonical native RGBA frame into the lossless artifact form.
    pub(crate) async fn write_rgba_frame(
        &mut self,
        pixels: &[u8],
        fingerprint: RawRgbaHash,
        profile: RenderProfile,
    ) -> Result<(), FrameArtifactError> {
        let png = encode_rgba(pixels, profile, &self.output)?;
        self.write_frame(&CapturedFrame::recorded(png, fingerprint))
            .await
    }

    /// Finalizes the fixed header and publishes the staged bytes without replacement.
    pub(crate) async fn finish(self) -> Result<FrameArtifact, FrameArtifactError> {
        let expected_frames = self.descriptor.output.len().get();
        if self.frames != expected_frames {
            return Err(FrameArtifactError::new(
                FrameArtifactErrorKind::Incomplete,
                &self.output,
                "frame artifact does not contain its planned output frame count",
            ));
        }

        let Self {
            mut file,
            staging,
            output,
            descriptor,
            limits,
            frames,
            payload_bytes,
            digest,
        } = self;
        let header = Header {
            descriptor,
            frames,
            payload_bytes,
            digest: digest.finalize().into(),
        };
        file.seek(SeekFrom::Start(0)).await.map_err(|source| {
            FrameArtifactError::io(
                FrameArtifactErrorKind::Output,
                &output,
                "failed to seek frame artifact header",
                source,
            )
        })?;
        write_all(
            &mut file,
            &output,
            &header.encode(),
            "failed to write frame artifact header",
        )
        .await?;
        file.sync_all().await.map_err(|source| {
            FrameArtifactError::io(
                FrameArtifactErrorKind::Output,
                &output,
                "failed to flush frame artifact staging file",
                source,
            )
        })?;

        drop(file);
        publish(staging, &output)?;

        Ok(FrameArtifact::from_header(output, header, limits))
    }

    fn next_record(&self, bytes: &[u8]) -> Result<NextRecord, FrameArtifactError> {
        if bytes.is_empty() {
            return Err(FrameArtifactError::new(
                FrameArtifactErrorKind::InvalidArtifact,
                &self.output,
                "frame artifact cannot retain an empty PNG frame",
            ));
        }

        let frames = self.frames.checked_add(1).ok_or_else(|| {
            FrameArtifactError::new(
                FrameArtifactErrorKind::FrameLimit,
                &self.output,
                "frame artifact frame count exceeds its accounting domain",
            )
        })?;
        if frames > self.limits.max_frames() {
            return Err(FrameArtifactError::new(
                FrameArtifactErrorKind::FrameLimit,
                &self.output,
                "frame artifact frame count exceeds the configured limit",
            ));
        }

        let frame_bytes = u64::try_from(bytes.len()).map_err(|_| {
            FrameArtifactError::new(
                FrameArtifactErrorKind::ByteLimit,
                &self.output,
                "frame artifact frame size exceeds its accounting domain",
            )
        })?;
        if bytes.len() > self.limits.max_frame_bytes() {
            return Err(FrameArtifactError::new(
                FrameArtifactErrorKind::FrameByteLimit,
                &self.output,
                "frame artifact PNG exceeds the configured per-frame byte limit",
            ));
        }

        let record_bytes = FRAME_LENGTH_BYTES
            .checked_add(frame_bytes)
            .and_then(|bytes| bytes.checked_add(RAW_RGBA_HASH_BYTES))
            .ok_or_else(|| {
                FrameArtifactError::new(
                    FrameArtifactErrorKind::ByteLimit,
                    &self.output,
                    "frame artifact record size exceeds its accounting domain",
                )
            })?;
        let payload_bytes = self
            .payload_bytes
            .checked_add(record_bytes)
            .ok_or_else(|| {
                FrameArtifactError::new(
                    FrameArtifactErrorKind::ByteLimit,
                    &self.output,
                    "frame artifact payload exceeds its accounting domain",
                )
            })?;
        if payload_bytes > self.limits.max_bytes() {
            return Err(FrameArtifactError::new(
                FrameArtifactErrorKind::ByteLimit,
                &self.output,
                "frame artifact payload exceeds the configured byte limit",
            ));
        }

        Ok(NextRecord {
            frames,
            frame_bytes,
            payload_bytes,
        })
    }
}

fn encode_rgba(
    pixels: &[u8],
    profile: RenderProfile,
    output: &Path,
) -> Result<EncodedPng, FrameArtifactError> {
    let mut bytes = Vec::new();
    let mut encoder = Encoder::new(&mut bytes, profile.width(), profile.height());
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|source| {
        FrameArtifactError::new(
            FrameArtifactErrorKind::Output,
            output,
            format!("failed to start native frame PNG encoding: {source}"),
        )
    })?;
    writer.write_image_data(pixels).map_err(|source| {
        FrameArtifactError::new(
            FrameArtifactErrorKind::Output,
            output,
            format!("failed to encode native frame PNG: {source}"),
        )
    })?;
    drop(writer);
    Ok(EncodedPng::new(bytes))
}

async fn write_all(
    file: &mut File,
    output: &Path,
    bytes: &[u8],
    message: &'static str,
) -> Result<(), FrameArtifactError> {
    file.write_all(bytes).await.map_err(|source| {
        FrameArtifactError::io(FrameArtifactErrorKind::Output, output, message, source)
    })
}

/// Record metadata computed before any bytes become part of the artifact.
struct NextRecord {
    frames: u64,
    frame_bytes: u64,
    payload_bytes: u64,
}

fn output_parent(output: &Path) -> &Path {
    output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn publish(staging: TempPath, output: &Path) -> Result<(), FrameArtifactError> {
    std::fs::hard_link(staging, output).map_err(|source| {
        let kind = if output.exists() {
            FrameArtifactErrorKind::OutputExists
        } else {
            FrameArtifactErrorKind::Output
        };
        FrameArtifactError::io(kind, output, "failed to publish frame artifact", source)
    })?;

    // The link above is the publication point. TempPath owns best-effort
    // staging cleanup on drop, so cleanup failure cannot turn a published,
    // verified artifact into a reported capture failure.
    Ok(())
}
