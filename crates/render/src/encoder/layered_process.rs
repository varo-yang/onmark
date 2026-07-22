//! `FFmpeg` command and bounded output reader for layered composition.

use std::fmt::Write as _;
use std::io;
use std::path::Path;
use std::process::Stdio;

use onmark_core::protocol::WireFrameRate;
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncReadExt as _;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use super::error::{EncodeError, EncodeErrorKind};
use super::layered::{CanonicalFrame, LayeredJob};
use super::limits::EncodeLimits;
use crate::{RawRgbaHash, RenderProfile};

const MAX_MEDIA_INPUTS: usize = 64;
const RGBA_CHANNELS: u64 = 4;

pub(super) fn validate_job(job: &LayeredJob, limits: EncodeLimits) -> Result<(), EncodeError> {
    let frames = job.frame_count();
    if frames == 0 {
        return Err(job_error(
            job,
            EncodeErrorKind::NoFrames,
            "layered composition output cannot be empty",
        ));
    }
    if frames > limits.max_frames() {
        return Err(job_error(
            job,
            EncodeErrorKind::FrameLimit,
            "layered composition exceeds the configured frame limit",
        ));
    }
    if job.media.is_empty() || job.media.len() > MAX_MEDIA_INPUTS {
        return Err(job_error(
            job,
            EncodeErrorKind::FrameLimit,
            "layered composition media count is outside the supported process bound",
        ));
    }
    let planned_frames = job
        .media
        .iter()
        .try_fold(0_u64, |total, media| total.checked_add(media.frames));
    if planned_frames != Some(frames) || job.media.iter().any(|media| media.frames == 0) {
        return Err(job_error(
            job,
            EncodeErrorKind::FrameLimit,
            "layered media segments do not match the planned output frame count",
        ));
    }
    if let Some(output) = job.destination.video_path()
        && output.exists()
    {
        return Err(EncodeError::new(
            EncodeErrorKind::OutputExists,
            output,
            "output already exists",
        ));
    }
    Ok(())
}

pub(super) fn spawn(executable: &Path, job: &LayeredJob) -> Result<Child, EncodeError> {
    let rate = frame_rate(job.output_frame_rate);
    let frames = job.frame_count().to_string();
    let filter = composition_filter(job);
    let mut command = Command::new(executable);
    command.args([
        "-nostdin",
        "-loglevel",
        "error",
        "-filter_complex_threads",
        "1",
        "-threads",
        "1",
    ]);
    for media in &job.media {
        command.arg("-i").arg(&media.path);
    }
    let dimensions = format!("{}x{}", job.profile.width(), job.profile.height());
    command.args([
        "-f",
        "rawvideo",
        "-framerate",
        &rate,
        "-video_size",
        &dimensions,
        "-pixel_format",
        "rgba",
        "-i",
        "pipe:0",
        "-filter_complex",
        &filter,
        "-map",
        "[canonical]",
        "-frames:v",
        &frames,
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "pipe:1",
    ]);
    if let Some(output) = job.destination.video_path() {
        configure_video_output(&mut command, output, &frames);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| {
            EncodeError::io(
                EncodeErrorKind::Spawn,
                &job.diagnostic_path,
                "failed to start layered FFmpeg composition",
                source,
            )
        })
}

fn configure_video_output(command: &mut Command, output: &Path, frames: &str) {
    command
        .args([
            "-map",
            "[encoded]",
            "-frames:v",
            frames,
            "-an",
            "-c:v",
            "libx264",
            "-threads",
            "1",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-color_range",
            "tv",
            "-f",
            "mp4",
            "-n",
        ])
        .arg(output);
}

fn composition_filter(job: &LayeredJob) -> String {
    let mut filter = String::new();
    for (index, media) in job.media.iter().enumerate() {
        append_media_filter(&mut filter, index, media, job.output_frame_rate);
    }
    let base = append_concat_filter(&mut filter, job.media.len());
    let output = if job.destination.video_path().is_some() {
        "format=rgba,split=2[canonical][encoded]"
    } else {
        "format=rgba[canonical]"
    };
    write!(
        filter,
        "{base}[{}:v]overlay=shortest=1:format=rgb,{output}",
        job.media.len(),
    )
    .expect("writing an FFmpeg filter into a String cannot fail");
    filter
}

fn append_media_filter(
    filter: &mut String,
    index: usize,
    media: &super::layered::LayeredMediaInput,
    output_rate: WireFrameRate,
) {
    let selection = source_selection_filter(media.source_frame_rate, output_rate);
    write!(
        filter,
        concat!(
            "[{index}:v]{selection},trim=start_frame=0:end_frame={frames},",
            "setpts=PTS-STARTPTS,",
            "scale=in_range=limited:in_color_matrix=bt709:",
            "out_range=full:out_color_matrix=bt709,format=rgba[base{index}];",
        ),
        index = index,
        selection = selection,
        frames = media.frames,
    )
    .expect("writing an FFmpeg filter into a String cannot fail");
}

fn append_concat_filter(filter: &mut String, inputs: usize) -> &'static str {
    if inputs == 1 {
        return "[base0]";
    }
    for index in 0..inputs {
        write!(filter, "[base{index}]")
            .expect("writing an FFmpeg filter into a String cannot fail");
    }
    write!(filter, "concat=n={inputs}:v=1:a=0[base];")
        .expect("writing an FFmpeg filter into a String cannot fail");
    "[base]"
}

