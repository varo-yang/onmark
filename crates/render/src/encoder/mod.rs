//! `FFmpeg` visual encoding and final audio-mix boundary.

mod audio;
mod error;
mod layered;
mod layered_process;
mod limits;
mod process;
mod session;

pub use error::{EncodeError, EncodeErrorKind};
pub use limits::{EncodeLimits, InvalidFfmpeg};
pub use session::{EncodedVideo, Ffmpeg, FfmpegSession};

pub(crate) use audio::AudioInput;
pub(crate) use layered::{
    CanonicalFrame, LayeredCompletion, LayeredJob, LayeredMediaInput, LayeredOutput, LayeredSession,
};
