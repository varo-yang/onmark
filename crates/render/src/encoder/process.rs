use std::collections::VecDeque;
use std::ffi::OsStr;
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
) -> Result<Child, EncodeError> {
    let frame_rate = format!("{}/{}", frame_rate.numerator(), frame_rate.denominator());
    Command::new(executable)
        .args([
            OsStr::new("-nostdin"),
            OsStr::new("-loglevel"),
            OsStr::new("error"),
            OsStr::new("-f"),
            OsStr::new("image2pipe"),
            OsStr::new("-framerate"),
            OsStr::new(&frame_rate),
            OsStr::new("-vcodec"),
            OsStr::new("png"),
            OsStr::new("-i"),
            OsStr::new("pipe:0"),
            OsStr::new("-an"),
            OsStr::new("-c:v"),
            OsStr::new("libx264"),
            OsStr::new("-pix_fmt"),
            OsStr::new("yuv420p"),
            OsStr::new("-movflags"),
            OsStr::new("+faststart"),
            OsStr::new("-f"),
            OsStr::new("mp4"),
            OsStr::new("-n"),
        ])
        .arg(output)
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
    use super::retain_tail;
    use std::collections::VecDeque;

    #[test]
    fn retains_only_the_bounded_stderr_tail() {
        let mut retained = VecDeque::new();

        assert!(!retain_tail(&mut retained, b"first", 8));
        assert!(retain_tail(&mut retained, b"-second", 8));
        assert_eq!(Vec::from(retained), b"t-second");
    }
}
