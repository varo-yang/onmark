//! Captured PNG validation and raw-RGBA visual fingerprinting.
//!
//! Encoded bytes prove artifact integrity; decoded pixels prove equivalence
//! across independently captured or compressed artifacts.

use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use png::{BitDepth, ColorType, Decoder, Limits, Transformations};
use sha2::{Digest as _, Sha256};

use super::BrowserError;
use crate::RenderProfile;

const RGBA_CHANNELS: usize = 4;

// Successful normalization is immutable and shared by every payload clone.
#[derive(Debug)]
struct EncodedPngPayload {
    bytes: Vec<u8>,
    decoded: OnceLock<DecodedPng>,
}

/// One immutable PNG screenshot retained across capture and encoder boundaries.
///
/// Chromium may omit pixels when the compositor reports no damage. Clones
/// therefore share both encoded and successfully normalized pixels, so reusing
/// the preceding frame never copies or decodes a viewport-sized allocation.
#[derive(Clone, Debug)]
pub struct EncodedPng(Arc<EncodedPngPayload>);

impl EncodedPng {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self(Arc::new(EncodedPngPayload {
            bytes,
            decoded: OnceLock::new(),
        }))
    }

    /// Returns the encoded PNG bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.0.bytes.as_slice()
    }

    /// Transfers ownership of the encoded PNG bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        match Arc::try_unwrap(self.0) {
            Ok(payload) => payload.bytes,
            Err(payload) => payload.bytes.clone(),
        }
    }

    pub(crate) fn decode_rgba(&self, profile: RenderProfile) -> Result<DecodedRgba, BrowserError> {
        if let Some(pixels) = self.cached_pixels(profile) {
            return Ok(pixels);
        }

        let pixels = decode_png(self, profile)?;
        let decoded = DecodedPng {
            profile,
            pixels: pixels.clone(),
        };
        if self.0.decoded.set(decoded).is_ok() {
            return Ok(pixels);
        }
        // A concurrent decode may have won the immutable cache. Reuse it only
        // when it proved the same profile; otherwise retain this exact result.
        Ok(self.cached_pixels(profile).unwrap_or(pixels))
    }

    fn cached_pixels(&self, profile: RenderProfile) -> Option<DecodedRgba> {
        self.0
            .decoded
            .get()
            .filter(|decoded| decoded.profile == profile)
            .map(|decoded| decoded.pixels.clone())
    }
}

impl PartialEq for EncodedPng {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for EncodedPng {}

#[derive(Debug)]
struct DecodedPng {
    profile: RenderProfile,
    pixels: DecodedRgba,
}

/// One profile-sized browser frame normalized for native pixel composition.
#[derive(Clone, Debug)]
pub(crate) struct DecodedRgba {
    bytes: Arc<[u8]>,
}

impl DecodedRgba {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn fingerprint(&self) -> RawRgbaHash {
        RawRgbaHash::from_bytes(Sha256::digest(&self.bytes).into())
    }
}

/// SHA-256 of one canonical, exact 8-bit RGBA browser frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RawRgbaHash([u8; Self::BYTE_LENGTH]);

impl RawRgbaHash {
    /// Number of SHA-256 digest bytes in one frame fingerprint.
    pub const BYTE_LENGTH: usize = 32;

    pub(crate) const fn from_bytes(bytes: [u8; Self::BYTE_LENGTH]) -> Self {
        Self(bytes)
    }

    /// Returns the fixed SHA-256 digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; Self::BYTE_LENGTH] {
        &self.0
    }
}

/// One browser capture with its encoder payload and canonical pixel evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedFrame {
    png: EncodedPng,
    raw_rgba_hash: RawRgbaHash,
}

impl CapturedFrame {
    pub(crate) fn from_png(png: EncodedPng, profile: RenderProfile) -> Result<Self, BrowserError> {
        let raw_rgba_hash = png.decode_rgba(profile)?.fingerprint();
        Ok(Self { png, raw_rgba_hash })
    }

    pub(crate) const fn recorded(png: EncodedPng, raw_rgba_hash: RawRgbaHash) -> Self {
        Self { png, raw_rgba_hash }
    }

    /// Returns the PNG bytes passed unchanged into the visual encoder.
    #[must_use]
    pub const fn png(&self) -> &EncodedPng {
        &self.png
    }

