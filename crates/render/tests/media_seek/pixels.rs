//! Streaming RGBA comparison for the decoded-media experiment.

use std::io::Write as _;

use onmark_render::EncodedPng;
use tempfile::NamedTempFile;
use tokio::process::Command;
use tokio::time::timeout;

use super::{PROCESS_DEADLINE, assert_process_succeeded, required_path};

#[derive(Clone, Copy, Debug)]
pub(super) struct PixelDifference {
    pub(super) channels: usize,
    pub(super) differing_channels: usize,
    pub(super) maximum_delta: u8,
    pub(super) mean_absolute_delta: f64,
}

#[derive(Default)]
struct PixelDifferenceAccumulator {
    channels: usize,
    differing_channels: usize,
    maximum_delta: u8,
    total_delta: u64,
}

impl PixelDifferenceAccumulator {
    fn observe(&mut self, browser: &[u8], native: &[u8]) {
        assert_eq!(browser.len(), native.len(), "RGBA frame domains must match");
        self.channels += native.len();
        for (&browser, &native) in browser.iter().zip(native) {
            let delta = browser.abs_diff(native);
            self.differing_channels += usize::from(delta != 0);
            self.maximum_delta = self.maximum_delta.max(delta);
            self.total_delta += u64::from(delta);
        }
    }

    fn finish(self) -> PixelDifference {
        assert!(
            self.channels > 0,
            "RGBA comparison requires at least one channel"
        );
        PixelDifference {
            channels: self.channels,
            differing_channels: self.differing_channels,
            maximum_delta: self.maximum_delta,
            mean_absolute_delta: self.total_delta as f64 / self.channels as f64,
        }
    }
}

pub(super) fn compare_pixels(browser: &[Vec<u8>], native: &[Vec<u8>]) -> PixelDifference {
    assert_eq!(browser.len(), native.len(), "RGBA sequences must align");
    let mut difference = PixelDifferenceAccumulator::default();
    for (browser, native) in browser.iter().zip(native) {
        difference.observe(browser, native);
    }
    difference.finish()
}

pub(super) fn composite_pixels(base: &[Vec<u8>], overlay: &[Vec<u8>]) -> Vec<Vec<u8>> {
    assert_eq!(base.len(), overlay.len(), "RGBA sequences must align");
    base.iter()
        .zip(overlay)
        .map(|(base, overlay)| composite_frame(base, overlay))
        .collect()
}

fn composite_frame(base: &[u8], overlay: &[u8]) -> Vec<u8> {
    assert_eq!(base.len(), overlay.len(), "RGBA frame domains must match");
    assert_eq!(base.len() % 4, 0, "RGBA frames must contain whole pixels");
    let mut output = Vec::with_capacity(base.len());
    for (base, overlay) in base.chunks_exact(4).zip(overlay.chunks_exact(4)) {
        let alpha = u32::from(overlay[3]);
        output.push(composite_channel(base[0], overlay[0], alpha));
        output.push(composite_channel(base[1], overlay[1], alpha));
        output.push(composite_channel(base[2], overlay[2], alpha));
        output.push(u8::MAX);
    }
    output
}

fn composite_channel(base: u8, overlay: u8, alpha: u32) -> u8 {
    let inverse = u32::from(u8::MAX) - alpha;
    let mixed = u32::from(overlay) * alpha + u32::from(base) * inverse;
    u8::try_from((mixed + 127) / u32::from(u8::MAX))
        .expect("straight-alpha composition remains an eight-bit channel")
}

pub(super) async fn compare_encoded_frames(
    browser: &[EncodedPng],
    native: &[EncodedPng],
) -> PixelDifference {
    assert_eq!(browser.len(), native.len(), "encoded sequences must align");
    let mut difference = PixelDifferenceAccumulator::default();
    for (browser, native) in browser.iter().zip(native) {
        let browser = decode_browser_frame(browser).await;
        let native = decode_browser_frame(native).await;
        difference.observe(&browser, &native);
    }
    difference.finish()
}

pub(super) async fn decode_browser_frames(frames: &[EncodedPng]) -> Vec<Vec<u8>> {
    let mut decoded = Vec::with_capacity(frames.len());
    for frame in frames {
        decoded.push(decode_browser_frame(frame).await);
    }
    decoded
}

async fn decode_browser_frame(frame: &EncodedPng) -> Vec<u8> {
    let mut encoded = NamedTempFile::new().expect("the PNG staging file must be available");
    encoded
        .write_all(frame.as_bytes())
        .expect("the captured PNG must fit its staging file");
    encoded.flush().expect("the staged PNG must be readable");

    let decoded = Command::new(required_path("ONMARK_FFMPEG"))
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(encoded.path())
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output();
    let decoded = timeout(PROCESS_DEADLINE, decoded)
        .await
        .expect("PNG decoding must finish before its deadline")
        .expect("FFmpeg must decode the browser capture");
    assert_process_succeeded("browser PNG decoding", &decoded);
    decoded.stdout
}