fn source_selection_filter(source: WireFrameRate, output: WireFrameRate) -> String {
    // The explicit midpoint formula is the Rust-owned frame-selection policy;
    // FFmpeg only realizes its projected PTS by dropping or repeating frames.
    format!(
        concat!(
            "setpts='ceil(N*{output_numerator}*{source_denominator}/",
            "({output_denominator}*{source_numerator})-0.5)*",
            "{output_denominator}/({output_numerator}*TB)',",
            "fps=fps={output_numerator}/{output_denominator}:round=near",
        ),
        source_numerator = source.numerator(),
        source_denominator = source.denominator(),
        output_numerator = output.numerator(),
        output_denominator = output.denominator(),
    )
}

pub(super) async fn read_frames(
    mut output: tokio::process::ChildStdout,
    frame_bytes: usize,
    frame_count: u64,
    retains_pixels: bool,
    sender: mpsc::Sender<CanonicalFrame>,
) -> io::Result<()> {
    let receiver_open = if retains_pixels {
        retain_frames(&mut output, frame_bytes, frame_count, &sender).await?
    } else {
        drain_frames(&mut output, frame_bytes, frame_count, &sender).await?
    };
    if !receiver_open {
        return Ok(());
    }

    let mut trailing = [0];
    if output.read(&mut trailing).await? != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "FFmpeg emitted bytes after the planned composed frames",
        ));
    }
    Ok(())
}

async fn retain_frames(
    output: &mut tokio::process::ChildStdout,
    frame_bytes: usize,
    frame_count: u64,
    sender: &mpsc::Sender<CanonicalFrame>,
) -> io::Result<bool> {
    for _ in 0..frame_count {
        let mut pixels = vec![0; frame_bytes];
        output.read_exact(&mut pixels).await?;
        let fingerprint = RawRgbaHash::from_bytes(Sha256::digest(&pixels).into());
        let frame = CanonicalFrame::Pixels {
            bytes: pixels.into_boxed_slice(),
            fingerprint,
        };
        if sender.send(frame).await.is_err() {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn drain_frames(
    output: &mut tokio::process::ChildStdout,
    frame_bytes: usize,
    frame_count: u64,
    sender: &mpsc::Sender<CanonicalFrame>,
) -> io::Result<bool> {
    let mut pixels = vec![0; frame_bytes];
    for _ in 0..frame_count {
        output.read_exact(&mut pixels).await?;
        if sender.send(CanonicalFrame::Consumed).await.is_err() {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn frame_bytes(profile: RenderProfile, output: &Path) -> Result<usize, EncodeError> {
    u64::from(profile.width())
        .checked_mul(u64::from(profile.height()))
        .and_then(|pixels| pixels.checked_mul(RGBA_CHANNELS))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or_else(|| {
            EncodeError::new(
                EncodeErrorKind::FrameRead,
                output,
                "render profile exceeds layered-frame accounting",
            )
        })
}

pub(super) fn take_pipe<T>(pipe: Option<T>, output: &Path, name: &str) -> Result<T, EncodeError> {
    pipe.ok_or_else(|| {
        EncodeError::new(
            EncodeErrorKind::Spawn,
            output,
            format!("layered FFmpeg started without its configured {name} pipe"),
        )
    })
}

fn frame_rate(rate: WireFrameRate) -> String {
    format!("{}/{}", rate.numerator(), rate.denominator())
}

fn job_error(job: &LayeredJob, kind: EncodeErrorKind, message: &'static str) -> EncodeError {
    EncodeError::new(kind, &job.diagnostic_path, message)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use onmark_core::model::FrameRate;

    use super::{composition_filter, source_selection_filter};
    use crate::RenderProfile;
    use crate::encoder::{LayeredJob, LayeredMediaInput, LayeredOutput};

    #[test]
    fn owns_the_exact_midpoint_frame_selection_formula() {
        let source = FrameRate::new(24, 1).expect("the source rate is valid");
        let output = FrameRate::new(30, 1).expect("the output rate is valid");

        assert_eq!(
            source_selection_filter(source.into(), output.into()),
            "setpts='ceil(N*30*1/(1*24)-0.5)*1/(30*TB)',fps=fps=30/1:round=near",
        );
    }

    #[test]
    fn concatenates_partition_media_before_one_foreground_composition() {
        let rate = FrameRate::new(30, 1).expect("the fixture rate is valid");
        let job = LayeredJob {
            media: vec![media("first.mp4", 10, rate), media("second.mp4", 20, rate)],
            output_frame_rate: rate.into(),
            frames: 30,
            profile: RenderProfile::new(320, 180).expect("the fixture profile is valid"),
            destination: LayeredOutput::Frames,
            diagnostic_path: PathBuf::from("artifact.onmark-frames"),
        };

        let filter = composition_filter(&job);

        assert!(filter.contains("[base0][base1]concat=n=2:v=1:a=0[base];"));
        assert!(
            filter.ends_with("[base][2:v]overlay=shortest=1:format=rgb,format=rgba[canonical]")
        );
    }

    fn media(path: &str, frames: u64, rate: FrameRate) -> LayeredMediaInput {
        LayeredMediaInput {
            path: PathBuf::from(path),
            source_frame_rate: rate.into(),
            frames,
        }
    }
}