    /// Returns the fingerprint of the canonical decoded RGBA pixels.
    #[must_use]
    pub const fn raw_rgba_hash(&self) -> RawRgbaHash {
        self.raw_rgba_hash
    }
}

fn decode_png(png: &EncodedPng, profile: RenderProfile) -> Result<DecodedRgba, BrowserError> {
    let expected = expected_rgba_bytes(profile)?;
    // The profile has already bounded a frame. Give the decoder that same
    // budget before it sees untrusted compressed pixels.
    let mut png_decoder =
        Decoder::new_with_limits(Cursor::new(png.as_bytes()), Limits { bytes: expected });
    png_decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut reader = png_decoder
        .read_info()
        .map_err(|source| BrowserError::png("failed to decode captured PNG", source))?;
    if reader.info().width != profile.width() || reader.info().height != profile.height() {
        return Err(BrowserError::capture_pixels(
            "captured PNG dimensions do not match the render profile",
        ));
    }
    let output_bytes = reader.output_buffer_size().ok_or_else(|| {
        BrowserError::capture_pixels("captured PNG does not declare a bounded output size")
    })?;
    if output_bytes > expected {
        return Err(BrowserError::capture_pixels(
            "captured PNG exceeds the render profile's RGBA memory bound",
        ));
    }
    let mut output = vec![0; output_bytes];
    let info = reader
        .next_frame(&mut output)
        .map_err(|source| BrowserError::png("failed to read captured PNG pixels", source))?;
    output.truncate(info.buffer_size());
    // APNG may expose a subframe even when its image header matched the
    // profile.
    if info.width != profile.width() || info.height != profile.height() {
        return Err(BrowserError::capture_pixels(
            "captured PNG dimensions do not match the render profile",
        ));
    }
    if info.bit_depth != BitDepth::Eight {
        return Err(BrowserError::capture_pixels(
            "captured PNG does not decode to eight-bit pixels",
        ));
    }

    let bytes = match info.color_type {
        ColorType::Rgba => checked_rgba(output, expected)?,
        ColorType::Rgb => rgb_to_rgba(&output, expected)?,
        _ => {
            return Err(BrowserError::capture_pixels(
                "captured PNG does not decode to RGB or RGBA pixels",
            ));
        }
    };
    Ok(DecodedRgba { bytes })
}

fn expected_rgba_bytes(profile: RenderProfile) -> Result<usize, BrowserError> {
    let width = usize::try_from(profile.width())
        .map_err(|_| BrowserError::capture_pixels("render profile exceeds pixel accounting"))?;
    let height = usize::try_from(profile.height())
        .map_err(|_| BrowserError::capture_pixels("render profile exceeds pixel accounting"))?;
    let pixels = width
        .checked_mul(height)
        .ok_or_else(|| BrowserError::capture_pixels("render profile exceeds pixel accounting"))?;
    pixels
        .checked_mul(RGBA_CHANNELS)
        .ok_or_else(|| BrowserError::capture_pixels("render profile exceeds RGBA accounting"))
}

fn checked_rgba(pixels: Vec<u8>, expected: usize) -> Result<Arc<[u8]>, BrowserError> {
    if pixels.len() != expected {
        return Err(BrowserError::capture_pixels(
            "captured RGBA PNG has an unexpected pixel length",
        ));
    }
    Ok(pixels.into())
}

