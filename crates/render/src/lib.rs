#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![deny(clippy::unwrap_used, clippy::print_stdout, clippy::print_stderr)]

//! Bounded browser and encoder execution for one local render unit.
//!
//! This crate owns Chromium and `FFmpeg` process lifecycles. Browser and vendor
//! types are translated at this boundary and never enter `onmark-core`.

mod browser;

pub use browser::{
    BrowserError, BrowserErrorKind, BrowserLimits, BrowserSession, EncodedPng, InvalidBrowserLimits,
};
