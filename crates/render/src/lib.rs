#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![deny(clippy::unwrap_used, clippy::print_stdout, clippy::print_stderr)]

//! Bounded browser and encoder execution for one local render unit.
//!
//! This crate owns Chromium and `FFmpeg` process lifecycles. Browser and vendor
//! types are translated at this boundary and never enter `onmark-core`.

mod browser;
mod encoder;
mod environment;
mod executor;
mod frame_artifact;
mod profile;
mod unit;
mod unit_root;
mod video;
mod worker;

pub use browser::{
    BrowserError, BrowserErrorKind, BrowserLaunchPolicy, BrowserLimits, BrowserSession,
    CapturedFrame, EncodedPng, InvalidBrowserLimits, RawRgbaHash,
};
pub use encoder::{
    EncodeError, EncodeErrorKind, EncodeLimits, EncodedVideo, Ffmpeg, FfmpegSession, InvalidFfmpeg,
};
pub use environment::{CaptureEnvironmentId, InvalidCaptureEnvironmentId};
pub use executor::{
    FrameCaptureExecutor, FrameCaptureMetrics, FrameCaptureReport, RenderError, RenderErrorKind,
    RenderExecutor,
};
pub use frame_artifact::{
    FrameArtifact, FrameArtifactError, FrameArtifactErrorKind, FrameArtifactId,
    FrameArtifactLimits, InvalidFrameArtifactId, InvalidFrameArtifactLimits,
};
pub use profile::{InvalidRenderProfile, RenderProfile};
pub use unit::{
    AudioPlan, InvalidMaterializedAsset, InvalidRenderUnit, MaterializedAsset, RenderAudio,
    RenderUnit, RenderVideo,
};
pub use unit_root::{
    ExecutableUnit, InvalidUnitRootLimits, UnitRoot, UnitRootError, UnitRootErrorKind,
    UnitRootLimits,
};
pub use video::{AdmittedVideo, UnsupportedVideo};
pub use worker::{WorkerCaptureRequest, WorkerCaptureVersion};
