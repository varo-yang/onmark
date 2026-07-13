mod audio;
mod error;
mod limits;
mod process;
mod session;

pub use error::{EncodeError, EncodeErrorKind};
pub use limits::{EncodeLimits, InvalidFfmpeg};
pub use session::{EncodedVideo, Ffmpeg, FfmpegSession};

pub(crate) use audio::AudioInput;
