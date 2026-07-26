//! `FFmpeg` process construction and bounded diagnostic capture.
//!
//! Arguments are passed without a shell, and stderr retention preserves the
//! final failure cause without allowing an encoder to grow memory unboundedly.

use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;

use onmark_core::protocol::WireFrameRate;
use tokio::io::{AsyncRead, AsyncReadExt as _};
use tokio::process::{Child, Command};

use super::error::{EncodeError, EncodeErrorKind};

pub(super) fn spawn_ffmpeg(
    executable: &Path,
    output: &Path,
    frame_rate: WireFrameRate,
    video_encoder_threads: usize,
) -> Result<Child, EncodeError> {
    let frame_rate = format!("{}/{}", frame_rate.numerator(), frame_rate.denominator());
    let mut command = Command::new(executable);
    command
        .args([
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "image2pipe",
            "-framerate",
        ])
        .arg(frame_rate)
        .args(["-vcodec", "png", "-i", "pipe:0"]);
    configure_h264_output(&mut command, output, video_encoder_threads);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| {
            EncodeError::io(
                EncodeErrorKind::Spawn,
                output,
                "failed to start FFmpeg",
                source,
            )
        })
}

pub(super) fn configure_h264_output(
    command: &mut Command,
    output: &Path,
    video_encoder_threads: usize,
) {
    let video_encoder_threads = video_encoder_threads.to_string();
    // Encoder threads retain full-resolution reference frames. Keep the exact
    // bounded policy independent of ambient CPU count.
    command
        .args([
            "-an", "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-threads",
        ])
        .arg(video_encoder_threads)
        .args([
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

#[derive(Debug)]
pub(super) struct CapturedStderr {
    pub(super) bytes: Vec<u8>,
    pub(super) truncated: bool,
}

pub(super) async fn capture_stderr(
    mut stderr: impl AsyncRead + Unpin,
    limit: usize,
) -> std::io::Result<CapturedStderr> {
    let mut retained = VecDeque::with_capacity(limit.min(8_192));
    let mut buffer = [0_u8; 8_192];
    let mut truncated = false;

    loop {
        let count = stderr.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        truncated |= retain_tail(&mut retained, &buffer[..count], limit);
    }

    Ok(CapturedStderr {
        bytes: retained.into(),
        truncated,
    })
}

fn retain_tail(retained: &mut VecDeque<u8>, chunk: &[u8], limit: usize) -> bool {
    let truncated = retained.len().saturating_add(chunk.len()) > limit;
    let overflow = retained
        .len()
        .saturating_add(chunk.len())
        .saturating_sub(limit);
    retained.drain(..overflow.min(retained.len()));

    if chunk.len() >= limit {
        retained.clear();
        retained.extend(&chunk[chunk.len() - limit..]);
    } else {
        retained.extend(chunk);
    }
    truncated
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::ffi::OsString;
    use std::path::Path;

    use tokio::process::Command;

    use super::{configure_h264_output, retain_tail};

    #[test]
    fn owns_the_standard_h264_quality_policy() {
        let mut command = Command::new("ffmpeg");

        configure_h264_output(&mut command, Path::new("film.mp4"), 4);

        let arguments = command
            .as_std()
            .get_args()
            .map(OsString::from)
            .collect::<Vec<_>>();
        assert!(has_argument_pair(&arguments, "-preset", "medium"));
        assert!(has_argument_pair(&arguments, "-crf", "18"));
        assert!(has_argument_pair(&arguments, "-colorspace", "bt709"));
        assert!(has_argument_pair(&arguments, "-color_range", "tv"));
    }

    #[test]
    fn retains_only_the_bounded_stderr_tail() {
        let mut retained = VecDeque::new();

        assert!(!retain_tail(&mut retained, b"first", 8));
        assert!(retain_tail(&mut retained, b"-second", 8));
        assert_eq!(Vec::from(retained), b"t-second");
    }

    fn has_argument_pair(arguments: &[OsString], name: &str, value: &str) -> bool {
        arguments
            .windows(2)
            .any(|pair| pair[0] == name && pair[1] == value)
    }
}
