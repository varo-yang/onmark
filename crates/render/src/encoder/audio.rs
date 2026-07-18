//! Final audio mix over an already continuous visual encode.
//!
//! Timeline IR supplies exact frame starts. Rust projects them once onto the
//! fixed output sample grid, so `FFmpeg` receives integer lengths and delays.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use onmark_core::model::{
    AudioChannelLayout, AudioGain, AudioSampleConversionOverflow, AudioSampleCount,
    AudioSampleRate, FrameCount, FrameIndex, FrameRate, Rounding,
};
use onmark_core::protocol::WireFrameRate;
use tempfile::NamedTempFile;
use tokio::process::{Child, Command};
use tokio::runtime::Handle;
use tokio::time::timeout;

use super::error::{EncodeError, EncodeErrorKind};
use super::limits::EncodeLimits;
use super::process::{CapturedStderr, capture_stderr};
use super::session::{EncodedVideo, with_stderr};

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const OUTPUT_SAMPLE_RATE_HZ: u32 = 48_000;

/// One materialized Timeline audio placement for an `FFmpeg` mix operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AudioInput {
    mix_order: usize,
    source: PathBuf,
    start: FrameIndex,
    duration: FrameCount,
    samples: AudioSampleCount,
    channel_layout: AudioChannelLayout,
    gain: AudioGain,
}

impl AudioInput {
    pub(crate) fn new(
        mix_order: usize,
        source: PathBuf,
        start: FrameIndex,
        duration: FrameCount,
        samples: AudioSampleCount,
        channel_layout: AudioChannelLayout,
        gain: AudioGain,
    ) -> Self {
        Self {
            mix_order,
            source,
            start,
            duration,
            samples,
            channel_layout,
            gain,
        }
    }

    pub(crate) const fn mix_order(&self) -> usize {
        self.mix_order
    }

    fn write_filter(&self, output: &mut String, index: usize, placement: AudioPlacement) {
        let stream = index + 1;
        write!(
            output,
            "[{stream}:a]atrim=end_sample={},asetpts=N/SR/TB,",
            self.samples.get(),
        )
        .expect("writing into a String cannot fail");
        write!(
            output,
            "aresample={OUTPUT_SAMPLE_RATE_HZ},{},",
            channel_filter(self.channel_layout),
        )
        .expect("writing into a String cannot fail");
        write!(
            output,
            "aformat=sample_fmts=fltp:sample_rates={OUTPUT_SAMPLE_RATE_HZ}:channel_layouts=stereo,\
             atrim=end_sample={},asetpts=N/SR/TB,",
            placement.samples.get(),
        )
        .expect("writing into a String cannot fail");
        write!(
            output,
            "adelay=delays={}S:all=1,volume={}/{}[audio{index}];",
            placement.delay.get(),
            self.gain.numerator(),
            self.gain.denominator(),
        )
        .expect("writing into a String cannot fail");
    }
}

fn channel_filter(layout: AudioChannelLayout) -> &'static str {
    match layout {
        AudioChannelLayout::Mono => "pan=stereo|c0=c0|c1=c0",
        AudioChannelLayout::Stereo => "anull",
    }
}

#[derive(Clone, Copy)]
struct AudioPlacement {
    delay: AudioSampleCount,
    samples: AudioSampleCount,
}

/// `FFmpeg` must retain access to its private filter script until process exit.
struct RunningAudioMix {
    child: Child,
    _filter_script: NamedTempFile,
}

pub(super) async fn mix_audio(
    executable: &Path,
    limits: EncodeLimits,
    visual: EncodedVideo,
    inputs: Vec<AudioInput>,
    frame_rate: WireFrameRate,
    output: PathBuf,
) -> Result<EncodedVideo, EncodeError> {
    if output.exists() {
        return Err(EncodeError::new(
            EncodeErrorKind::OutputExists,
            &output,
            "output already exists",
        ));
    }

    let frame_rate = model_frame_rate(frame_rate);
    let output_samples = output_samples(visual.frames(), frame_rate, &output)?;

    let runtime = Handle::try_current().map_err(|_| {
        EncodeError::new(
            EncodeErrorKind::Spawn,
            &output,
            "FFmpeg audio mixing requires a Tokio runtime",
        )
    })?;
    let mut process = spawn_audio_mix(
        executable,
        visual.path(),
        &inputs,
        frame_rate,
        output_samples,
        &output,
    )?;
    let Some(stderr) = process.child.stderr.take() else {
        discard_partial_output(&output);
        return Err(EncodeError::new(
            EncodeErrorKind::Spawn,
            &output,
            "FFmpeg started without its configured diagnostic pipe",
        ));
    };
    let stderr = runtime.spawn(capture_stderr(stderr, limits.max_stderr_bytes()));
    let status = match wait_for_mix(&mut process.child, limits.inactivity_timeout(), &output).await
    {
        Ok(status) => status,
        Err(error) => {
            let _ = finish_stderr(stderr, &output).await;
            discard_partial_output(&output);
            return Err(error);
        }
    };
    let stderr = match finish_stderr(stderr, &output).await {
        Ok(stderr) => stderr,
        Err(error) => {
            discard_partial_output(&output);
            return Err(error);
        }
    };

    if !status.success() {
        let message = with_stderr(
            &format!("FFmpeg audio mixing exited with {status}"),
            &stderr,
        );
        discard_partial_output(&output);
        return Err(EncodeError::new(EncodeErrorKind::Failed, &output, message));
    }

    Ok(visual.muxed_at(output))
}

