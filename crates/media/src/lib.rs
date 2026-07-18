//! Media probing and subtitle normalization without browser responsibilities.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![deny(clippy::print_stderr, clippy::print_stdout, clippy::unwrap_used)]

mod error;
mod probe;
mod process;
mod response;
mod subtitle;

pub use error::{InvalidFfprobe, ProbeError, ProbeFailure};
pub use probe::Ffprobe;
pub use subtitle::{
    InvalidSubtitleLimits, SubtitleError, SubtitleErrorKind, SubtitleLimits, SubtitleReport,
    parse_ass, parse_subrip, parse_webvtt,
};
