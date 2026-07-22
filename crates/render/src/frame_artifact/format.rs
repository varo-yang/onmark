//! Canonical binary layout and identity derivation for worker frame artifacts.
//!
//! Encoding and decoding share one fixed field order. Checksums protect stored
//! bytes; raw-RGBA fingerprints separately prove visual equivalence.

use std::fmt;
use std::io;
use std::path::Path;

use onmark_core::model::{FrameIndex, FrameInterval, FrameRate};
use onmark_core::protocol::BrowserPlan;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as _, Sha256};

use super::{FrameArtifactError, FrameArtifactErrorKind, FrameArtifactLimits};
use crate::{CaptureEnvironmentId, ExecutableUnit, RawRgbaHash, RenderProfile};

pub(super) const HEADER_BYTES: usize = 156;
pub(super) const FRAME_LENGTH_BYTES: u64 = 8;
pub(super) const RAW_RGBA_HASH_BYTES: u64 = RawRgbaHash::BYTE_LENGTH as u64;
pub(super) const MIN_FRAME_RECORD_BYTES: u64 = FRAME_LENGTH_BYTES + 1 + RAW_RGBA_HASH_BYTES;

const MAGIC: [u8; 8] = *b"ONMARKF1";
const VERSION: u16 = 1;
const ID_DOMAIN: &[u8] = b"onmark-frame-artifact-id\0";

/// Deterministic identity of one frame-artifact capture contract.
///
/// This identifies the immutable visual inputs and locked capture environment,
/// not the encoded PNG payload. Deployments use it to choose an object key;
/// the artifact's internal checksum still verifies the published bytes.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FrameArtifactId(
    #[cfg_attr(
        feature = "schema",
        schemars(with = "String", regex(pattern = r"^sha256:[0-9a-f]{64}$"))
    )]
    [u8; 32],
);

impl FrameArtifactId {
    const SHA256_BYTES: usize = 32;

    /// Parses the canonical `sha256:<lowercase-hex>` spelling.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidFrameArtifactId`] when the prefix, digest length, or
    /// lowercase hexadecimal spelling is not canonical.
    pub fn parse(value: &str) -> Result<Self, InvalidFrameArtifactId> {
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(InvalidFrameArtifactId::MissingPrefix);
        };
        if hex.len() != Self::SHA256_BYTES * 2 {
            return Err(InvalidFrameArtifactId::InvalidLength);
        }

        let mut digest = [0; Self::SHA256_BYTES];
        for (index, byte) in digest.iter_mut().enumerate() {
            let offset = index * 2;
            let high = hex_value(hex.as_bytes()[offset])?;
            let low = hex_value(hex.as_bytes()[offset + 1])?;
            *byte = high << 4 | low;
        }
        Ok(Self(digest))
    }

    /// Returns the canonical SHA-256 digest bytes.
    #[must_use]
    pub const fn as_sha256(&self) -> &[u8; 32] {
        &self.0
    }

    pub(crate) fn from_facts(
        plan: &BrowserPlan,
        bundle_id: &str,
        profile: RenderProfile,
        capture_environment: CaptureEnvironmentId,
    ) -> Self {
        FrameArtifactDescriptor::from_facts(plan, bundle_id, profile, capture_environment).id()
    }

    fn from_descriptor(descriptor: &FrameArtifactDescriptor) -> Self {
        let mut digest = Sha256::new();
        digest.update(ID_DOMAIN);
        digest.update(VERSION.to_be_bytes());
        digest.update(descriptor.capture_environment.as_sha256());
        digest.update(descriptor.visual_plan_digest);
        Self(digest.finalize().into())
    }
}

impl fmt::Display for FrameArtifactId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sha256:")?;
        for byte in self.as_sha256() {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for FrameArtifactId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for FrameArtifactId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(D::Error::custom)
    }
}

/// Reason a frame-artifact identity spelling cannot name one capture contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidFrameArtifactId {
    /// The required `sha256:` prefix is absent.
    MissingPrefix,
    /// The SHA-256 digest does not have exactly 64 hexadecimal characters.
    InvalidLength,
    /// The digest contains a noncanonical hexadecimal byte.
    InvalidHex,
}

impl fmt::Display for InvalidFrameArtifactId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingPrefix => "frame artifact identity must start with sha256:",
            Self::InvalidLength => "frame artifact identity must contain 64 hexadecimal characters",
            Self::InvalidHex => "frame artifact identity must use lowercase hexadecimal characters",
        })
    }
}

impl std::error::Error for InvalidFrameArtifactId {}

