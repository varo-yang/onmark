//! Normalization of untrusted ffprobe JSON into stable core metadata.
//!
//! Stream-level facts are preferred over reconstructed per-frame timestamps so
//! ordinary container rounding does not make constant-rate media look variable.

use std::path::Path;

use onmark_core::model::{
    AssetMetadata, AudioChannelLayout, AudioMetadata, AudioSampleRate, Duration, FrameRate,
    VideoColorProfile, VideoMetadata, VideoTiming,
};
use serde::Deserialize;

use crate::error::ProbeError;

/// Minimal ffprobe projection; fields outside Onmark's contract are ignored.
#[derive(Deserialize)]
struct ProbeResponse {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Deserialize)]
struct ProbeStream {
    index: Option<u32>,
    codec_type: Option<Box<str>>,
    duration: Option<Box<str>>,
    codec_name: Option<Box<str>>,
    pix_fmt: Option<Box<str>>,
    color_range: Option<Box<str>>,
    color_space: Option<Box<str>>,
    color_transfer: Option<Box<str>>,
    color_primaries: Option<Box<str>>,
    avg_frame_rate: Option<Box<str>>,
    r_frame_rate: Option<Box<str>>,
    nb_frames: Option<Box<str>>,
    sample_rate: Option<Box<str>>,
    channels: Option<u32>,
    #[serde(default)]
    disposition: ProbeDisposition,
}

#[derive(Default, Deserialize)]
struct ProbeDisposition {
    #[serde(default, rename = "default")]
    is_default: u8,
    #[serde(default)]
    attached_pic: u8,
}

#[derive(Deserialize)]
struct ProbeFormat {
    duration: Option<Box<str>>,
}

pub(crate) fn parse_metadata(path: &Path, bytes: &[u8]) -> Result<AssetMetadata, ProbeError> {
    let response = serde_json::from_slice::<ProbeResponse>(bytes)
        .map_err(|source| ProbeError::invalid_response(path, source))?;
    let format_duration = response
        .format
        .and_then(|format| format.duration)
        .map(|duration| parse_format_duration(path, &duration))
        .transpose()?;

    let SelectedStreams {
        audio: audio_stream,
        video: video_stream,
    } = select_streams(response.streams);

    let video = video_stream
        .map(|stream| parse_video(path, stream, format_duration))
        .transpose()?;
    let audio = audio_stream
        .map(|stream| parse_audio(path, stream, format_duration))
        .transpose()?;
    let duration = format_duration
        .or_else(|| {
            longest_duration(
                video.as_ref().map(VideoMetadata::duration),
                audio.as_ref().map(AudioMetadata::duration),
            )
        })
        .ok_or_else(|| ProbeError::missing_duration(path))?;

    match (audio, video) {
        (Some(audio), Some(video)) => Ok(AssetMetadata::audio_video(duration, audio, video)),
        (Some(audio), None) => Ok(AssetMetadata::audio_only(duration, audio)),
        (None, Some(video)) => Ok(AssetMetadata::video(duration, video)),
        (None, None) => Ok(AssetMetadata::without_media_tracks(duration)),
    }
}

impl ProbeStream {
    fn is_audio(&self) -> bool {
        self.codec_type.as_deref() == Some("audio")
    }

    fn is_visual(&self) -> bool {
        self.codec_type.as_deref() == Some("video") && self.disposition.attached_pic != 1
    }

