use std::io;
use std::path::Path;

use onmark_core::model::{FrameIndex, FrameInterval, FrameRate};
use onmark_core::protocol::BrowserPlan;
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use super::{FrameArtifactError, FrameArtifactErrorKind, FrameArtifactLimits};
use crate::{CaptureEnvironmentId, ExecutableUnit, RawRgbaHash, RenderProfile};

pub(super) const HEADER_BYTES: usize = 156;
pub(super) const FRAME_LENGTH_BYTES: u64 = 8;
pub(super) const RAW_RGBA_HASH_BYTES: u64 = RawRgbaHash::BYTE_LENGTH as u64;
pub(super) const MIN_FRAME_RECORD_BYTES: u64 = FRAME_LENGTH_BYTES + 1 + RAW_RGBA_HASH_BYTES;

const MAGIC: [u8; 8] = *b"ONMARKF1";
// The magic identifies the frame-artifact family. V3 adds a capture
// environment identity to the fixed header, so artifacts cannot cross a
// browser/font deployment merely because their visual plan matches.
const VERSION: u16 = 3;

/// Immutable input facts that determine one visual frame artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FrameArtifactDescriptor {
    pub(super) output: FrameInterval,
    pub(super) frame_rate: FrameRate,
    pub(super) profile: RenderProfile,
    pub(super) capture_environment: CaptureEnvironmentId,
    pub(super) visual_plan_digest: [u8; 32],
}

