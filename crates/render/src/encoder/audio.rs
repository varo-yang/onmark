//! Final audio mix over an already continuous visual encode.
//!
//! Timeline IR supplies exact frame starts. `FFmpeg` receives rational delay
//! expressions so this I/O boundary never rounds authored timing through `f64`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use onmark_core::model::FrameIndex;
use onmark_core::protocol::WireFrameRate;
use tokio::process::{Child, Command};
use tokio::runtime::Handle;
use tokio::time::timeout;

use super::error::{EncodeError, EncodeErrorKind};
use super::limits::EncodeLimits;
use super::process::{CapturedStderr, capture_stderr};
use super::session::{EncodedVideo, with_stderr};

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

/// One materialized voice-over input for an `FFmpeg` mix operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AudioInput {
    source: PathBuf,
    start: FrameIndex,
}

impl AudioInput {
    pub(crate) fn new(source: PathBuf, start: FrameIndex) -> Self {
        Self { source, start }
    }
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

    let runtime = Handle::try_current().map_err(|_| {
        EncodeError::new(
            EncodeErrorKind::Spawn,
            &output,
            "FFmpeg audio mixing requires a Tokio runtime",
        )
    })?;
    let mut child = spawn_audio_mix(executable, visual.path(), &inputs, frame_rate, &output)?;
    let Some(stderr) = child.stderr.take() else {
        discard_partial_output(&output);
        return Err(EncodeError::new(
            EncodeErrorKind::Spawn,
            &output,
            "FFmpeg started without its configured diagnostic pipe",
        ));
    };
    let stderr = runtime.spawn(capture_stderr(stderr, limits.max_stderr_bytes()));
    let status = match wait_for_mix(&mut child, limits.inactivity_timeout(), &output).await {
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
    frame_rate: WireFrameRate,
    output: &Path,
) -> Result<Child, EncodeError> {
    let filter = audio_filter(inputs, frame_rate);
    let mut command = Command::new(executable);
    command.args(["-nostdin", "-loglevel", "error", "-i"]);
    command.arg(visual);
    for input in inputs {
        command.args(["-i"]);
        command.arg(&input.source);
    }
    command
        .args([
            "-filter_complex",
            &filter,
            "-map",
            "0:v:0",
            "-map",
            "[audio]",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
            "-n",
        ])
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
        })
}

fn audio_filter(inputs: &[AudioInput], frame_rate: WireFrameRate) -> String {
    let mut filter = String::new();
    for (index, input) in inputs.iter().enumerate() {
        let stream = index + 1;
        let delay = frame_delay_expression(input.start, frame_rate);
        write!(
            filter,
            "[{stream}:a]asetpts=PTS-STARTPTS+{delay}/TB[audio{index}];"
        )
        .expect("writing into a String cannot fail");
    }
    for index in 0..inputs.len() {
        write!(filter, "[audio{index}]").expect("writing into a String cannot fail");
    }
    write!(
        filter,
        "amix=inputs={}:duration=longest:dropout_transition=0[audio]",
        inputs.len()
    )
    .expect("writing into a String cannot fail");
    filter
}

fn frame_delay_expression(start: FrameIndex, frame_rate: WireFrameRate) -> String {
    // The filter expression stays rational until FFmpeg maps it into the
    // selected audio stream time base. A decimal seconds projection would
    // discard the exact frame boundary owned by Timeline IR.
    format!(
        "({}*{})/{}",
        start.get(),
        frame_rate.denominator(),
        frame_rate.numerator()
    )
}

async fn wait_for_mix(
    child: &mut Child,
    deadline: Duration,
    output: &Path,
) -> Result<std::process::ExitStatus, EncodeError> {
    match timeout(deadline, child.wait()).await {
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
                "FFmpeg audio mixing made no progress before its inactivity timeout",
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
    use onmark_core::model::FrameRate;
    use onmark_core::protocol::WireFrameRate;

    use super::{AudioInput, audio_filter, frame_delay_expression};

    #[test]
    fn projects_frame_delays_as_rational_filter_expressions() {
        let rate = WireFrameRate::from(FrameRate::new(30_000, 1_001).expect("rate is valid"));

        assert_eq!(
            frame_delay_expression(onmark_core::model::FrameIndex::new(15), rate),
            "(15*1001)/30000"
        );
    }

    #[test]
    fn mixes_tracks_in_authored_order() {
        let rate = WireFrameRate::from(FrameRate::new(30, 1).expect("rate is valid"));
        let inputs = [
            AudioInput::new("first.m4a".into(), onmark_core::model::FrameIndex::new(0)),
            AudioInput::new("second.m4a".into(), onmark_core::model::FrameIndex::new(15)),
        ];

        assert_eq!(
            audio_filter(&inputs, rate),
            "[1:a]asetpts=PTS-STARTPTS+(0*1)/30/TB[audio0];[2:a]asetpts=PTS-STARTPTS+(15*1)/30/TB[audio1];[audio0][audio1]amix=inputs=2:duration=longest:dropout_transition=0[audio]"
        );
    }
}
