use std::path::Path;

use onmark_core::model::{AssetMetadata, Duration, FrameRate, VideoMetadata, VideoTiming};
use serde::Deserialize;

use crate::error::ProbeError;

#[derive(Deserialize)]
struct ProbeResponse {
    #[serde(default)]
    frames: Vec<ProbeFrame>,
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Deserialize)]
struct ProbeFrame {
    best_effort_timestamp: Option<i64>,
}

#[derive(Deserialize)]
struct ProbeStream {
    duration: Option<Box<str>>,
    codec_name: Option<Box<str>>,
    pix_fmt: Option<Box<str>>,
    time_base: Option<Box<str>>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    duration: Option<Box<str>>,
}

pub(crate) fn parse_metadata(path: &Path, bytes: &[u8]) -> Result<AssetMetadata, ProbeError> {
    let response = serde_json::from_slice::<ProbeResponse>(bytes)
        .map_err(|source| ProbeError::invalid_response(path, source))?;
    let Some(duration) = response.format.and_then(|format| format.duration) else {
        return Err(ProbeError::missing_duration(path));
    };
    let duration = Duration::parse(&format!("{duration}s"))
        .map_err(|source| ProbeError::invalid_duration(path, &duration, source))?;

    let Some(stream) = response.streams.into_iter().next() else {
        return Ok(AssetMetadata::audio(duration));
    };
    let video = parse_video(path, stream, &response.frames)?;

    Ok(AssetMetadata::video(duration, video))
}

fn parse_video(
    path: &Path,
    stream: ProbeStream,
    frames: &[ProbeFrame],
) -> Result<VideoMetadata, ProbeError> {
    let duration = required_field(path, "duration", stream.duration.as_deref())?;
    let duration = Duration::parse(&format!("{duration}s"))
        .map_err(|source| ProbeError::invalid_video_duration(path, duration, source))?;
    let timing = parse_timing(path, &stream, frames)?;
    let codec = required_field(path, "codec name", stream.codec_name)?;
    let pixel_format = required_field(path, "pixel format", stream.pix_fmt)?;

    VideoMetadata::new(duration, codec, pixel_format, timing)
        .map_err(|source| ProbeError::invalid_video(path, source.to_string()))
}

fn parse_timing(
    path: &Path,
    stream: &ProbeStream,
    frames: &[ProbeFrame],
) -> Result<VideoTiming, ProbeError> {
    let Some((first, remaining)) = frames.split_first() else {
        return Err(ProbeError::invalid_video(
            path,
            "video stream contains no frames",
        ));
    };
    let Some((second, remaining)) = remaining.split_first() else {
        return Ok(VideoTiming::Still);
    };

    let first = frame_timestamp(path, first)?;
    let second = frame_timestamp(path, second)?;
    let interval = positive_interval(path, first, second)?;
    let mut previous = second;

    for frame in remaining {
        let timestamp = frame_timestamp(path, frame)?;
        if positive_interval(path, previous, timestamp)? != interval {
            return Ok(VideoTiming::Variable);
        }
        previous = timestamp;
    }

    let time_base = required_field(path, "time base", stream.time_base.as_deref())?;
    let (ticks, scale) = parse_ratio(path, "time base", time_base)?;
    let denominator = ticks
        .checked_mul(interval)
        .ok_or_else(|| ProbeError::invalid_video(path, "source frame interval is too large"))?;
    let numerator = u32::try_from(scale)
        .map_err(|_| ProbeError::invalid_video(path, "time-base scale is too large"))?;
    let denominator = u32::try_from(denominator)
        .map_err(|_| ProbeError::invalid_video(path, "source frame interval is too large"))?;
    let rate = FrameRate::new(numerator, denominator)
        .map_err(|source| ProbeError::invalid_video(path, source.to_string()))?;

    Ok(VideoTiming::Constant(rate))
}

fn parse_ratio(path: &Path, name: &str, value: &str) -> Result<(u64, u64), ProbeError> {
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

    Ok((numerator, denominator))
}

fn frame_timestamp(path: &Path, frame: &ProbeFrame) -> Result<i64, ProbeError> {
    frame
        .best_effort_timestamp
        .ok_or_else(|| ProbeError::invalid_video(path, "source frame has no timestamp"))
}

fn positive_interval(path: &Path, previous: i64, current: i64) -> Result<u64, ProbeError> {
    let interval = current
        .checked_sub(previous)
        .and_then(|value| u64::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            ProbeError::invalid_video(path, "source frame timestamps are not strictly increasing")
        })?;

    Ok(interval)
}

fn required_field<T>(path: &Path, name: &str, value: Option<T>) -> Result<T, ProbeError> {
    value.ok_or_else(|| ProbeError::invalid_video(path, format!("video stream has no {name}")))
}