impl FrameArtifactDescriptor {
    pub(super) fn from_unit(
        unit: &ExecutableUnit,
        capture_environment: CaptureEnvironmentId,
    ) -> Self {
        let plan = unit.browser_plan();
        let output = frame_interval(plan.output());
        let frame_rate = frame_rate(plan);
        let profile = unit.profile();
        let visual_plan_digest = visual_plan_digest(plan, unit.bundle_id(), profile);

        Self {
            output,
            frame_rate,
            profile,
            capture_environment,
            visual_plan_digest,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnitIdentity<'a> {
    // Declaration order is the compact-JSON identity contract.
    version: u16,
    bundle_id: &'a str,
    width: u32,
    height: u32,
    browser_plan: &'a BrowserPlan,
}

fn visual_plan_digest(plan: &BrowserPlan, bundle_id: &str, profile: RenderProfile) -> [u8; 32] {
    let identity = UnitIdentity {
        version: VERSION,
        bundle_id,
        width: profile.width(),
        height: profile.height(),
        browser_plan: plan,
    };
    let mut writer = DigestWriter(Sha256::new());
    serde_json::to_writer(&mut writer, &identity)
        .expect("a validated render unit always serializes into its digest writer");
    writer.0.finalize().into()
}

struct DigestWriter(Sha256);

impl io::Write for DigestWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Fixed-size envelope preceding every length-prefixed frame payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Header {
    pub(super) descriptor: FrameArtifactDescriptor,
    pub(super) frames: u64,
    pub(super) payload_bytes: u64,
    pub(super) digest: [u8; 32],
}

impl Header {
    pub(super) fn encode(&self) -> [u8; HEADER_BYTES] {
        let mut bytes = [0; HEADER_BYTES];
        let mut cursor = 0;

        write_bytes(&mut bytes, &mut cursor, &MAGIC);
        write_bytes(&mut bytes, &mut cursor, &VERSION.to_be_bytes());
        write_bytes(&mut bytes, &mut cursor, &[0, 0]);
        write_bytes(
            &mut bytes,
            &mut cursor,
            &self.descriptor.output.start().get().to_be_bytes(),
        );
        write_bytes(
            &mut bytes,
            &mut cursor,
            &self.descriptor.output.end().get().to_be_bytes(),
        );
        write_bytes(
            &mut bytes,
            &mut cursor,
            &self.descriptor.frame_rate.numerator().to_be_bytes(),
        );
        write_bytes(
            &mut bytes,
            &mut cursor,
            &self.descriptor.frame_rate.denominator().to_be_bytes(),
        );
        write_bytes(
            &mut bytes,
            &mut cursor,
            &self.descriptor.profile.width().to_be_bytes(),
        );
        write_bytes(
            &mut bytes,
            &mut cursor,
            &self.descriptor.profile.height().to_be_bytes(),
        );
        write_bytes(
            &mut bytes,
            &mut cursor,
            self.descriptor.capture_environment.as_sha256(),
        );
        write_bytes(&mut bytes, &mut cursor, &self.descriptor.visual_plan_digest);
        write_bytes(&mut bytes, &mut cursor, &self.frames.to_be_bytes());
        write_bytes(&mut bytes, &mut cursor, &self.payload_bytes.to_be_bytes());
        write_bytes(&mut bytes, &mut cursor, &self.digest);

        debug_assert_eq!(cursor, HEADER_BYTES);
        bytes
    }

    pub(super) fn decode(
        path: &Path,
        bytes: [u8; HEADER_BYTES],
    ) -> Result<Self, FrameArtifactError> {
        let mut cursor = 0;
        if read_array::<8>(&bytes, &mut cursor) != MAGIC {
            return Err(FrameArtifactError::invalid(
                path,
                "frame artifact magic is invalid",
            ));
        }
        if read_u16(&bytes, &mut cursor) != VERSION {
            return Err(FrameArtifactError::invalid(
                path,
                "frame artifact version is unsupported",
            ));
        }
        if read_array::<2>(&bytes, &mut cursor) != [0, 0] {
            return Err(FrameArtifactError::invalid(
                path,
                "frame artifact reserved header bytes are nonzero",
            ));
        }

        let output = FrameInterval::new(
            FrameIndex::new(read_u64(&bytes, &mut cursor)),
            FrameIndex::new(read_u64(&bytes, &mut cursor)),
        )
        .map_err(|_| {
            FrameArtifactError::invalid(path, "frame artifact output interval is reversed")
        })?;
        let numerator = read_u32(&bytes, &mut cursor);
        let denominator = read_u32(&bytes, &mut cursor);
        let frame_rate = FrameRate::new(numerator, denominator).map_err(|_| {
            FrameArtifactError::invalid(path, "frame artifact frame rate is invalid")
        })?;
        if frame_rate.numerator() != numerator || frame_rate.denominator() != denominator {
            return Err(FrameArtifactError::invalid(
                path,
                "frame artifact frame rate is not canonical",
            ));
        }
        let profile =
            RenderProfile::new(read_u32(&bytes, &mut cursor), read_u32(&bytes, &mut cursor))
                .map_err(|_| {
                    FrameArtifactError::invalid(path, "frame artifact render profile is invalid")
                })?;
        let capture_environment =
            CaptureEnvironmentId::from_sha256(read_array(&bytes, &mut cursor));
        let visual_plan_digest = read_array::<32>(&bytes, &mut cursor);
        let frames = read_u64(&bytes, &mut cursor);
        let payload_bytes = read_u64(&bytes, &mut cursor);
        let digest = read_array::<32>(&bytes, &mut cursor);

        debug_assert_eq!(cursor, HEADER_BYTES);
        Ok(Self {
            descriptor: FrameArtifactDescriptor {
                output,
                frame_rate,
                profile,
                capture_environment,
                visual_plan_digest,
            },
            frames,
            payload_bytes,
            digest,
        })
    }

    pub(super) fn validate(
        &self,
        path: &Path,
        limits: FrameArtifactLimits,
    ) -> Result<(), FrameArtifactError> {
        let expected_frames = self.descriptor.output.len().get();
        if self.frames == 0 || self.frames != expected_frames {
            return Err(FrameArtifactError::invalid(
                path,
                "frame artifact frame count does not match its output interval",
            ));
        }
        if self.frames > limits.max_frames() {
            return Err(FrameArtifactError::new(
                FrameArtifactErrorKind::FrameLimit,
                path,
                "frame artifact frame count exceeds the configured limit",
            ));
        }
        if self.payload_bytes > limits.max_bytes() {
            return Err(FrameArtifactError::new(
                FrameArtifactErrorKind::ByteLimit,
                path,
                "frame artifact payload exceeds the configured byte limit",
            ));
        }
        let minimum_payload_bytes =
            self.frames
                .checked_mul(MIN_FRAME_RECORD_BYTES)
                .ok_or_else(|| {
                    FrameArtifactError::invalid(
                        path,
                        "frame artifact payload exceeds its accounting domain",
                    )
                })?;
        if self.payload_bytes < minimum_payload_bytes {
            return Err(FrameArtifactError::invalid(
                path,
                "frame artifact payload cannot contain every declared frame",
            ));
        }
        Ok(())
    }
}

fn frame_interval(interval: onmark_core::protocol::WireInterval) -> FrameInterval {
    FrameInterval::new(
        FrameIndex::new(interval.start().get()),
        FrameIndex::new(interval.end().get()),
    )
    .expect("a browser plan retains an ordered output interval")
}

fn frame_rate(plan: &BrowserPlan) -> FrameRate {
    let rate = plan.frame_rate();
    FrameRate::new(rate.numerator(), rate.denominator())
        .expect("a browser plan retains a validated canonical frame rate")
}

fn write_bytes(target: &mut [u8], cursor: &mut usize, source: &[u8]) {
    let end = cursor
        .checked_add(source.len())
        .expect("the fixed frame artifact header cannot overflow usize");
    target[*cursor..end].copy_from_slice(source);
    *cursor = end;
}

fn read_array<const N: usize>(source: &[u8], cursor: &mut usize) -> [u8; N] {
    let end = cursor
        .checked_add(N)
        .expect("the fixed frame artifact header cannot overflow usize");
    let bytes = source[*cursor..end]
        .try_into()
        .expect("the fixed frame artifact header contains every requested field");
    *cursor = end;
    bytes
}

fn read_u16(source: &[u8], cursor: &mut usize) -> u16 {
    u16::from_be_bytes(read_array(source, cursor))
}

fn read_u32(source: &[u8], cursor: &mut usize) -> u32 {
    u32::from_be_bytes(read_array(source, cursor))
}

fn read_u64(source: &[u8], cursor: &mut usize) -> u64 {
    u64::from_be_bytes(read_array(source, cursor))
}
