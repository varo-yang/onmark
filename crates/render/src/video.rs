//! Admission policy for video streams supported by the locked browser profile.
//!
//! Probe facts remain media-owned; this module proves only the subset required
//! for deterministic source-frame selection.

use std::error::Error;
use std::fmt;

use onmark_core::model::{AssetMetadata, FrameRate, VideoMetadata, VideoTiming};

/// A visual stream proven admissible by the Gate-one browser profile.
///
/// Admission borrows normalized probe facts rather than copying them into a
/// second render-owned media model. Render Unit composition retains the proved
/// source rate before releasing this borrow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmittedVideo<'a> {
    metadata: &'a VideoMetadata,
    frame_rate: FrameRate,
}

impl<'a> AdmittedVideo<'a> {
    /// Applies the complete Gate-one visual-asset policy.
    ///
    /// # Errors
    ///
    /// Returns [`UnsupportedVideo`] when the artifact has no visual stream,
    /// uses a codec outside the locked profile, or has variable frame timing.
    pub fn admit(metadata: &'a AssetMetadata) -> Result<Self, UnsupportedVideo> {
        let video = metadata
            .video_metadata()
            .ok_or(UnsupportedVideo::MissingVideoStream)?;
        if video.codec() != "h264" {
            return Err(UnsupportedVideo::Codec(video.codec().into()));
        }
        let frame_rate = match video.timing() {
            VideoTiming::Constant(frame_rate) => frame_rate,
            VideoTiming::Variable => return Err(UnsupportedVideo::VariableFrameRate),
            VideoTiming::Still => return Err(UnsupportedVideo::StillFrame),
        };

        Ok(Self {
            metadata: video,
            frame_rate,
        })
    }

    /// Returns the normalized facts admitted by this proof.
    #[must_use]
    pub const fn metadata(self) -> &'a VideoMetadata {
        self.metadata
    }

    /// Returns the exact source frame rate admitted from normalized stream facts.
    #[must_use]
    pub const fn frame_rate(self) -> FrameRate {
        self.frame_rate
    }
}

/// Reason an asset cannot enter the Gate-one browser media path.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum UnsupportedVideo {
    /// The artifact has no selected visual stream.
    MissingVideoStream,
    /// The selected codec is outside the locked Gate-one profile.
    Codec(Box<str>),
    /// Source-frame presentation intervals are not constant.
    VariableFrameRate,
    /// A single-frame stream has no source frame rate.
    StillFrame,
}

impl fmt::Display for UnsupportedVideo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVideoStream => formatter.write_str("asset has no video stream"),
            Self::Codec(codec) => write!(formatter, "video codec {codec:?} is not supported"),
            Self::VariableFrameRate => {
                formatter.write_str("variable-frame-rate video is not supported")
            }
            Self::StillFrame => formatter.write_str("single-frame video is not supported"),
        }
    }
}

impl Error for UnsupportedVideo {}

#[cfg(test)]
mod tests {
    use onmark_core::model::{
        AssetMetadata, AudioChannelLayout, AudioSampleRate, Duration, FrameRate, VideoMetadata,
        VideoTiming,
    };

    use super::{AdmittedVideo, UnsupportedVideo};

    #[test]
    fn admits_only_cfr_h264_visual_streams() {
        let rate = FrameRate::new(30_000, 1_001).expect("NTSC timing is valid");
        let supported = video("h264", VideoTiming::Constant(rate));
        let admitted = AdmittedVideo::admit(&supported).expect("CFR H.264 is the Gate-one profile");

        assert_eq!(admitted.frame_rate(), rate);
        assert_eq!(admitted.metadata().pixel_format(), "yuv420p");
        assert_eq!(
            AdmittedVideo::admit(&AssetMetadata::audio(
                Duration::from_nanos(1),
                AudioSampleRate::new(48_000).expect("48 kHz is valid"),
                AudioChannelLayout::Stereo,
            )),
            Err(UnsupportedVideo::MissingVideoStream),
        );
        assert_eq!(
            AdmittedVideo::admit(&video("vp9", VideoTiming::Constant(rate))),
            Err(UnsupportedVideo::Codec("vp9".into())),
        );
        assert_eq!(
            AdmittedVideo::admit(&video("h264", VideoTiming::Variable)),
            Err(UnsupportedVideo::VariableFrameRate),
        );
        assert_eq!(
            AdmittedVideo::admit(&video("h264", VideoTiming::Still)),
            Err(UnsupportedVideo::StillFrame),
        );
    }

    fn video(codec: &str, timing: VideoTiming) -> AssetMetadata {
        let duration = Duration::from_nanos(1);
        let metadata = VideoMetadata::new(duration, codec, "yuv420p", timing)
            .expect("the fixture metadata is normalized");
        AssetMetadata::video(duration, metadata)
    }
}
