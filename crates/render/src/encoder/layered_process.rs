//! `FFmpeg` command and bounded output reader for layered composition.

use std::fmt::Write as _;
use std::io;
use std::path::Path;
use std::process::Stdio;

use onmark_core::model::MediaSource;
use onmark_core::protocol::WireFrameRate;
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncReadExt as _;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use super::error::{EncodeError, EncodeErrorKind};
use super::layered::{
    BackdropMediaInput, CanonicalFrame, LayeredInputs, LayeredJob, LayeredMediaInput, LayeredOutput,
};
use super::limits::EncodeLimits;
use super::process::configure_video_output;
use super::profile::EncodeProfile;
use crate::visual::NativeMediaSchedule;
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
    if job.inputs.media_count() == 0 || job.inputs.media_count() > MAX_MEDIA_INPUTS {
        return Err(job_error(
            job,
            EncodeErrorKind::FrameLimit,
            "layered composition media count is outside the supported process bound",
        ));
    }
    validate_inputs(job)?;
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

fn validate_inputs(job: &LayeredJob) -> Result<(), EncodeError> {
    let valid = match &job.inputs {
        LayeredInputs::VideoBase(media) => {
            let planned = media
                .iter()
                .try_fold(0_u64, |total, media| total.checked_add(media.frames));
            planned == Some(job.frames)
                && media.iter().all(|media| {
                    media.frames != 0
                        && media
                            .source_skip
                            .checked_add(media.frames)
                            .is_some_and(|end| end <= media.schedule.output_frames())
                })
        }
        LayeredInputs::BrowserBase(media) => media.iter().all(|media| {
            media.frames != 0
                && media
                    .source_skip
                    .checked_add(media.frames)
                    .is_some_and(|end| end <= media.schedule.output_frames())
                && media
                    .output_start
                    .checked_add(media.frames)
                    .is_some_and(|end| end <= job.frames)
        }),
    };
    if valid {
        return Ok(());
    }
    Err(job_error(
        job,
        EncodeErrorKind::FrameLimit,
        "layered media placements do not fit the planned output",
    ))
}

pub(super) fn spawn(
    executable: &Path,
    job: &LayeredJob,
    video_encoder_threads: usize,
    profile: EncodeProfile,
) -> Result<Child, EncodeError> {
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
    append_media_inputs(&mut command, &job.inputs);
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
    ]);
    match &job.destination {
        LayeredOutput::Frames => {
            command
                .args([
                    "-map",
                    "[canonical]",
                    "-frames:v",
                    &frames,
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "rgba",
                    "pipe:1",
                ])
                .stdout(Stdio::piped());
        }
        LayeredOutput::Video(output) => {
            configure_layered_video_output(
                &mut command,
                output,
                &frames,
                video_encoder_threads,
                profile,
            );
            command.stdout(Stdio::null());
        }
    }
    command
        .stdin(Stdio::piped())
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

fn append_media_inputs(command: &mut Command, inputs: &LayeredInputs) {
    match inputs {
        LayeredInputs::VideoBase(media) => {
            for media in media {
                command.arg("-i").arg(&media.path);
            }
        }
        LayeredInputs::BrowserBase(media) => {
            for media in media {
                command.arg("-i").arg(&media.path);
            }
        }
    }
}

fn configure_layered_video_output(
    command: &mut Command,
    output: &Path,
    frames: &str,
    video_encoder_threads: usize,
    profile: EncodeProfile,
) {
    command.args(["-map", "[encoded]", "-frames:v", frames]);
    configure_video_output(command, output, video_encoder_threads, profile);
}

fn composition_filter(job: &LayeredJob) -> String {
    match &job.inputs {
        LayeredInputs::VideoBase(media) => foreground_composition_filter(job, media),
        LayeredInputs::BrowserBase(media) => backdrop_composition_filter(job, media),
    }
}

fn foreground_composition_filter(
    job: &LayeredJob,
    media: &[super::layered::LayeredMediaInput],
) -> String {
    let mut filter = String::new();
    for (index, media) in media.iter().enumerate() {
        append_media_filter(&mut filter, index, media, job.output_frame_rate);
    }
    let base = append_concat_filter(&mut filter, media.len());
    let output = match job.destination {
        LayeredOutput::Video(_) => "format=rgba[encoded]",
        LayeredOutput::Frames => "format=rgba[canonical]",
    };
    write!(
        filter,
        "{base}[{}:v]overlay=shortest=1:format=rgb,{output}",
        media.len(),
    )
    .expect("writing an FFmpeg filter into a String cannot fail");
    filter
}