    fn selection_key(&self) -> (StreamPriority, u32) {
        (
            if self.disposition.is_default == 1 {
                StreamPriority::Default
            } else {
                StreamPriority::Other
            },
            self.index.unwrap_or(u32::MAX),
        )
    }
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum StreamPriority {
    Default,
    Other,
}

#[derive(Default)]
struct SelectedStreams {
    audio: Option<ProbeStream>,
    video: Option<ProbeStream>,
}

fn select_streams(streams: Vec<ProbeStream>) -> SelectedStreams {
    let mut selected = SelectedStreams::default();
    for stream in streams {
        if stream.is_audio() {
            select_stream(&mut selected.audio, stream);
        } else if stream.is_visual() {
            select_stream(&mut selected.video, stream);
        }
    }
    selected
}

fn select_stream(selected: &mut Option<ProbeStream>, candidate: ProbeStream) {
    let replace = match selected {
        Some(current) => candidate.selection_key() < current.selection_key(),
        None => true,
    };
    if replace {
        *selected = Some(candidate);
    }
}

fn parse_video(
    path: &Path,
    stream: ProbeStream,
    format_duration: Option<Duration>,
) -> Result<VideoMetadata, ProbeError> {
    let duration = video_duration(path, stream.duration.as_deref(), format_duration)?;
    let timing = parse_timing(path, &stream)?;
    let color_profile = parse_color_profile(&stream);
    let codec = required_field(path, "codec name", stream.codec_name)?;
    let pixel_format = required_field(path, "pixel format", stream.pix_fmt)?;

    let metadata = VideoMetadata::new(duration, codec, pixel_format, timing)
        .map_err(|source| ProbeError::invalid_video(path, source.to_string()))?;
    Ok(match color_profile {
        Some(profile) => metadata.with_color_profile(profile),
        None => metadata,
    })
}

fn parse_color_profile(stream: &ProbeStream) -> Option<VideoColorProfile> {
    let profile = (
        stream.color_range.as_deref(),
        stream.color_space.as_deref(),
        stream.color_transfer.as_deref(),
        stream.color_primaries.as_deref(),
    );
    match profile {
        (Some("tv"), Some("bt709"), Some("bt709"), Some("bt709")) => {
            Some(VideoColorProfile::Bt709Limited)
        }
        _ => None,
    }
}

fn video_duration(
    path: &Path,
    stream_duration: Option<&str>,
    format_duration: Option<Duration>,
) -> Result<Duration, ProbeError> {
    match stream_duration {
        None | Some("N/A") => format_duration
            .ok_or_else(|| ProbeError::invalid_video(path, "video stream has no duration")),
        Some(duration) => Duration::parse(&format!("{duration}s"))
            .map_err(|source| ProbeError::invalid_video_duration(path, duration, source)),
    }
}

fn parse_audio(
    path: &Path,
    stream: ProbeStream,
    format_duration: Option<Duration>,
) -> Result<AudioMetadata, ProbeError> {
    let duration = match stream.duration.as_deref() {
        None | Some("N/A") => {
            format_duration.ok_or_else(|| ProbeError::invalid_audio(path, "missing duration"))?
        }
        Some(duration) => parse_audio_duration(path, duration)?,
    };
    let sample_rate = required_audio_field(path, "sample rate", stream.sample_rate)?;
    let sample_rate = sample_rate.parse::<u32>().map_err(|_| {
        ProbeError::invalid_audio(
            path,
            format!("sample rate {sample_rate:?} is not an integer"),
        )
    })?;
    let sample_rate = AudioSampleRate::new(sample_rate)
        .map_err(|source| ProbeError::invalid_audio(path, source))?;
    let channel_layout = parse_audio_channels(path, stream.channels)?;

    Ok(AudioMetadata::new(duration, sample_rate, channel_layout))
}

fn parse_audio_channels(
    path: &Path,
    channels: Option<u32>,
) -> Result<AudioChannelLayout, ProbeError> {
    match channels {
        Some(1) => Ok(AudioChannelLayout::Mono),
        Some(2) => Ok(AudioChannelLayout::Stereo),
        Some(channels) => Err(ProbeError::invalid_audio(
            path,
            format!("{channels}-channel audio is not supported"),
        )),
        None => Err(ProbeError::invalid_audio(path, "missing channel count")),
    }
}

fn parse_format_duration(path: &Path, duration: &str) -> Result<Duration, ProbeError> {
    Duration::parse(&format!("{duration}s"))
        .map_err(|source| ProbeError::invalid_duration(path, duration, source))
}

fn parse_audio_duration(path: &Path, duration: &str) -> Result<Duration, ProbeError> {
    Duration::parse(&format!("{duration}s"))
        .map_err(|source| ProbeError::invalid_audio_duration(path, duration, source))
}

fn longest_duration(first: Option<Duration>, second: Option<Duration>) -> Option<Duration> {
    match (first, second) {
        (Some(first), Some(second)) => Some(first.max(second)),
        (None, Some(duration)) | (Some(duration), None) => Some(duration),
        (None, None) => None,
    }
}

fn parse_timing(path: &Path, stream: &ProbeStream) -> Result<VideoTiming, ProbeError> {
    let frame_count = parse_frame_count(path, stream.nb_frames.as_deref())?;
    if frame_count == Some(0) {
        return Err(ProbeError::invalid_video(
            path,
            "video stream contains no frames",
        ));
    }
    if frame_count == Some(1) {
        return Ok(VideoTiming::Still);
    }

    let average = parse_frame_rate(path, "average frame rate", stream.avg_frame_rate.as_deref())?;
    let nominal = parse_frame_rate(path, "nominal frame rate", stream.r_frame_rate.as_deref())?;
    let (Some(average), Some(nominal)) = (average, nominal) else {
        return Ok(VideoTiming::Variable);
    };
    if average == nominal {
        return Ok(VideoTiming::Constant(average));
    }

    Ok(VideoTiming::Variable)
}

fn parse_frame_count(path: &Path, value: Option<&str>) -> Result<Option<u64>, ProbeError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value == "N/A" {
        return Ok(None);
    }

    value.parse::<u64>().map(Some).map_err(|_| {
        ProbeError::invalid_video(path, format!("frame count {value:?} is not an integer"))
    })
}

fn parse_frame_rate(
    path: &Path,
    name: &str,
    value: Option<&str>,
) -> Result<Option<FrameRate>, ProbeError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if matches!(value, "0/0" | "N/A") {
        return Ok(None);
    }

    let invalid =
        || ProbeError::invalid_video(path, format!("{name} {value:?} is not a rational number"));
    let Some((numerator, denominator)) = value.split_once('/') else {
        return Err(invalid());
    };
    let numerator = numerator.parse::<u64>().map_err(|_| invalid())?;
    let denominator = denominator.parse::<u64>().map_err(|_| invalid())?;
    if numerator == 0 || denominator == 0 {
        return Err(ProbeError::invalid_video(
            path,
            format!("{name} {value:?} must be positive"),
        ));
    }

    let numerator = u32::try_from(numerator)
        .map_err(|_| ProbeError::invalid_video(path, format!("{name} numerator is too large")))?;
    let denominator = u32::try_from(denominator)
        .map_err(|_| ProbeError::invalid_video(path, format!("{name} denominator is too large")))?;
    FrameRate::new(numerator, denominator)
        .map(Some)
        .map_err(|source| ProbeError::invalid_video(path, source.to_string()))
}

fn required_field<T>(path: &Path, name: &str, value: Option<T>) -> Result<T, ProbeError> {
    value.ok_or_else(|| ProbeError::invalid_video(path, format!("video stream has no {name}")))
}

fn required_audio_field<T>(path: &Path, name: &str, value: Option<T>) -> Result<T, ProbeError> {
    value.ok_or_else(|| ProbeError::invalid_audio(path, format!("audio stream has no {name}")))
}
