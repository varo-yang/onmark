//! Normalization of untrusted ffprobe JSON into stable core metadata.
//!
//! Stream-level facts are preferred over reconstructed per-frame timestamps so
//! ordinary container rounding does not make constant-rate media look variable.

use std::path::Path;

use onmark_core::model::{
    AssetMetadata, AudioMetadata, Duration, FrameRate, VideoMetadata, VideoTiming,
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
    codec_type: Option<Box<str>>,
    duration: Option<Box<str>>,
    codec_name: Option<Box<str>>,
    pix_fmt: Option<Box<str>>,
    avg_frame_rate: Option<Box<str>>,
    r_frame_rate: Option<Box<str>>,
    nb_frames: Option<Box<str>>,
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

    let mut audio_stream = None;
    let mut video_stream = None;
    for stream in response.streams {
        if stream.is_audio() && audio_stream.is_none() {
            audio_stream = Some(stream);
            continue;
        }
        if stream.is_video() && video_stream.is_none() {
            video_stream = Some(stream);
        }
    }

    let video = video_stream
        .map(|stream| parse_video(path, stream))
        .transpose()?;
    let audio_duration = match audio_stream
        .as_ref()
        .and_then(|stream| stream.duration.as_deref())
    {
        None | Some("N/A") => None,
        Some(duration) => Some(parse_stream_duration(path, duration)?),
    };
    let duration = format_duration
        .or_else(|| longest_duration(video.as_ref().map(VideoMetadata::duration), audio_duration))
        .ok_or_else(|| ProbeError::missing_duration(path))?;

    match (audio_stream, video, audio_duration) {
        (Some(_), Some(video), audio_duration) => Ok(AssetMetadata::audio_video(
            duration,
            AudioMetadata::new(audio_duration.unwrap_or(duration)),
            video,
        )),
        (Some(_), None, audio_duration) => Ok(AssetMetadata::audio_with_stream_duration(
            duration,
            audio_duration.unwrap_or(duration),
        )),
        (None, Some(video), _) => Ok(AssetMetadata::video(duration, video)),
        (None, None, _) => Ok(AssetMetadata::without_media_tracks(duration)),
    }
}

impl ProbeStream {
    fn is_audio(&self) -> bool {
        self.codec_type.as_deref() == Some("audio")
    }

    fn is_video(&self) -> bool {
        self.codec_type.as_deref() == Some("video")
    }
}

fn parse_video(path: &Path, stream: ProbeStream) -> Result<VideoMetadata, ProbeError> {
    let duration = required_field(path, "duration", stream.duration.as_deref())?;
    let duration = Duration::parse(&format!("{duration}s"))
        .map_err(|source| ProbeError::invalid_video_duration(path, duration, source))?;
    let timing = parse_timing(path, &stream)?;
    let codec = required_field(path, "codec name", stream.codec_name)?;
    let pixel_format = required_field(path, "pixel format", stream.pix_fmt)?;

    VideoMetadata::new(duration, codec, pixel_format, timing)
        .map_err(|source| ProbeError::invalid_video(path, source.to_string()))
}

fn parse_format_duration(path: &Path, duration: &str) -> Result<Duration, ProbeError> {
    Duration::parse(&format!("{duration}s"))
        .map_err(|source| ProbeError::invalid_duration(path, duration, source))
}

fn parse_stream_duration(path: &Path, duration: &str) -> Result<Duration, ProbeError> {
    parse_format_duration(path, duration)
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