fn rgb_to_rgba(pixels: &[u8], expected: usize) -> Result<Arc<[u8]>, BrowserError> {
    let rgb_bytes = expected / RGBA_CHANNELS * 3;
    if pixels.len() != rgb_bytes {
        return Err(BrowserError::capture_pixels(
            "captured RGB PNG has an unexpected pixel length",
        ));
    }

    let mut rgba = Vec::with_capacity(expected);
    for pixel in pixels.chunks_exact(3) {
        rgba.extend_from_slice(pixel);
        rgba.push(u8::MAX);
    }
    Ok(rgba.into())
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::sync::Arc;

    use super::{CapturedFrame, EncodedPng};
    use crate::RenderProfile;
    use sha2::Digest as _;

    #[test]
    fn hashes_the_decoded_rgba_pixels() {
        let pixels = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let frame = CapturedFrame::from_png(
            encoded_png(png::ColorType::Rgba, 2, 2, &pixels),
            RenderProfile::new(2, 2).expect("the two-by-two profile is valid"),
        )
        .expect("the canonical PNG decodes");

        let expected: [u8; super::RawRgbaHash::BYTE_LENGTH] = sha2::Sha256::digest(pixels).into();
        assert_eq!(frame.raw_rgba_hash().as_bytes(), &expected);
    }

    #[test]
    fn normalizes_rgb_to_opaque_rgba() {
        let rgb = CapturedFrame::from_png(
            encoded_png(
                png::ColorType::Rgb,
                2,
                2,
                &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
            ),
            RenderProfile::new(2, 2).expect("the two-by-two profile is valid"),
        )
        .expect("the canonical PNG decodes");
        let rgba = CapturedFrame::from_png(
            encoded_png(
                png::ColorType::Rgba,
                2,
                2,
                &[
                    1,
                    2,
                    3,
                    u8::MAX,
                    4,
                    5,
                    6,
                    u8::MAX,
                    7,
                    8,
                    9,
                    u8::MAX,
                    10,
                    11,
                    12,
                    u8::MAX,
                ],
            ),
            RenderProfile::new(2, 2).expect("the two-by-two profile is valid"),
        )
        .expect("the canonical PNG decodes");

        assert_eq!(rgb.raw_rgba_hash(), rgba.raw_rgba_hash());
    }

    #[test]
    fn exposes_normalized_rgba_bytes_to_native_composition() {
        let profile = RenderProfile::new(2, 2).expect("the four-pixel profile is valid");
        let rgb = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let expected = [
            1,
            2,
            3,
            u8::MAX,
            4,
            5,
            6,
            u8::MAX,
            7,
            8,
            9,
            u8::MAX,
            10,
            11,
            12,
            u8::MAX,
        ];
        let png = encoded_png(png::ColorType::Rgb, 2, 2, &rgb);

        let decoded = png
            .decode_rgba(profile)
            .expect("the browser pixels normalize to RGBA");

        assert_eq!(decoded.as_bytes(), &expected);
    }

    #[test]
    fn rejects_a_png_outside_the_render_profile() {
        let error = CapturedFrame::from_png(
            encoded_png(png::ColorType::Rgba, 4, 4, &[0; 64]),
            RenderProfile::new(2, 2).expect("the two-by-two profile is valid"),
        )
        .expect_err("a mismatched viewport must not yield a fingerprint");

        assert_eq!(error.kind(), super::super::BrowserErrorKind::Capture);
    }

    #[test]
    fn retains_the_png_decoder_failure_as_a_capture_source() {
        let error = CapturedFrame::from_png(
            EncodedPng::new(vec![0]),
            RenderProfile::new(2, 2).expect("the two-by-two profile is valid"),
        )
        .expect_err("invalid PNG bytes must not yield a fingerprint");

        assert_eq!(error.kind(), super::super::BrowserErrorKind::Capture);
        assert!(error.source().is_some());
    }

    #[test]
    fn cloning_a_png_does_not_copy_its_frame_payload() {
        let original = EncodedPng::new(vec![1, 2, 3, 4]);
        let cloned = original.clone();

        assert!(Arc::ptr_eq(&original.0, &cloned.0));
    }

    #[test]
    fn cloned_pngs_share_one_successful_rgba_decode() {
        let profile = RenderProfile::new(2, 2).expect("the two-by-two profile is valid");
        let original = encoded_png(png::ColorType::Rgba, 2, 2, &[7; 16]);
        let cloned = original.clone();

        let first = original
            .decode_rgba(profile)
            .expect("the original PNG decodes");
        let second = cloned
            .decode_rgba(profile)
            .expect("the cloned PNG reuses decoded pixels");

        assert!(Arc::ptr_eq(&first.bytes, &second.bytes));
    }

    fn encoded_png(color: png::ColorType, width: u32, height: u32, pixels: &[u8]) -> EncodedPng {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(color);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder
                .write_header()
                .expect("the in-memory PNG header is writable");
            writer
                .write_image_data(pixels)
                .expect("the in-memory PNG pixels are writable");
        }
        EncodedPng::new(bytes)
    }
}