fn backdrop_composition_filter(job: &LayeredJob, media: &[BackdropMediaInput]) -> String {
    let mut filter = String::new();
    let browser_input = media.len();
    write!(filter, "[{browser_input}:v]format=rgba[canvas0];")
        .expect("writing an FFmpeg filter into a String cannot fail");

    for (index, media) in media.iter().enumerate() {
        append_backdrop_media_filter(&mut filter, index, media, job.output_frame_rate);
        let next = index + 1;
        write!(
            filter,
            concat!(
                "[canvas{index}][media{index}]overlay=",
                "x={x}:y={y}:eof_action=pass:repeatlast=0:shortest=0:format=rgb",
                "[canvas{next}];",
            ),
            index = index,
            next = next,
            x = media.destination_region.x(),
            y = media.destination_region.y(),
        )
        .expect("writing an FFmpeg filter into a String cannot fail");
    }

    let output = match job.destination {
        LayeredOutput::Video(_) => "format=rgba[encoded]",
        LayeredOutput::Frames => "format=rgba[canonical]",
    };
    write!(filter, "[canvas{}]{output}", media.len())
        .expect("writing an FFmpeg filter into a String cannot fail");
    filter
}

fn append_backdrop_media_filter(
    filter: &mut String,
    index: usize,
    media: &BackdropMediaInput,
    output_rate: WireFrameRate,
) {
    let selection = source_selection_filter(media.source_frame_rate, output_rate, media.source);
    append_source_schedule(filter, index, &selection, media.schedule, output_rate);
    let end = media
        .source_skip
        .checked_add(media.frames)
        .expect("validated backdrop source frames fit their accounting domain");
    write!(
        filter,
        concat!(
            "[source{index}]",
            "trim=start_frame={skip}:end_frame={end},",
            "setpts=PTS-STARTPTS+{start}*{rate_denominator}/",
            "({rate_numerator}*TB),",
            "crop={source_width}:{source_height}:{source_x}:{source_y},",
            "scale={destination_width}:{destination_height}:flags=bicubic:",
            "in_range=limited:in_color_matrix=bt709:",
            "out_range=full:out_color_matrix=bt709,format=rgba[media{index}];",
        ),
        index = index,
        skip = media.source_skip,
        end = end,
        start = media.output_start,
        rate_numerator = output_rate.numerator(),
        rate_denominator = output_rate.denominator(),
        source_x = media.source_region.x(),
        source_y = media.source_region.y(),
        source_width = media.source_region.width(),
        source_height = media.source_region.height(),
        destination_width = media.destination_region.width(),
        destination_height = media.destination_region.height(),
    )
    .expect("writing an FFmpeg filter into a String cannot fail");
}

fn append_media_filter(
    filter: &mut String,
    index: usize,
    media: &LayeredMediaInput,
    output_rate: WireFrameRate,
) {
    let selection = source_selection_filter(media.source_frame_rate, output_rate, media.source);
    append_source_schedule(filter, index, &selection, media.schedule, output_rate);
    let end = media
        .source_skip
        .checked_add(media.frames)
        .expect("validated layered source frames fit their accounting domain");
    write!(
        filter,
        concat!(
            "[source{index}]trim=start_frame={skip}:end_frame={end},",
            "setpts=PTS-STARTPTS,",
            "scale=in_range=limited:in_color_matrix=bt709:",
            "out_range=full:out_color_matrix=bt709,format=rgba[base{index}];",
        ),
        index = index,
        skip = media.source_skip,
        end = end,
    )
    .expect("writing an FFmpeg filter into a String cannot fail");
}

