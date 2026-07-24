//! Explicit resource policy shared by local rendering and worker capture.
//!
//! The renderer owns enforcement. The CLI selects concrete bounded values for
//! its local composition root and its Gate-three worker entry point.

use std::time::Duration;

use onmark_render::{BrowserLimits, EncodeLimits, FrameArtifactLimits, UnitRootLimits};

const PROCESS_DEADLINE: Duration = Duration::from_mins(2);
const ENCODER_INACTIVITY_TIMEOUT: Duration = Duration::from_mins(1);
const MAX_CAPTURE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENCODED_FRAMES: u64 = 1_000_000;
const MAX_ENCODER_INPUT_BYTES: u64 = 128 * 1024 * 1024 * 1024;
const MAX_PROCESS_STDERR_BYTES: usize = 1024 * 1024;
pub(super) const LOCAL_VIDEO_ENCODER_THREADS: usize = 4;
const WORKER_VIDEO_ENCODER_THREADS: usize = 1;
const MAX_UNIT_FILES: usize = 10_000;
const MAX_UNIT_BYTES: u64 = 256 * 1024 * 1024 * 1024;

pub(super) const fn process_deadline() -> Duration {
    PROCESS_DEADLINE
}

pub(super) fn browser_limits() -> BrowserLimits {
    BrowserLimits::new(PROCESS_DEADLINE, MAX_CAPTURE_BYTES)
        .expect("the CLI browser policy stays within the render safety envelope")
}

pub(super) fn local_encode_limits(video_encoder_threads: usize) -> EncodeLimits {
    encode_limits(video_encoder_threads)
}

pub(super) fn worker_encode_limits() -> EncodeLimits {
    encode_limits(WORKER_VIDEO_ENCODER_THREADS)
}

fn encode_limits(video_encoder_threads: usize) -> EncodeLimits {
    EncodeLimits::new(
        ENCODER_INACTIVITY_TIMEOUT,
        MAX_ENCODED_FRAMES,
        MAX_ENCODER_INPUT_BYTES,
        MAX_PROCESS_STDERR_BYTES,
    )
    .and_then(|limits| limits.with_video_encoder_threads(video_encoder_threads))
    .expect("the CLI encoder policy stays within the render safety envelope")
}

pub(super) fn frame_artifact_limits() -> FrameArtifactLimits {
    FrameArtifactLimits::new(
        MAX_ENCODED_FRAMES,
        MAX_ENCODER_INPUT_BYTES,
        MAX_CAPTURE_BYTES,
    )
    .expect("the CLI worker-artifact policy stays within the render safety envelope")
}

pub(super) fn unit_root_limits() -> UnitRootLimits {
    UnitRootLimits::new(MAX_UNIT_FILES, MAX_UNIT_BYTES)
        .expect("the CLI unit policy stays within the render safety envelope")
}
