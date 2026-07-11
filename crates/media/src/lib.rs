//! Media probing without browser or compiler responsibilities.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![deny(clippy::print_stderr, clippy::print_stdout, clippy::unwrap_used)]

mod error;
mod probe;
mod process;
mod response;

pub use error::{InvalidFfprobe, ProbeError, ProbeFailure};
pub use probe::Ffprobe;
