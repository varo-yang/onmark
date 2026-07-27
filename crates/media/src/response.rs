//! Normalization of untrusted ffprobe JSON into stable core metadata.
//!
//! Stream facts identify tracks and formats. A second bounded frame probe
//! supplies the complete presentation timestamps that prove CFR or carry VFR.

use std::path::Path;

use onmark_core::model::{
    AssetMetadata, AudioChannelLayout, AudioMetadata, AudioSampleRate, Duration, FrameRate,
    MediaTimebase, VideoColorProfile, VideoDimensions, VideoFrameMap, VideoMetadata, VideoTiming,
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
    sample_rate: Option<Box<str>>,
    channels: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
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

/// Track facts retained while a visual stream receives its exact timing probe.
pub(crate) struct PendingMetadata {
    format_duration: Option<Duration>,
    audio: Option<AudioMetadata>,
    video: Option<PendingVideo>,
}

impl PendingMetadata {
    pub(crate) fn frame_probe_request(&self) -> Option<FrameProbeRequest> {
        self.video.as_ref().map(|video| FrameProbeRequest {
            stream_index: video.stream_index,
        })
    }

    pub(crate) fn finish(
        self,
        path: &Path,
        timing: Option<ProbedVideoTiming>,
    ) -> Result<AssetMetadata, ProbeError> {
        let video = self
            .video
            .map(|video| {
                let timing = timing
                    .ok_or_else(|| ProbeError::invalid_video(path, "frame timing is missing"))?;
                video.finish(path, timing)
            })
            .transpose()?;
        let duration = self
            .format_duration
            .or_else(|| {
                longest_duration(
                    video.as_ref().map(VideoMetadata::duration),
                    self.audio.as_ref().map(AudioMetadata::duration),
                )
            })
            .ok_or_else(|| ProbeError::missing_duration(path))?;

        Ok(match (self.audio, video) {
            (Some(audio), Some(video)) => AssetMetadata::audio_video(duration, audio, video),
            (Some(audio), None) => AssetMetadata::audio_only(duration, audio),
            (None, Some(video)) => AssetMetadata::video(duration, video),
            (None, None) => AssetMetadata::without_media_tracks(duration),
        })
    }
}

/// Selected visual-stream identity needed by the frame probe.
#[derive(Clone, Copy)]
pub(crate) struct FrameProbeRequest {
    stream_index: u32,
}

impl FrameProbeRequest {
    pub(crate) const fn stream_index(self) -> u32 {
        self.stream_index
    }
}

/// Complete timing facts returned by the selected-stream frame probe.
pub(crate) struct ProbedVideoTiming {
    timing: VideoTiming,
    duration: Duration,
}

pub(crate) fn parse_metadata(path: &Path, bytes: &[u8]) -> Result<PendingMetadata, ProbeError> {
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
        .map(|stream| parse_video(path, stream))
        .transpose()?;
    let audio = audio_stream
        .map(|stream| parse_audio(path, stream, format_duration))
        .transpose()?;

    Ok(PendingMetadata {
        format_duration,
        audio,
        video,
    })
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

struct PendingVideo {
    stream_index: u32,
    dimensions: VideoDimensions,
    codec: Box<str>,
    pixel_format: Box<str>,
    color_profile: Option<VideoColorProfile>,
}

impl PendingVideo {
    fn finish(self, path: &Path, timing: ProbedVideoTiming) -> Result<VideoMetadata, ProbeError> {
        let metadata = VideoMetadata::new(
            timing.duration,
            self.dimensions,
            self.codec,
            self.pixel_format,
            timing.timing,
        )
        .map_err(|source| ProbeError::invalid_video(path, source.to_string()))?;
        Ok(match self.color_profile {
            Some(profile) => metadata.with_color_profile(profile),
            None => metadata,
        })
    }
}

fn parse_video(path: &Path, stream: ProbeStream) -> Result<PendingVideo, ProbeError> {
    let stream_index = required_field(path, "stream index", stream.index)?;
    let color_profile = parse_color_profile(&stream);
    let codec = required_field(path, "codec name", stream.codec_name)?;
    let pixel_format = required_field(path, "pixel format", stream.pix_fmt)?;
    let width = required_field(path, "width", stream.width)?;
    let height = required_field(path, "height", stream.height)?;
    let dimensions = VideoDimensions::new(width, height)
        .map_err(|source| ProbeError::invalid_video(path, source))?;

    Ok(PendingVideo {
        stream_index,
        dimensions,
        codec,
        pixel_format,
        color_profile,
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

#[derive(Deserialize)]
struct FrameProbeResponse {
    #[serde(default)]
    frames: Vec<ProbeFrame>,
    #[serde(default)]
    streams: Vec<FrameProbeStream>,
}

#[derive(Deserialize)]
struct ProbeFrame {
    best_effort_timestamp: Option<ProbeInteger>,
    pkt_duration: Option<ProbeInteger>,
}

#[derive(Deserialize)]
struct FrameProbeStream {
    time_base: Option<Box<str>>,
    duration_ts: Option<ProbeInteger>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ProbeInteger {
    Text(Box<str>),
    Signed(i64),
    Unsigned(u64),
}

pub(crate) fn parse_frame_timing(
    path: &Path,
    bytes: &[u8],
) -> Result<ProbedVideoTiming, ProbeError> {
    let response = serde_json::from_slice::<FrameProbeResponse>(bytes)
        .map_err(|source| ProbeError::invalid_response(path, source))?;
    let [stream] = response.streams.as_slice() else {
        return Err(ProbeError::invalid_video(
            path,
            "frame probe must report exactly one selected video stream",
        ));
    };
    let timebase = parse_media_timebase(path, stream.time_base.as_deref())?;
    let mut timestamps = parse_frame_timestamps(path, response.frames)?;
    if timestamps.is_empty() {
        return Err(ProbeError::invalid_video(
            path,
            "video stream contains no frames",
        ));
    }
    timestamps.sort_unstable_by_key(|frame| frame.timestamp);

    let origin = timestamps[0].timestamp;
    let mut boundaries = Vec::with_capacity(timestamps.len() + 1);
    for frame in &timestamps {
        let relative = frame
            .timestamp
            .checked_sub(origin)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| {
                ProbeError::invalid_video(path, "video frame timestamps cannot be normalized")
            })?;
        boundaries.push(relative);
    }
    let terminal = terminal_timestamp(path, stream, &timestamps, origin)?;
    boundaries.push(terminal);

    let duration = timebase
        .duration_at(terminal)
        .map_err(|source| ProbeError::invalid_video(path, source))?;
    let timing = classify_frame_timing(path, timebase, boundaries)?;
    Ok(ProbedVideoTiming { timing, duration })
}

struct ParsedFrame {
    timestamp: i64,
    duration: Option<u64>,
}

fn parse_frame_timestamps(
    path: &Path,
    frames: Vec<ProbeFrame>,
) -> Result<Vec<ParsedFrame>, ProbeError> {
    frames
        .into_iter()
        .map(|frame| {
            let timestamp = required_video_integer(
                path,
                "best-effort frame timestamp",
                frame.best_effort_timestamp.as_ref(),
            )?;
            let duration =
                optional_video_integer(path, "packet duration", frame.pkt_duration.as_ref())?;
            Ok(ParsedFrame {
                timestamp,
                duration,
            })
        })
        .collect()
}

fn terminal_timestamp(
    path: &Path,
    stream: &FrameProbeStream,
    frames: &[ParsedFrame],
    origin: i64,
) -> Result<u64, ProbeError> {
    let last = frames
        .last()
        .expect("the caller rejects an empty frame sequence");
    let last_start = last.timestamp.checked_sub(origin).ok_or_else(|| {
        ProbeError::invalid_video(path, "video frame timestamps cannot be normalized")
    })?;
    let last_start = u64::try_from(last_start)
        .map_err(|_| ProbeError::invalid_video(path, "video timestamp exceeds its domain"))?;

    let terminal =
        match optional_video_integer(path, "stream duration", stream.duration_ts.as_ref())? {
            Some(duration) => duration,
            None => last_start
                .checked_add(last.duration.ok_or_else(|| {
                    ProbeError::invalid_video(path, "final video frame duration is missing")
                })?)
                .ok_or_else(|| {
                    ProbeError::invalid_video(path, "video timestamp exceeds its domain")
                })?,
        };
    if terminal <= last_start {
        return Err(ProbeError::invalid_video(
            path,
            "video stream ends before its final frame",
        ));
    }
    Ok(terminal)
}

fn classify_frame_timing(
    path: &Path,
    timebase: MediaTimebase,
    boundaries: Vec<u64>,
) -> Result<VideoTiming, ProbeError> {
    if boundaries.len() == 2 {
        return Ok(VideoTiming::Still);
    }
    let map = VideoFrameMap::new(timebase, boundaries)
        .map_err(|source| ProbeError::invalid_video(path, source))?;
    let mut intervals = map.boundaries().windows(2);
    let first = intervals
        .next()
        .expect("a variable frame map contains at least two frames");
    let ticks_per_frame = first[1] - first[0];
    if intervals.all(|interval| interval[1] - interval[0] == ticks_per_frame) {
        return constant_frame_rate(path, timebase, ticks_per_frame).map(VideoTiming::Constant);
    }
    Ok(VideoTiming::Variable(map))
}

fn constant_frame_rate(
    path: &Path,
    timebase: MediaTimebase,
    ticks_per_frame: u64,
) -> Result<FrameRate, ProbeError> {
    let numerator = u64::from(timebase.denominator());
    let denominator = u64::from(timebase.numerator())
        .checked_mul(ticks_per_frame)
        .ok_or_else(|| ProbeError::invalid_video(path, "source frame rate exceeds its domain"))?;
    let divisor = greatest_common_divisor(numerator, denominator);
    let numerator = u32::try_from(numerator / divisor)
        .map_err(|_| ProbeError::invalid_video(path, "source frame-rate numerator is too large"))?;
    let denominator = u32::try_from(denominator / divisor).map_err(|_| {
        ProbeError::invalid_video(path, "source frame-rate denominator is too large")
    })?;
    FrameRate::new(numerator, denominator).map_err(|source| ProbeError::invalid_video(path, source))
}

fn parse_media_timebase(path: &Path, value: Option<&str>) -> Result<MediaTimebase, ProbeError> {
    let value =
        value.ok_or_else(|| ProbeError::invalid_video(path, "video stream timebase is missing"))?;
    let invalid =
        || ProbeError::invalid_video(path, format!("video timebase {value:?} is invalid"));
    let (numerator, denominator) = value.split_once('/').ok_or_else(invalid)?;
    let numerator = numerator.parse::<u32>().map_err(|_| invalid())?;
    let denominator = denominator.parse::<u32>().map_err(|_| invalid())?;
    MediaTimebase::new(numerator, denominator)
        .map_err(|source| ProbeError::invalid_video(path, source))
}

fn required_video_integer(
    path: &Path,
    name: &str,
    value: Option<&ProbeInteger>,
) -> Result<i64, ProbeError> {
    let value =
        value.ok_or_else(|| ProbeError::invalid_video(path, format!("{name} is missing")))?;
    match value {
        ProbeInteger::Text(value) => value.parse::<i64>().map_err(|_| {
            ProbeError::invalid_video(path, format!("{name} {value:?} is not an integer"))
        }),
        ProbeInteger::Signed(value) => Ok(*value),
        ProbeInteger::Unsigned(value) => i64::try_from(*value).map_err(|_| {
            ProbeError::invalid_video(path, format!("{name} {value} exceeds its integer domain"))
        }),
    }
}

fn optional_video_integer(
    path: &Path,
    name: &str,
    value: Option<&ProbeInteger>,
) -> Result<Option<u64>, ProbeError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = match value {
        ProbeInteger::Text(value) => value.parse::<u64>().map_err(|_| {
            ProbeError::invalid_video(path, format!("{name} {value:?} is not a positive integer"))
        })?,
        ProbeInteger::Signed(value) => u64::try_from(*value).map_err(|_| {
            ProbeError::invalid_video(path, format!("{name} {value} is not a positive integer"))
        })?,
        ProbeInteger::Unsigned(value) => *value,
    };
    if value == 0 {
        return Err(ProbeError::invalid_video(
            path,
            format!("{name} must be positive"),
        ));
    }
    Ok(Some(value))
}

const fn greatest_common_divisor(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn required_field<T>(path: &Path, name: &str, value: Option<T>) -> Result<T, ProbeError> {
    value.ok_or_else(|| ProbeError::invalid_video(path, format!("video stream has no {name}")))
}

fn required_audio_field<T>(path: &Path, name: &str, value: Option<T>) -> Result<T, ProbeError> {
    value.ok_or_else(|| ProbeError::invalid_audio(path, format!("audio stream has no {name}")))
}
