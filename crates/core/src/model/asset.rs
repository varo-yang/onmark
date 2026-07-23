//! Immutable asset identity and normalized media facts used by pure compilation.
//!
//! Paths and probing vendors stay outside the model; only content identity and
//! facts that can affect solving cross this boundary.

use std::error::Error;
use std::fmt;

use super::{AudioChannelLayout, AudioSampleRate, Duration, FrameRate};

/// Byte width of the SHA-256 asset digest.
const SHA256_BYTES: usize = 32;

/// Immutable identity of the exact asset bytes consumed by compilation.
///
/// Paths and authored references may change between machines. This identity
/// crosses into Timeline IR so later materialization can prove it supplied the
/// bytes whose metadata the compiler used.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FrozenAssetId([u8; SHA256_BYTES]);

impl FrozenAssetId {
    /// Creates an asset identity from a SHA-256 digest computed while freezing
    /// the input bytes.
    #[must_use]
    pub const fn from_sha256(digest: [u8; SHA256_BYTES]) -> Self {
        Self(digest)
    }

    /// Parses the canonical `sha256:<lowercase-hex>` identity spelling.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidFrozenAssetId`] when the prefix, digest length, or
    /// lowercase hexadecimal spelling is not canonical.
    pub fn parse(value: &str) -> Result<Self, InvalidFrozenAssetId> {
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(InvalidFrozenAssetId::MissingPrefix);
        };
        if hex.len() != SHA256_BYTES * 2 {
            return Err(InvalidFrozenAssetId::InvalidLength);
        }

        let mut digest = [0; SHA256_BYTES];
        for (index, byte) in digest.iter_mut().enumerate() {
            let offset = index * 2;
            let high = hex_value(hex.as_bytes()[offset])?;
            let low = hex_value(hex.as_bytes()[offset + 1])?;
            *byte = high << 4 | low;
        }
        Ok(Self::from_sha256(digest))
    }

    /// Returns the SHA-256 digest bytes.
    #[must_use]
    pub const fn as_sha256(&self) -> &[u8; SHA256_BYTES] {
        &self.0
    }
}

/// Reason a frozen-asset identity spelling cannot name immutable bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidFrozenAssetId {
    /// The required `sha256:` prefix is absent.
    MissingPrefix,
    /// The SHA-256 digest does not have exactly 64 hexadecimal characters.
    InvalidLength,
    /// The digest contains a noncanonical hexadecimal byte.
    InvalidHex,
}

impl fmt::Display for InvalidFrozenAssetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingPrefix => "frozen asset identity must start with sha256:",
            Self::InvalidLength => "frozen asset identity must contain 64 hexadecimal characters",
            Self::InvalidHex => "frozen asset identity must use lowercase hexadecimal characters",
        })
    }
}

impl Error for InvalidFrozenAssetId {}

impl fmt::Display for FrozenAssetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sha256:")?;
        for byte in self.as_sha256() {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

fn hex_value(byte: u8) -> Result<u8, InvalidFrozenAssetId> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(InvalidFrozenAssetId::InvalidHex),
    }
}

/// Normalized facts probed from one media artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetMetadata {
    duration: Duration,
    tracks: MediaTracks,
}

impl AssetMetadata {
    /// Creates metadata for an artifact with an audio stream and no visual stream.
    #[must_use]
    pub const fn audio(
        duration: Duration,
        sample_rate: AudioSampleRate,
        channel_layout: AudioChannelLayout,
    ) -> Self {
        Self::audio_only(
            duration,
            AudioMetadata::new(duration, sample_rate, channel_layout),
        )
    }

    /// Creates metadata for an artifact with one normalized audio stream.
    #[must_use]
    pub const fn audio_only(duration: Duration, audio: AudioMetadata) -> Self {
        Self {
            duration,
            tracks: MediaTracks::Audio(audio),
        }
    }

    /// Creates metadata for an artifact with one visual stream and no audio stream.
    #[must_use]
    pub const fn video(duration: Duration, video: VideoMetadata) -> Self {
        Self {
            duration,
            tracks: MediaTracks::Video(video),
        }
    }

    /// Creates metadata for an artifact with independently measured audio and
    /// visual streams.
    #[must_use]
    pub const fn audio_video(
        duration: Duration,
        audio: AudioMetadata,
        video: VideoMetadata,
    ) -> Self {
        Self {
            duration,
            tracks: MediaTracks::AudioVideo { audio, video },
        }
    }