fn append_source_schedule(
    filter: &mut String,
    index: usize,
    selection: &str,
    schedule: NativeMediaSchedule,
    output_rate: WireFrameRate,
) {
    let playback = schedule.playback_frames();
    let held = schedule.output_frames() - playback;

    match (playback, held) {
        (0, held) => {
            append_held_source(
                filter,
                index,
                held,
                schedule,
                output_rate,
                HoldBranch::WholeSource,
            );
        }
        (playback, 0) => {
            write!(
                filter,
                concat!(
                    "[{index}:v]{selection},",
                    "trim=end_frame={playback},setpts=PTS-STARTPTS[source{index}];",
                ),
                index = index,
                selection = selection,
                playback = playback,
            )
            .expect("writing an FFmpeg filter into a String cannot fail");
        }
        (playback, held) => {
            // The second branch consumes the same decode but emits only the
            // selected interval's final frame. No complete pass is cached.
            write!(filter, "[{index}:v]split=2[selected{index}][final{index}];")
                .expect("writing an FFmpeg filter into a String cannot fail");
            write!(
                filter,
                "[selected{index}]{selection},trim=end_frame={playback},\
                 setpts=PTS-STARTPTS[playback{index}];",
            )
            .expect("writing an FFmpeg filter into a String cannot fail");
            append_held_source(
                filter,
                index,
                held,
                schedule,
                output_rate,
                HoldBranch::SplitFinal,
            );
            write!(
                filter,
                "[playback{index}][held{index}]concat=n=2:v=1:a=0[source{index}];",
            )
            .expect("writing an FFmpeg filter into a String cannot fail");
        }
    }
}

fn append_held_source(
    filter: &mut String,
    index: usize,
    frames: u64,
    schedule: NativeMediaSchedule,
    output_rate: WireFrameRate,
    branch: HoldBranch,
) {
    branch.write_input(filter, index);
    let last = schedule.final_source_frame();
    let end = last + 1;
    let padding = frames - 1;
    write!(
        filter,
        concat!(
            "trim=start_frame={last}:end_frame={end},setpts=PTS-STARTPTS,",
            "fps=fps={rate_numerator}/{rate_denominator}:",
            "round=near:start_time=0:eof_action=pass,",
            "tpad=stop_mode=clone:stop={padding},",
            "setpts=N*{rate_denominator}/({rate_numerator}*TB)",
        ),
        last = last,
        end = end,
        padding = padding,
        rate_numerator = output_rate.numerator(),
        rate_denominator = output_rate.denominator(),
    )
    .expect("writing an FFmpeg filter into a String cannot fail");
    branch.write_output(filter, index);
    filter.push(';');
}

#[derive(Clone, Copy)]
enum HoldBranch {
    WholeSource,
    SplitFinal,
}

impl HoldBranch {
    fn write_input(self, filter: &mut String, index: usize) {
        match self {
            Self::WholeSource => write!(filter, "[{index}:v]"),
            Self::SplitFinal => write!(filter, "[final{index}]"),
        }
        .expect("writing an FFmpeg filter into a String cannot fail");
    }

