#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![deny(clippy::unwrap_used, clippy::print_stdout, clippy::print_stderr)]

//! Bounded browser and encoder execution for one local render unit.
//!
//! This crate owns Chromium and `FFmpeg` process lifecycles. Browser and vendor
//! types are translated at this boundary and never enter `onmark-core`.

mod browser;
mod encoder;
mod executor;
mod unit;
mod video;

pub use browser::{
    BrowserError, BrowserErrorKind, BrowserLimits, BrowserSession, EncodedPng, InvalidBrowserLimits,
};
pub use encoder::{
    EncodeError, EncodeErrorKind, EncodeLimits, EncodedVideo, Ffmpeg, FfmpegSession, InvalidFfmpeg,
};
pub use executor::{RenderError, RenderErrorKind, RenderExecutor};
pub use unit::{
    InvalidMaterializedAsset, InvalidRenderUnit, MaterializedAsset, RenderUnit, RenderVideo,
};
pub use video::{AdmittedVideo, UnsupportedVideo};