    /// Creates metadata for an artifact with neither audio nor visual streams.
    #[must_use]
    pub const fn without_media_tracks(duration: Duration) -> Self {
        Self {
            duration,
            tracks: MediaTracks::None,
        }
    }

    /// Returns the exact probed artifact duration.
    #[must_use]
    pub const fn duration(&self) -> Duration {
        self.duration
    }

    /// Returns normalized facts for the selected visual stream, when present.
    #[must_use]
    pub const fn video_metadata(&self) -> Option<&VideoMetadata> {
        match &self.tracks {
            MediaTracks::Video(video) | MediaTracks::AudioVideo { video, .. } => Some(video),
            MediaTracks::None | MediaTracks::Audio(_) => None,
        }
    }

    /// Returns normalized facts for the selected audio stream, when present.
    #[must_use]
    pub const fn audio_metadata(&self) -> Option<&AudioMetadata> {
        match &self.tracks {
            MediaTracks::Audio(audio) | MediaTracks::AudioVideo { audio, .. } => Some(audio),
            MediaTracks::None | MediaTracks::Video(_) => None,
        }
    }

    /// Returns whether probing found at least one audio stream.
    #[must_use]
    pub const fn has_audio_stream(&self) -> bool {
        self.audio_metadata().is_some()
    }
}

/// The track combinations retained by compilation.
///
/// This remains private because callers ask only the two questions relevant to
/// their element: whether an audio stream exists and whether normalized visual
/// metadata exists. One closed probe fact is clearer than independently
/// mutable audio and video flags.
#[derive(Clone, Debug, Eq, PartialEq)]
enum MediaTracks {
    None,
    Audio(AudioMetadata),
    Video(VideoMetadata),
    AudioVideo {
        audio: AudioMetadata,
        video: VideoMetadata,
    },
}

/// Normalized facts for the selected audio stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioMetadata {
    duration: Duration,
    sample_rate: AudioSampleRate,
    channel_layout: AudioChannelLayout,
}

impl AudioMetadata {
    /// Creates audio metadata from an exact stream duration.
    #[must_use]
    pub const fn new(
        duration: Duration,
        sample_rate: AudioSampleRate,
        channel_layout: AudioChannelLayout,
    ) -> Self {
        Self {
            duration,
            sample_rate,
            channel_layout,
        }
    }

    /// Returns the exact selected-stream duration.
    #[must_use]
    pub const fn duration(&self) -> Duration {
        self.duration
    }

    /// Returns the exact selected-stream sample rate.
    #[must_use]
    pub const fn sample_rate(&self) -> AudioSampleRate {
        self.sample_rate
    }

    /// Returns the normalized source channel layout.
    #[must_use]
    pub const fn channel_layout(&self) -> AudioChannelLayout {
        self.channel_layout
    }
}

/// Closed normalized source-color profile.
///
/// A profile names the complete range, matrix, transfer, and primaries tuple.
/// Keeping the tuple closed prevents a renderer from combining partial probe
/// facts into a color conversion that no media boundary actually observed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoColorProfile {
    /// BT.709 primaries, transfer, and matrix with limited-range samples.
    Bt709Limited,
}

/// Positive source-pixel dimensions observed at the media boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoDimensions {
    width: u32,
    height: u32,
}

impl VideoDimensions {
    /// Creates one normalized source raster.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidVideoDimensions`] when either edge is zero.
    pub const fn new(width: u32, height: u32) -> Result<Self, InvalidVideoDimensions> {
        if width == 0 || height == 0 {
            return Err(InvalidVideoDimensions);
        }
        Ok(Self { width, height })
    }

    /// Returns the source width in pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the source height in pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
}

/// Reason source-pixel dimensions were rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidVideoDimensions;

impl fmt::Display for InvalidVideoDimensions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("video dimensions must be positive")
    }
}

impl Error for InvalidVideoDimensions {}

/// Normalized facts for the visual stream selected during probing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VideoMetadata {
    duration: Duration,
    dimensions: VideoDimensions,
    codec: Box<str>,
    pixel_format: Box<str>,
    timing: VideoTiming,
    color_profile: Option<VideoColorProfile>,
}