fn spawn_audio_mix(
    executable: &Path,
    visual: &Path,
    inputs: &[AudioInput],
    frame_rate: FrameRate,
    output_samples: AudioSampleCount,
    output: &Path,
) -> Result<RunningAudioMix, EncodeError> {
    let filter = audio_filter(inputs, frame_rate, output_samples).map_err(|source| {
        EncodeError::new(
            EncodeErrorKind::InputLimit,
            output,
            format!("audio placement exceeds its sample domain: {source}"),
        )
    })?;
    let filter = write_filter_script(&filter, output)?;
    let mut command = Command::new(executable);
    command.args(["-nostdin", "-loglevel", "error", "-i"]);
    command.arg(visual);
    for input in inputs {
        command.args(["-i"]);
        command.arg(&input.source);
    }
    let child = command
        .args(["-filter_complex_script"])
        .arg(filter.path())
        .args([
            "-map", "0:v:0", "-map", "[audio]", "-c:v", "copy", "-c:a", "aac",
        ])
        .arg("-ar")
        .arg(OUTPUT_SAMPLE_RATE_HZ.to_string())
        .args(["-ac", "2"])
        .args(["-shortest", "-movflags", "+faststart", "-f", "mp4", "-n"])
        .arg(output)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| {
            EncodeError::io(
                EncodeErrorKind::Spawn,
                output,
                "failed to start FFmpeg audio mixing",
                source,
            )
        })?;

    Ok(RunningAudioMix {
        child,
        _filter_script: filter,
    })
}

fn write_filter_script(filter: &str, output: &Path) -> Result<NamedTempFile, EncodeError> {
    let directory = output.parent().unwrap_or_else(|| Path::new("."));
    let mut script = tempfile::Builder::new()
        .prefix(".onmark-audio-")
        .suffix(".ffscript")
        .tempfile_in(directory)
        .map_err(|source| {
            EncodeError::io(
                EncodeErrorKind::InputWrite,
                output,
                "failed to create the private FFmpeg audio filter script",
                source,
            )
        })?;
    script.write_all(filter.as_bytes()).map_err(|source| {
        EncodeError::io(
            EncodeErrorKind::InputWrite,
            output,
            "failed to write the private FFmpeg audio filter script",
            source,
        )
    })?;
    script.flush().map_err(|source| {
        EncodeError::io(
            EncodeErrorKind::InputWrite,
            output,
            "failed to flush the private FFmpeg audio filter script",
            source,
        )
    })?;
    Ok(script)
}

fn audio_filter(
    inputs: &[AudioInput],
    frame_rate: FrameRate,
    output_samples: AudioSampleCount,
) -> Result<String, AudioSampleConversionOverflow> {
    let mut filter = String::new();
    for (index, input) in inputs.iter().enumerate() {
        let delay = output_sample_rate().samples_for(
            FrameCount::new(input.start.get()),
            frame_rate,
            Rounding::Ceil,
        )?;
        let samples =
            output_sample_rate().samples_for(input.duration, frame_rate, Rounding::Ceil)?;
        input.write_filter(&mut filter, index, AudioPlacement { delay, samples });
    }
    for index in 0..inputs.len() {
        write!(filter, "[audio{index}]").expect("writing into a String cannot fail");
    }
    write!(
        filter,
        "amix=inputs={}:duration=longest:dropout_transition=0:normalize=0[mixed];",
        inputs.len(),
    )
    .expect("writing into a String cannot fail");
    write!(
        filter,
        "[mixed]aresample={OUTPUT_SAMPLE_RATE_HZ},atrim=end_sample={},\
         apad=whole_len={}[audio]",
        output_samples.get(),
        output_samples.get(),
    )
    .expect("writing into a String cannot fail");
    Ok(filter)
}

fn output_samples(
    frames: u64,
    frame_rate: FrameRate,
    output: &Path,
) -> Result<AudioSampleCount, EncodeError> {
    output_sample_rate()
        .samples_for(FrameCount::new(frames), frame_rate, Rounding::Ceil)
        .map_err(|source| {
            EncodeError::new(
                EncodeErrorKind::InputLimit,
                output,
                format!("audio output length exceeds its sample domain: {source}"),
            )
        })
}

fn output_sample_rate() -> AudioSampleRate {
    AudioSampleRate::new(OUTPUT_SAMPLE_RATE_HZ)
        .expect("the fixed output audio sample rate is positive")
}

fn model_frame_rate(frame_rate: WireFrameRate) -> FrameRate {
    FrameRate::new(frame_rate.numerator(), frame_rate.denominator())
        .expect("WireFrameRate retains one validated positive frame rate")
}

