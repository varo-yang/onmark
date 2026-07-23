//! Chromium adapter boundary for protocol execution and frame capture.

mod error;
mod frame;
mod limits;
mod process;
mod resource;
mod session;

pub use error::{BrowserError, BrowserErrorKind};
pub(crate) use frame::DecodedRgba;
pub use frame::{CapturedFrame, EncodedPng, RawRgbaHash};
pub use limits::{BrowserLimits, InvalidBrowserLimits};
pub use process::{BrowserCaptureMode, BrowserLaunchPolicy};
pub use session::BrowserSession;