impl VideoMetadata {
    /// Creates video metadata from normalized ffprobe names and exact timing.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidVideoMetadata`] when a name is empty or contains ASCII
    /// whitespace. The media boundary translates this local validation reason
    /// once; downstream code receives trusted format names.
    pub fn new(
        duration: Duration,
        dimensions: VideoDimensions,
        codec: impl Into<Box<str>>,
        pixel_format: impl Into<Box<str>>,
        timing: VideoTiming,
    ) -> Result<Self, InvalidVideoMetadata> {
        let codec = codec.into();
        validate_format_name(&codec, InvalidVideoMetadata::InvalidCodec)?;

        let pixel_format = pixel_format.into();
        validate_format_name(&pixel_format, InvalidVideoMetadata::InvalidPixelFormat)?;

        Ok(Self {
            duration,
            dimensions,
            codec,
            pixel_format,
            timing,
            color_profile: None,
        })
    }

    /// Records one complete source-color tuple recognized by the media
    /// boundary.
    #[must_use]
    pub const fn with_color_profile(mut self, color_profile: VideoColorProfile) -> Self {
        self.color_profile = Some(color_profile);
        self
    }

    /// Returns the exact selected-stream duration.
    #[must_use]
    pub const fn duration(&self) -> Duration {
        self.duration
    }

    /// Returns the selected stream's source-pixel dimensions.
    #[must_use]
    pub const fn dimensions(&self) -> VideoDimensions {
        self.dimensions
    }

    /// Returns the normalized ffprobe codec name.
    #[must_use]
    pub fn codec(&self) -> &str {
        &self.codec
    }

    /// Returns the normalized ffprobe pixel-format name.
    #[must_use]
    pub fn pixel_format(&self) -> &str {
        &self.pixel_format
    }

    /// Returns the observed source-frame timing shape.
    #[must_use]
    pub const fn timing(&self) -> VideoTiming {
        self.timing
    }

    /// Returns the complete admitted source-color tuple, when probing reported
    /// one without missing or unsupported fields.
    #[must_use]
    pub const fn color_profile(&self) -> Option<VideoColorProfile> {
        self.color_profile
    }
}

/// Timing shape inferred from ffprobe's stream-level frame facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VideoTiming {
    /// ffprobe reports matching average and nominal rational frame rates.
    Constant(FrameRate),
    /// ffprobe reports disagreeing or unavailable stream frame rates.
    Variable,
    /// ffprobe reports exactly one frame and therefore no observable rate.
    Still,
}

/// Reason normalized video metadata cannot be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidVideoMetadata {
    /// The codec name is empty or contains ASCII whitespace.
    InvalidCodec,
    /// The pixel-format name is empty or contains ASCII whitespace.
    InvalidPixelFormat,
}

impl fmt::Display for InvalidVideoMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidCodec => "video codec name is invalid",
            Self::InvalidPixelFormat => "video pixel-format name is invalid",
        };
        formatter.write_str(message)
    }
}

impl Error for InvalidVideoMetadata {}

fn validate_format_name(
    name: &str,
    invalid: InvalidVideoMetadata,
) -> Result<(), InvalidVideoMetadata> {
    if name.is_empty() || name.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return Err(invalid);
    }
    Ok(())
}

/// One frozen artifact and the normalized facts probed from those same bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrozenAsset {
    id: FrozenAssetId,
    metadata: AssetMetadata,
}

impl FrozenAsset {
    /// Joins immutable byte identity with metadata derived from those bytes.
    ///
    /// The IO boundary constructing this value must ensure that `metadata` was
    /// probed from the bytes identified by `id`; pure core cannot inspect that
    /// external fact.
    #[must_use]
    pub const fn new(id: FrozenAssetId, metadata: AssetMetadata) -> Self {
        Self { id, metadata }
    }

    /// Returns the immutable artifact identity.
    #[must_use]
    pub const fn id(&self) -> FrozenAssetId {
        self.id
    }

    /// Returns normalized probe facts for the immutable artifact.
    #[must_use]
    pub const fn metadata(&self) -> &AssetMetadata {
        &self.metadata
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AssetMetadata, AudioMetadata, FrozenAssetId, InvalidFrozenAssetId, InvalidVideoDimensions,
        InvalidVideoMetadata, VideoDimensions, VideoMetadata, VideoTiming,
    };
    use crate::model::{AudioChannelLayout, AudioSampleRate, Duration, FrameRate};

    #[test]
    fn frozen_identity_has_an_algorithm_named_canonical_spelling() {
        let id = FrozenAssetId::from_sha256([0xab; 32]);

        assert_eq!(
            id.to_string(),
            "sha256:abababababababababababababababababababababababababababababababab",
        );
        assert_eq!(id.as_sha256(), &[0xab; 32]);
    }

