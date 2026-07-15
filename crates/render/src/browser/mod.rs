//! Chromium adapter boundary for protocol execution and frame capture.

mod error;
mod frame;
mod limits;
mod session;

pub use error::{BrowserError, BrowserErrorKind};
pub use frame::{CapturedFrame, EncodedPng, RawRgbaHash};
pub use limits::{BrowserLimits, InvalidBrowserLimits};
pub use session::{BrowserSession, ChromiumSandbox};