async fn wait_for_mix(
    child: &mut Child,
    completion_timeout: Duration,
    output: &Path,
) -> Result<std::process::ExitStatus, EncodeError> {
    match timeout(completion_timeout, child.wait()).await {
        Ok(Ok(status)) => Ok(status),
        Ok(Err(source)) => {
            stop_child(child).await;
            Err(EncodeError::io(
                EncodeErrorKind::ProcessControl,
                output,
                "failed to wait for FFmpeg audio mixing",
                source,
            ))
        }
        Err(_) => {
            stop_child(child).await;
            Err(EncodeError::new(
                EncodeErrorKind::Timeout,
                output,
                "FFmpeg audio mixing missed its completion deadline",
            ))
        }
    }
}

async fn finish_stderr(
    mut stderr: tokio::task::JoinHandle<std::io::Result<CapturedStderr>>,
    output: &Path,
) -> Result<CapturedStderr, EncodeError> {
    let Ok(joined) = timeout(CLEANUP_TIMEOUT, &mut stderr).await else {
        stderr.abort();
        let _ = stderr.await;
        return Err(EncodeError::new(
            EncodeErrorKind::StderrRead,
            output,
            "FFmpeg stderr reader missed its cleanup deadline",
        ));
    };
    let captured = joined.map_err(|source| EncodeError::join(output, source))?;
    captured.map_err(|source| {
        EncodeError::io(
            EncodeErrorKind::StderrRead,
            output,
            "failed to read FFmpeg stderr",
            source,
        )
    })
}

async fn stop_child(child: &mut Child) {
    let _ = child.start_kill();
    let _ = timeout(CLEANUP_TIMEOUT, child.wait()).await;
}

fn discard_partial_output(output: &Path) {
    let _ = std::fs::remove_file(output);
}

#[cfg(test)]
mod tests {
    use super::{AudioInput, audio_filter, output_samples};
    use onmark_core::model::{
        AudioChannelLayout, AudioGain, AudioSampleCount, FrameCount, FrameIndex, FrameRate,
    };

    #[test]
    fn mixes_tracks_in_authored_order_with_integer_sample_delays() {
        let rate = FrameRate::new(30, 1).expect("rate is valid");
        let output_samples = AudioSampleCount::new(96_000);
        let inputs = [
            AudioInput::new(
                0,
                "first.m4a".into(),
                FrameIndex::new(0),
                FrameCount::new(30),
                AudioSampleCount::new(48_000),
                AudioChannelLayout::Stereo,
                AudioGain::UNITY,
            ),
            AudioInput::new(
                1,
                "second.m4a".into(),
                FrameIndex::new(15),
                FrameCount::new(15),
                AudioSampleCount::new(24_000),
                AudioChannelLayout::Mono,
                AudioGain::new(1, 2).expect("one half is a valid gain"),
            ),
        ];

        assert_eq!(
            audio_filter(&inputs, rate, output_samples)
                .expect("the fixture placements fit the sample grid"),
            concat!(
                "[1:a]atrim=end_sample=48000,asetpts=N/SR/TB,",
                "aresample=48000,anull,",
                "aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,",
                "atrim=end_sample=48000,asetpts=N/SR/TB,",
                "adelay=delays=0S:all=1,volume=1/1[audio0];",
                "[2:a]atrim=end_sample=24000,asetpts=N/SR/TB,",
                "aresample=48000,pan=stereo|c0=c0|c1=c0,",
                "aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,",
                "atrim=end_sample=24000,asetpts=N/SR/TB,",
                "adelay=delays=24000S:all=1,volume=1/2[audio1];",
                "[audio0][audio1]",
                "amix=inputs=2:duration=longest:dropout_transition=0:normalize=0[mixed];",
                "[mixed]aresample=48000,atrim=end_sample=96000,",
                "apad=whole_len=96000[audio]",
            ),
        );
    }

    #[test]
    fn trims_again_on_the_output_grid_after_resampling() {
        let rate = FrameRate::new(30_000, 1_001).expect("rate is valid");
        let input = AudioInput::new(
            0,
            "voice-44k.m4a".into(),
            FrameIndex::ZERO,
            FrameCount::new(1),
            AudioSampleCount::new(1_472),
            AudioChannelLayout::Stereo,
            AudioGain::UNITY,
        );

        let filter = audio_filter(&[input], rate, AudioSampleCount::new(1_602))
            .expect("one frame fits both sample grids");

        assert!(filter.contains(
            "atrim=end_sample=1472,asetpts=N/SR/TB,aresample=48000,anull,\
             aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,\
             atrim=end_sample=1602,asetpts=N/SR/TB"
        ));
    }

    #[test]
    fn bounds_the_mix_to_the_visual_frame_count() {
        let rate = FrameRate::new(30_000, 1_001).expect("rate is valid");
        let output = std::path::Path::new("output.mp4");

        assert_eq!(
            output_samples(1, rate, output)
                .expect("one frame fits the output sample domain")
                .get(),
            1_602,
        );
    }
}