    fn write_output(self, filter: &mut String, index: usize) {
        match self {
            Self::WholeSource => write!(filter, "[source{index}]"),
            Self::SplitFinal => write!(filter, "[held{index}]"),
        }
        .expect("writing an FFmpeg filter into a String cannot fail");
    }
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

fn source_selection_filter(
    source_rate: WireFrameRate,
    output_rate: WireFrameRate,
    source: MediaSource,
) -> String {
    let start = source.interval().start().as_nanos();
    let speed = source.playback_rate();
    // The explicit midpoint formula is the Rust-owned frame-selection policy;
    // FFmpeg only realizes its projected PTS by dropping or repeating frames.
    format!(
        concat!(
            "setpts='ceil(((N*{source_denominator}*1000000000-",
            "{start}*{source_numerator})*{output_numerator}*{speed_denominator})/",
            "({source_numerator}*1000000000*{output_denominator}*",
            "{speed_numerator})-0.5)*",
            "{output_denominator}/({output_numerator}*TB)',",
            "fps=fps={output_numerator}/{output_denominator}:round=near:start_time=0",
        ),
        source_numerator = source_rate.numerator(),
        source_denominator = source_rate.denominator(),
        start = start,
        output_numerator = output_rate.numerator(),
        output_denominator = output_rate.denominator(),
        speed_numerator = speed.numerator(),
        speed_denominator = speed.denominator(),
    )
}

pub(super) async fn read_frames(
    mut output: tokio::process::ChildStdout,
    frame_bytes: usize,
    frame_count: u64,
    sender: mpsc::Sender<CanonicalFrame>,
) -> io::Result<()> {
    for _ in 0..frame_count {
        let mut pixels = vec![0; frame_bytes];
        output.read_exact(&mut pixels).await?;
        let fingerprint = RawRgbaHash::from_bytes(Sha256::digest(&pixels).into());
        if sender
            .send(CanonicalFrame::new(pixels.into_boxed_slice(), fingerprint))
            .await
            .is_err()
        {
            return Ok(());
        }
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

    use onmark_core::model::{
        Duration, FrameRate, MediaSource, MediaSourceInterval, PlayCount, PlaybackRate,
    };

    use super::{composition_filter, source_selection_filter};
    use crate::RenderProfile;
    use crate::encoder::{
        BackdropMediaInput, LayeredInputs, LayeredJob, LayeredMediaInput, LayeredOutput,
    };
    use crate::visual::{NativeMediaSchedule, PixelRegion};

    #[test]
    fn owns_the_exact_midpoint_frame_selection_formula() {
        let source = FrameRate::new(24, 1).expect("the source rate is valid");
        let output = FrameRate::new(30, 1).expect("the output rate is valid");

        assert_eq!(
            source_selection_filter(source.into(), output.into(), identity_source()),
            concat!(
                "setpts='ceil(((N*1*1000000000-0*24)*30*1)/",
                "(24*1000000000*1*1)-0.5)*1/(30*TB)',",
                "fps=fps=30/1:round=near:start_time=0",
            ),
        );
    }

    #[test]
    fn projects_trim_and_speed_into_the_same_midpoint_formula() {
        let rate = FrameRate::new(30, 1).expect("the fixture rate is valid");
        let source = media_source(250_000_000, 750_000_000, 2, 1);

        assert_eq!(
            source_selection_filter(rate.into(), rate.into(), source),
            concat!(
                "setpts='ceil(((N*1*1000000000-250000000*30)*30*1)/",
                "(30*1000000000*1*2)-0.5)*1/(30*TB)',",
                "fps=fps=30/1:round=near:start_time=0",
            ),
        );
    }

    #[test]
    fn concatenates_partition_media_before_one_foreground_composition() {
        let rate = FrameRate::new(30, 1).expect("the fixture rate is valid");
        let job = LayeredJob {
            inputs: LayeredInputs::VideoBase(vec![
                media("first.mp4", 10, rate),
                media("second.mp4", 20, rate),
            ]),
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

    #[test]
    fn video_output_does_not_duplicate_frames_onto_stdout() {
        let rate = FrameRate::new(30, 1).expect("the fixture rate is valid");
        let job = LayeredJob {
            inputs: LayeredInputs::VideoBase(vec![media("film.mp4", 30, rate)]),
            output_frame_rate: rate.into(),
            frames: 30,
            profile: RenderProfile::new(320, 180).expect("the fixture profile is valid"),
            destination: LayeredOutput::Video(PathBuf::from("output.mp4")),
            diagnostic_path: PathBuf::from("output.mp4"),
        };

        let filter = composition_filter(&job);

        assert!(filter.ends_with("[base0][1:v]overlay=shortest=1:format=rgb,format=rgba[encoded]"));
        assert!(!filter.contains("split="));
        assert!(!filter.contains("[canonical]"));
    }

    #[test]
    fn trims_layered_media_to_the_published_source_window() {
        let rate = FrameRate::new(30, 1).expect("the fixture rate is valid");
        let mut media = media("film.mp4", 1, rate);
        media.source_skip = 17;
        let job = LayeredJob {
            inputs: LayeredInputs::VideoBase(vec![media]),
            output_frame_rate: rate.into(),
            frames: 1,
            profile: RenderProfile::new(320, 180).expect("the fixture profile is valid"),
            destination: LayeredOutput::Frames,
            diagnostic_path: PathBuf::from("artifact.onmark-frames"),
        };

        let filter = composition_filter(&job);

        assert!(filter.contains("trim=start_frame=17:end_frame=18"));
    }

    #[test]
    fn realizes_a_hold_from_the_selected_source_interval_final_frame() {
        let rate = FrameRate::new(30, 1).expect("the fixture rate is valid");
        let mut media = media("film.mp4", 45, rate);
        media.source = held_source();
        media.schedule = schedule(30, 45, 29);
        let job = LayeredJob {
            inputs: LayeredInputs::VideoBase(vec![media]),
            output_frame_rate: rate.into(),
            frames: 45,
            profile: RenderProfile::new(320, 180).expect("the fixture profile is valid"),
            destination: LayeredOutput::Frames,
            diagnostic_path: PathBuf::from("artifact.onmark-frames"),
        };

        let filter = composition_filter(&job);

        assert!(filter.contains("[0:v]split=2[selected0][final0]"));
        assert!(filter.contains("[final0]trim=start_frame=29:end_frame=30"));
        assert!(filter.contains("tpad=stop_mode=clone:stop=14"));
        assert!(filter.contains("[playback0][held0]concat=n=2:v=1:a=0[source0]"));
        assert!(!filter.contains("loop="));
    }

    #[test]
    fn realizes_a_hold_that_owns_the_first_output_midpoint() {
        let rate = FrameRate::new(2, 1).expect("the fixture rate is valid");
        let mut media = media("film.mp4", 2, rate);
        media.schedule = schedule(0, 2, 0);
        let job = LayeredJob {
            inputs: LayeredInputs::VideoBase(vec![media]),
            output_frame_rate: rate.into(),
            frames: 2,
            profile: RenderProfile::new(320, 180).expect("the fixture profile is valid"),
            destination: LayeredOutput::Frames,
            diagnostic_path: PathBuf::from("artifact.onmark-frames"),
        };

        let filter = composition_filter(&job);

        assert!(!filter.contains("split=2"));
        assert!(filter.contains("[0:v]trim=start_frame=0:end_frame=1"));
        assert!(filter.contains("tpad=stop_mode=clone:stop=1"));
    }

    #[test]
    fn places_native_media_above_the_browser_backdrop() {
        let rate = FrameRate::new(30, 1).expect("the fixture rate is valid");
        let job = LayeredJob {
            inputs: LayeredInputs::BrowserBase(vec![BackdropMediaInput {
                path: PathBuf::from("card.mp4"),
                source_frame_rate: rate.into(),
                source: identity_source(),
                schedule: schedule(30, 30, 29),
                source_skip: 2,
                output_start: 5,
                frames: 10,
                source_region: PixelRegion::new(420, 0, 1_080, 1_080),
                destination_region: PixelRegion::new(100, 40, 320, 180),
            }]),
            output_frame_rate: rate.into(),
            frames: 30,
            profile: RenderProfile::new(640, 360).expect("the fixture profile is valid"),
            destination: LayeredOutput::Frames,
            diagnostic_path: PathBuf::from("artifact.onmark-frames"),
        };

        let filter = composition_filter(&job);

        assert!(filter.contains("[1:v]format=rgba[canvas0]"));
        assert!(filter.contains("trim=start_frame=2:end_frame=12"));
        assert!(filter.contains("crop=1080:1080:420:0"));
        assert!(filter.contains("scale=320:180"));
        assert!(filter.contains("overlay=x=100:y=40"));
        assert!(filter.ends_with("[canvas1]format=rgba[canonical]"));
    }

    fn media(path: &str, frames: u64, rate: FrameRate) -> LayeredMediaInput {
        LayeredMediaInput {
            path: PathBuf::from(path),
            source_frame_rate: rate.into(),
            source: identity_source(),
            schedule: schedule(frames, frames, frames.saturating_sub(1)),
            source_skip: 0,
            frames,
        }
    }

    fn schedule(
        playback_frames: u64,
        output_frames: u64,
        final_source_frame: u64,
    ) -> NativeMediaSchedule {
        NativeMediaSchedule::new(playback_frames, output_frames, final_source_frame)
            .expect("the fixture schedule is valid")
    }

    fn identity_source() -> MediaSource {
        media_source(0, 1_000_000_000, 1, 1)
    }

    fn held_source() -> MediaSource {
        let interval =
            MediaSourceInterval::new(Duration::ZERO, Duration::from_nanos(1_000_000_000))
                .expect("the fixture source interval is valid");
        MediaSource::new(
            interval,
            PlaybackRate::ONE,
            PlayCount::ONE,
            Duration::from_nanos(500_000_000),
            Duration::from_nanos(1_000_000_000),
        )
        .expect("the fixture source selection is valid")
    }

    fn media_source(
        start: u64,
        end: u64,
        speed_numerator: u32,
        speed_denominator: u32,
    ) -> MediaSource {
        let interval =
            MediaSourceInterval::new(Duration::from_nanos(start), Duration::from_nanos(end))
                .expect("the fixture source interval is valid");
        let speed = PlaybackRate::new(speed_numerator, speed_denominator)
            .expect("the fixture playback rate is valid");
        MediaSource::new(
            interval,
            speed,
            PlayCount::ONE,
            Duration::ZERO,
            Duration::from_nanos(1_000_000_000),
        )
        .expect("the fixture source selection is valid")
    }
}