fn hex_value(byte: u8) -> Result<u8, InvalidFrameArtifactId> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(InvalidFrameArtifactId::InvalidHex),
    }
}

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
        Self::from_facts(
            unit.browser_plan(),
            unit.bundle_id(),
            unit.profile(),
            capture_environment,
        )
    }

    fn from_facts(
        plan: &BrowserPlan,
        bundle_id: &str,
        profile: RenderProfile,
        capture_environment: CaptureEnvironmentId,
    ) -> Self {
        let output = frame_interval(plan.output());
        let frame_rate = frame_rate(plan);
        let visual_plan_digest = visual_plan_digest(plan, bundle_id, profile);

        Self {
            output,
            frame_rate,
            profile,
            capture_environment,
            visual_plan_digest,
        }
    }

    pub(super) fn id(&self) -> FrameArtifactId {
        FrameArtifactId::from_descriptor(self)
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
        let mut header = HeaderEncoder::new();
        header.bytes(MAGIC);
        header.u16(VERSION);
        header.bytes([0, 0]);
        header.u64(self.descriptor.output.start().get());
        header.u64(self.descriptor.output.end().get());
        header.u32(self.descriptor.frame_rate.numerator());
        header.u32(self.descriptor.frame_rate.denominator());
        header.u32(self.descriptor.profile.width());
        header.u32(self.descriptor.profile.height());
        header.bytes(*self.descriptor.capture_environment.as_sha256());
        header.bytes(self.descriptor.visual_plan_digest);
        header.u64(self.frames);
        header.u64(self.payload_bytes);
        header.bytes(self.digest);
        header.finish()
    }

    pub(super) fn decode(
        path: &Path,
        bytes: [u8; HEADER_BYTES],
    ) -> Result<Self, FrameArtifactError> {
        let mut header = HeaderDecoder::new(bytes);
        if header.bytes::<8>() != MAGIC {
            return Err(FrameArtifactError::invalid(
                path,
                "frame artifact magic is invalid",
            ));
        }
        if header.u16() != VERSION {
            return Err(FrameArtifactError::invalid(
                path,
                "frame artifact version is unsupported",
            ));
        }
        if header.bytes::<2>() != [0, 0] {
            return Err(FrameArtifactError::invalid(
                path,
                "frame artifact reserved header bytes are nonzero",
            ));
        }

        let output =
            FrameInterval::new(FrameIndex::new(header.u64()), FrameIndex::new(header.u64()))
                .map_err(|_| {
                    FrameArtifactError::invalid(path, "frame artifact output interval is reversed")
                })?;
        let numerator = header.u32();
        let denominator = header.u32();
        let frame_rate = FrameRate::new(numerator, denominator).map_err(|_| {
            FrameArtifactError::invalid(path, "frame artifact frame rate is invalid")
        })?;
        if frame_rate.numerator() != numerator || frame_rate.denominator() != denominator {
            return Err(FrameArtifactError::invalid(
                path,
                "frame artifact frame rate is not canonical",
            ));
        }
        let profile = RenderProfile::new(header.u32(), header.u32()).map_err(|_| {
            FrameArtifactError::invalid(path, "frame artifact render profile is invalid")
        })?;
        let capture_environment = CaptureEnvironmentId::from_sha256(header.bytes());
        let visual_plan_digest = header.bytes();
        let frames = header.u64();
        let payload_bytes = header.u64();
        let digest = header.bytes();
        header.finish();
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

/// Forward-only writer whose final cursor proves the fixed layout stayed whole.
struct HeaderEncoder {
    bytes: [u8; HEADER_BYTES],
    cursor: usize,
}

impl HeaderEncoder {
    const fn new() -> Self {
        Self {
            bytes: [0; HEADER_BYTES],
            cursor: 0,
        }
    }

    fn bytes<const N: usize>(&mut self, value: [u8; N]) {
        let end = self
            .cursor
            .checked_add(N)
            .expect("the fixed frame artifact header cannot overflow usize");
        self.bytes[self.cursor..end].copy_from_slice(&value);
        self.cursor = end;
    }

    fn u16(&mut self, value: u16) {
        self.bytes(value.to_be_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.bytes(value.to_be_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes(value.to_be_bytes());
    }

    fn finish(self) -> [u8; HEADER_BYTES] {
        debug_assert_eq!(self.cursor, HEADER_BYTES);
        self.bytes
    }
}

/// Mirror of [`HeaderEncoder`] for one already length-checked header.
struct HeaderDecoder {
    bytes: [u8; HEADER_BYTES],
    cursor: usize,
}

impl HeaderDecoder {
    const fn new(bytes: [u8; HEADER_BYTES]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn bytes<const N: usize>(&mut self) -> [u8; N] {
        let end = self
            .cursor
            .checked_add(N)
            .expect("the fixed frame artifact header cannot overflow usize");
        let value = self.bytes[self.cursor..end]
            .try_into()
            .expect("the fixed frame artifact header contains every requested field");
        self.cursor = end;
        value
    }

    fn u16(&mut self) -> u16 {
        u16::from_be_bytes(self.bytes())
    }

    fn u32(&mut self) -> u32 {
        u32::from_be_bytes(self.bytes())
    }

    fn u64(&mut self) -> u64 {
        u64::from_be_bytes(self.bytes())
    }

    fn finish(self) {
        debug_assert_eq!(self.cursor, HEADER_BYTES);
    }
}
