//! `FFmpeg` visual encoding and final audio-mix boundary.

mod audio;
mod ducking;
mod error;
mod layered;
mod layered_process;
mod limits;
mod process;
mod profile;
mod session;

pub use error::{EncodeError, EncodeErrorKind};
pub use limits::{EncodeLimits, InvalidFfmpeg};
pub use profile::EncodeProfile;
pub use session::{EncodedVideo, Ffmpeg, FfmpegSession};

pub(crate) use audio::AudioInput;
pub(crate) use layered::{
    BackdropMediaInput, CanonicalFrame, LayeredCompletion, LayeredInputs, LayeredJob,
    LayeredMediaInput, LayeredOutput, LayeredSession,
};
