//! Pure domain and compilation core for Onmark.
//!
//! This crate does not perform filesystem, network, browser, media-process, or
//! cloud IO. External facts enter through typed inputs.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![deny(clippy::print_stderr, clippy::print_stdout, clippy::unwrap_used)]

pub mod diagnostics;
pub mod model;
