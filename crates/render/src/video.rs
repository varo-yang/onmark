//! Admission policy for video streams supported by the locked browser profile.
//!
//! Probe facts remain media-owned; this module proves only the subset required
//! for deterministic source-frame selection.

use std::error::Error;
use std::fmt;

use onmark_core::model::{AssetMetadata, VideoMetadata, VideoTiming};

/// A visual stream proven admissible by the browser media profile.
///
/// Admission borrows normalized probe facts rather than copying them into a
/// second render-owned media model. Render Unit composition retains the proved
/// source rate before releasing this borrow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmittedVideo<'a> {
    metadata: &'a VideoMetadata,
}

impl<'a> AdmittedVideo<'a> {
    /// Applies the complete browser visual-asset policy.
    ///
    /// # Errors
    ///
    /// Returns [`UnsupportedVideo`] when the artifact has no visual stream,
    /// uses a codec outside the locked profile, or represents a still image.
    pub fn admit(metadata: &'a AssetMetadata) -> Result<Self, UnsupportedVideo> {
        let video = metadata
            .video_metadata()
            .ok_or(UnsupportedVideo::MissingVideoStream)?;
        if !matches!(video.codec(), "av1" | "h264" | "vp9") {
            return Err(UnsupportedVideo::Codec(video.codec().into()));
        }
        if video.pixel_format() != "yuv420p" {
            return Err(UnsupportedVideo::PixelFormat(video.pixel_format().into()));
        }
        match video.timing() {
            VideoTiming::Constant(_) | VideoTiming::Variable(_) => {}
            VideoTiming::Still => return Err(UnsupportedVideo::StillFrame),
        }

        Ok(Self { metadata: video })
    }

    /// Returns the normalized facts admitted by this proof.
    #[must_use]
    pub const fn metadata(self) -> &'a VideoMetadata {
        self.metadata
    }

    /// Returns the complete source-frame timing admitted from normalized facts.
    #[must_use]
    pub const fn timing(self) -> &'a VideoTiming {
        self.metadata.timing()
    }
}

/// Reason an asset cannot enter the browser media path.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum UnsupportedVideo {
    /// The artifact has no selected visual stream.
    MissingVideoStream,
    /// The selected codec is outside the locked browser profile.
    Codec(Box<str>),
    /// The decoded source-pixel layout is outside the locked browser profile.
    PixelFormat(Box<str>),
    /// A single-frame stream has no source frame rate.
    StillFrame,
}

impl fmt::Display for UnsupportedVideo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVideoStream => formatter.write_str("asset has no video stream"),
            Self::Codec(codec) => write!(formatter, "video codec {codec:?} is not supported"),
            Self::PixelFormat(format) => {
                write!(formatter, "video pixel format {format:?} is not supported")
            }
            Self::StillFrame => formatter.write_str("single-frame video is not supported"),
        }
    }
}

impl Error for UnsupportedVideo {}

#[cfg(test)]
mod tests {
    use onmark_core::model::{
        AssetMetadata, AudioChannelLayout, AudioSampleRate, Duration, FrameRate, MediaTimebase,
        VideoDimensions, VideoFrameMap, VideoMetadata, VideoTiming,
    };

    use super::{AdmittedVideo, UnsupportedVideo};

    #[test]
    fn admits_timed_browser_video_streams() {
        let rate = FrameRate::new(30_000, 1_001).expect("NTSC timing is valid");
        for codec in ["av1", "h264", "vp9"] {
            let supported = video(codec, VideoTiming::Constant(rate));
            let admitted =
                AdmittedVideo::admit(&supported).expect("the browser codec profile is admitted");

            assert_eq!(admitted.timing(), &VideoTiming::Constant(rate));
            assert_eq!(admitted.metadata().pixel_format(), "yuv420p");
        }
        assert_eq!(
            AdmittedVideo::admit(&AssetMetadata::audio(
                Duration::from_nanos(1),
                AudioSampleRate::new(48_000).expect("48 kHz is valid"),
                AudioChannelLayout::Stereo,
            )),
            Err(UnsupportedVideo::MissingVideoStream),
        );
        assert_eq!(
            AdmittedVideo::admit(&video("hevc", VideoTiming::Constant(rate))),
            Err(UnsupportedVideo::Codec("hevc".into())),
        );
        assert_eq!(
            AdmittedVideo::admit(&video_with_format(
                "vp9",
                "yuv420p10le",
                VideoTiming::Constant(rate),
            )),
            Err(UnsupportedVideo::PixelFormat("yuv420p10le".into())),
        );
        assert_eq!(
            AdmittedVideo::admit(&video("h264", variable_timing()))
                .expect("complete VFR timing is admitted")
                .timing(),
            &variable_timing(),
        );
        assert_eq!(
            AdmittedVideo::admit(&video("h264", VideoTiming::Still)),
            Err(UnsupportedVideo::StillFrame),
        );
    }

    fn video(codec: &str, timing: VideoTiming) -> AssetMetadata {
        video_with_format(codec, "yuv420p", timing)
    }

    fn video_with_format(codec: &str, pixel_format: &str, timing: VideoTiming) -> AssetMetadata {
        let duration = match &timing {
            VideoTiming::Variable(frame_map) => frame_map.duration(),
            VideoTiming::Constant(_) | VideoTiming::Still => Duration::from_nanos(1),
        };
        let metadata = VideoMetadata::new(
            duration,
            VideoDimensions::new(1_920, 1_080).expect("fixture dimensions are positive"),
            codec,
            pixel_format,
            timing,
        )
        .expect("the fixture metadata is normalized");
        AssetMetadata::video(duration, metadata)
    }

    fn variable_timing() -> VideoTiming {
        let timebase =
            MediaTimebase::new(1, 1_000).expect("one millisecond ticks form a valid timebase");
        let frames = VideoFrameMap::new(timebase, [0, 40, 100])
            .expect("the fixture has two variable frame intervals");
        VideoTiming::Variable(frames)
    }
}
