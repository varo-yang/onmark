use std::io::Cursor;

use png::{BitDepth, ColorType, Decoder, Limits, Transformations};
use sha2::{Digest as _, Sha256};

use super::BrowserError;
use crate::RenderProfile;

const RGBA_CHANNELS: usize = 4;

/// One PNG screenshot retained for the encoder boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedPng(Vec<u8>);

impl EncodedPng {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Returns the encoded PNG bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Transfers ownership of the encoded PNG bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
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
    pub(super) fn from_png(png: EncodedPng, profile: RenderProfile) -> Result<Self, BrowserError> {
        let raw_rgba_hash = raw_rgba_hash(&png, profile)?;
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

fn raw_rgba_hash(png: &EncodedPng, profile: RenderProfile) -> Result<RawRgbaHash, BrowserError> {
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
    let pixels = &output[..info.buffer_size()];
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

    let hash = match info.color_type {
        ColorType::Rgba => hash_rgba(pixels, expected)?,
        ColorType::Rgb => hash_rgb_as_rgba(pixels, expected)?,
        _ => {
            return Err(BrowserError::capture_pixels(
                "captured PNG does not decode to RGB or RGBA pixels",
            ));
        }
    };
    Ok(RawRgbaHash::from_bytes(hash))
}

fn expected_rgba_bytes(profile: RenderProfile) -> Result<usize, BrowserError> {
    let pixels = usize::try_from(profile.width())
        .ok()
        .and_then(|width| width.checked_mul(usize::try_from(profile.height()).ok()?))
        .ok_or_else(|| BrowserError::capture_pixels("render profile exceeds pixel accounting"))?;
    pixels
        .checked_mul(RGBA_CHANNELS)
        .ok_or_else(|| BrowserError::capture_pixels("render profile exceeds RGBA accounting"))
}

fn hash_rgba(pixels: &[u8], expected: usize) -> Result<[u8; 32], BrowserError> {
    if pixels.len() != expected {
        return Err(BrowserError::capture_pixels(
            "captured RGBA PNG has an unexpected pixel length",
        ));
    }
    Ok(Sha256::digest(pixels).into())
}

fn hash_rgb_as_rgba(pixels: &[u8], expected: usize) -> Result<[u8; 32], BrowserError> {
    let rgb_bytes = expected / RGBA_CHANNELS * 3;
    if pixels.len() != rgb_bytes {
        return Err(BrowserError::capture_pixels(
            "captured RGB PNG has an unexpected pixel length",
        ));
    }

    let mut hasher = Sha256::new();
    for pixel in pixels.chunks_exact(3) {
        hasher.update(pixel);
        hasher.update([u8::MAX]);
    }
    Ok(hasher.finalize().into())
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

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