    #[test]
    fn parses_only_canonical_frozen_identity_spelling() {
        let canonical = "sha256:abababababababababababababababababababababababababababababababab";

        assert_eq!(
            FrozenAssetId::parse(canonical)
                .expect("the canonical fixture identity is valid")
                .as_sha256(),
            &[0xab; 32],
        );
        assert_eq!(
            FrozenAssetId::parse("sha256:AB"),
            Err(InvalidFrozenAssetId::InvalidLength),
        );
        assert_eq!(
            FrozenAssetId::parse("sha512:abab"),
            Err(InvalidFrozenAssetId::MissingPrefix),
        );
        assert_eq!(
            FrozenAssetId::parse(
                "sha256:Abababababababababababababababababababababababababababababababab",
            ),
            Err(InvalidFrozenAssetId::InvalidHex),
        );
    }

    #[test]
    fn video_metadata_rejects_names_that_are_not_normalized_tokens() {
        let rate = FrameRate::new(30, 1).expect("30 fps is valid");
        let dimensions = video_dimensions();

        assert_eq!(
            VideoMetadata::new(
                Duration::from_nanos(1),
                dimensions,
                "",
                "yuv420p",
                VideoTiming::Constant(rate),
            ),
            Err(InvalidVideoMetadata::InvalidCodec),
        );
        assert_eq!(
            VideoMetadata::new(
                Duration::from_nanos(1),
                dimensions,
                "h264",
                "yuv 420p",
                VideoTiming::Constant(rate),
            ),
            Err(InvalidVideoMetadata::InvalidPixelFormat),
        );
    }

    #[test]
    fn asset_metadata_preserves_closed_track_combinations() {
        let duration = Duration::from_nanos(1);
        let rate = FrameRate::new(30, 1).expect("30 fps is valid");
        let sample_rate = AudioSampleRate::new(48_000).expect("48 kHz is valid");
        let video = VideoMetadata::new(
            duration,
            video_dimensions(),
            "h264",
            "yuv420p",
            VideoTiming::Constant(rate),
        )
        .expect("the video metadata is valid");
        let audio = AssetMetadata::audio(duration, sample_rate, AudioChannelLayout::Stereo);
        let video_only = AssetMetadata::video(duration, video.clone());
        let audio_video = AssetMetadata::audio_video(
            duration,
            AudioMetadata::new(duration, sample_rate, AudioChannelLayout::Stereo),
            video,
        );
        let without_tracks = AssetMetadata::without_media_tracks(duration);

        assert!(audio.has_audio_stream());
        assert!(audio.video_metadata().is_none());
        assert!(!video_only.has_audio_stream());
        assert!(video_only.video_metadata().is_some());
        assert!(audio_video.has_audio_stream());
        assert!(audio_video.video_metadata().is_some());
        assert!(!without_tracks.has_audio_stream());
        assert!(without_tracks.video_metadata().is_none());
    }

    #[test]
    fn video_dimensions_require_positive_axes() {
        assert_eq!(VideoDimensions::new(0, 1_080), Err(InvalidVideoDimensions),);
        assert_eq!(VideoDimensions::new(1_920, 0), Err(InvalidVideoDimensions),);

        let dimensions = video_dimensions();
        assert_eq!(dimensions.width(), 1_920);
        assert_eq!(dimensions.height(), 1_080);
    }

    fn video_dimensions() -> VideoDimensions {
        VideoDimensions::new(1_920, 1_080).expect("fixture dimensions are positive")
    }

    #[test]
    fn audio_metadata_preserves_a_stream_duration_distinct_from_the_artifact() {
        let artifact = Duration::from_nanos(2);
        let stream = Duration::from_nanos(1);
        let sample_rate = AudioSampleRate::new(48_000).expect("48 kHz is valid");
        let metadata = AssetMetadata::audio_only(
            artifact,
            AudioMetadata::new(stream, sample_rate, AudioChannelLayout::Mono),
        );

        assert_eq!(metadata.duration(), artifact);
        assert_eq!(
            metadata
                .audio_metadata()
                .expect("the metadata has an audio stream")
                .duration(),
            stream,
        );
        assert_eq!(
            metadata
                .audio_metadata()
                .expect("the metadata has an audio stream")
                .sample_rate(),
            sample_rate,
        );
    }
}
