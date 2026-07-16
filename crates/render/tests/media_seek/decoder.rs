//! Bounded continuous decoding for the disposable media-strategy experiment.

use std::path::Path;
use std::process::Stdio;

use onmark_core::model::FrameRate;
use tokio::io::{AsyncRead, AsyncReadExt as _};
use tokio::process::Command;
use tokio::time::timeout;

use super::{BENCHMARK_FRAME_COUNT, PROCESS_DEADLINE, required_path};

const MAX_STDERR_BYTES: usize = 64 * 1024;

pub(super) async fn decode_sequence(
    media: &Path,
    frame_rate: FrameRate,
    dimensions: (u32, u32),
    retained_indices: &[u64],
) -> Vec<Vec<u8>> {
    read_sequence(
        decoder_command(media, frame_rate),
        dimensions,
        retained_indices,
    )
    .await
}

pub(super) async fn compose_sequence(
    media: &Path,
    overlay_pattern: &Path,
    frame_rate: FrameRate,
    dimensions: (u32, u32),
    retained_indices: &[u64],
) -> Vec<Vec<u8>> {
    read_sequence(
        compositor_command(media, overlay_pattern, frame_rate),
        dimensions,
        retained_indices,
    )
    .await
}

async fn read_sequence(
    mut command: Command,
    dimensions: (u32, u32),
    retained_indices: &[u64],
) -> Vec<Vec<u8>> {
    let frame_bytes = frame_bytes(dimensions);
    let mut child = command
        .spawn()
        .expect("FFmpeg must start the continuous frame stream");
    let mut stdout = child
        .stdout
        .take()
        .expect("the continuous frame stream must expose stdout");
    let stderr = child
        .stderr
        .take()
        .expect("the continuous frame stream must expose stderr");
    let stderr = tokio::spawn(read_bounded(stderr));

    let mut frame = vec![0; frame_bytes];
    let mut retained = Vec::with_capacity(retained_indices.len());
    for index in 0..BENCHMARK_FRAME_COUNT {
        stdout
            .read_exact(&mut frame)
            .await
            .expect("FFmpeg must emit every requested RGBA frame");
        if retained_indices.contains(&index) {
            retained.push(frame.clone());
        }
    }

    let mut trailing = [0];
    let trailing = stdout
        .read(&mut trailing)
        .await
        .expect("the continuous frame stream must remain readable");
    assert_eq!(trailing, 0, "FFmpeg emitted an unexpected partial frame");

    let status = timeout(PROCESS_DEADLINE, child.wait())
        .await
        .expect("the continuous frame stream must finish before its deadline")
        .expect("FFmpeg must report its frame-stream status");
    let stderr = stderr
        .await
        .expect("the frame-stream stderr reader must complete");
    assert!(
        status.success(),
        "continuous frame streaming failed: {}",
        String::from_utf8_lossy(&stderr),
    );
    assert_eq!(
        retained.len(),
        retained_indices.len(),
        "every requested native sample must be retained",
    );
    retained
}

fn decoder_command(media: &Path, frame_rate: FrameRate) -> Command {
    let frame_filter = rgba_frame_filter(frame_rate);
    let frame_count = BENCHMARK_FRAME_COUNT.to_string();
    let mut command = Command::new(required_path("ONMARK_FFMPEG"));
    command
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(media)
        .args([
            "-vf",
            &frame_filter,
            "-frames:v",
            &frame_count,
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

fn compositor_command(media: &Path, overlay_pattern: &Path, frame_rate: FrameRate) -> Command {
    let filter = composition_filter(frame_rate);
    let frame_rate = format!("{}/{}", frame_rate.numerator(), frame_rate.denominator(),);
    let frame_count = BENCHMARK_FRAME_COUNT.to_string();
    let mut command = Command::new(required_path("ONMARK_FFMPEG"));
    command
        .args([
            "-nostdin",
            "-v",
            "error",
            "-filter_complex_threads",
            "1",
            "-threads",
            "1",
            "-i",
        ])
        .arg(media)
        .args([
            "-framerate",
            &frame_rate,
            "-start_number",
            "0",
            "-threads",
            "1",
            "-i",
        ])
        .arg(overlay_pattern)
        .args([
            "-filter_complex",
            &filter,
            "-map",
            "[composited]",
            "-frames:v",
            &frame_count,
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

fn rgba_frame_filter(frame_rate: FrameRate) -> String {
    format!(
        "fps={}/{},format=rgba",
        frame_rate.numerator(),
        frame_rate.denominator(),
    )
}

fn composition_filter(frame_rate: FrameRate) -> String {
    let base = rgba_frame_filter(frame_rate);
    format!("[0:v]{base}[base];[base][1:v]overlay=shortest=1:format=rgb,format=rgba[composited]")
}

fn frame_bytes((width, height): (u32, u32)) -> usize {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .expect("the experiment dimensions must fit the pixel domain");
    let bytes = pixels
        .checked_mul(4)
        .expect("an RGBA frame must fit the byte domain");
    usize::try_from(bytes).expect("an RGBA frame must fit this process")
}

async fn read_bounded(mut input: impl AsyncRead + Unpin) -> Vec<u8> {
    let mut retained = Vec::new();
    let mut chunk = [0; 4 * 1024];
    loop {
        let count = input
            .read(&mut chunk)
            .await
            .expect("the frame-stream stderr must remain readable");
        if count == 0 {
            return retained;
        }
        assert!(
            retained.len() + count <= MAX_STDERR_BYTES,
            "frame-stream stderr exceeds its retained-byte ceiling",
        );
        retained.extend_from_slice(&chunk[..count]);
    }
}
