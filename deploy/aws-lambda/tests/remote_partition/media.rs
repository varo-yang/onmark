//! Deterministic media generation and decoded-output assertions.

use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use serde::Deserialize;
use tokio::process::Command;
use tokio::time::timeout;

use super::aws::RemoteEnvironment;

const PROCESS_TIMEOUT: Duration = Duration::from_mins(3);
const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;
const EXPECTED_FRAMES: usize = 60;
const AUDIO_SAMPLE_RATE: u64 = 48_000;
const AUDIBLE_SAMPLE_THRESHOLD: u16 = 256;
const EXPECTED_AUDIO_START_MICROS: u64 = 1_000_000;
const AAC_PACKET_TOLERANCE_MICROS: u64 = 25_000;

pub(super) struct GeneratedMedia {
    video: PathBuf,
    voice_over: PathBuf,
}

impl GeneratedMedia {
    pub(super) fn video(&self) -> &Path {
        &self.video
    }

    pub(super) fn voice_over(&self) -> &Path {
        &self.voice_over
    }
}

pub(super) async fn generate(workspace: &Path, environment: &RemoteEnvironment) -> GeneratedMedia {
    let video = workspace.join("source.mp4");
    let source = format!("testsrc2=size={WIDTH}x{HEIGHT}:rate=30:duration=1");
    run(
        Command::new(environment.ffmpeg())
            .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i", &source])
            .args([
                "-an",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-g",
                "30",
                "-bf",
                "3",
                "-movflags",
                "+faststart",
                "-y",
            ])
            .arg(&video),
        "generate source video",
    )
    .await;

    let voice_over = workspace.join("voice.m4a");
    run(
        Command::new(environment.ffmpeg())
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=1",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-y",
            ])
            .arg(&voice_over),
        "generate voice-over",
    )
    .await;

    GeneratedMedia { video, voice_over }
}

pub(super) async fn verify_output(path: &Path, environment: &RemoteEnvironment) {
    let video = run(
        Command::new(environment.ffprobe())
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-count_frames",
                "-show_entries",
                "stream=codec_name,width,height,avg_frame_rate,nb_read_frames",
                "-of",
                "json",
                "--",
            ])
            .arg(path),
        "probe assembled video",
    )
    .await;
    let response: VideoProbe =
        serde_json::from_slice(&video.stdout).expect("ffprobe video output is JSON");
    let [stream]: [VideoStream; 1] = response
        .streams
        .try_into()
        .expect("the output has exactly one video stream");
    assert_eq!(stream.codec_name.as_ref(), "h264");
    assert_eq!(stream.width, WIDTH);
    assert_eq!(stream.height, HEIGHT);
    assert_eq!(stream.avg_frame_rate.as_ref(), "30/1");
    assert_eq!(stream.nb_read_frames.as_ref(), EXPECTED_FRAMES.to_string());

    let hashes = decoded_video_hashes(path, environment).await;
    assert_eq!(hashes.len(), EXPECTED_FRAMES);
    assert!(
        hashes.iter().skip(1).any(|hash| hash != &hashes[0]),
        "the assembled remote output must retain decoded motion",
    );
    verify_audio(path, environment).await;
}

async fn decoded_video_hashes(path: &Path, environment: &RemoteEnvironment) -> Vec<Box<str>> {
    let output = run(
        Command::new(environment.ffmpeg())
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(path)
            .args(["-map", "0:v:0", "-f", "framemd5", "-"]),
        "decode assembled video",
    )
    .await;
    String::from_utf8(output.stdout)
        .expect("framemd5 output is UTF-8")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|record| {
            record
                .rsplit_once(',')
                .expect("every framemd5 record contains a hash")
                .1
                .trim()
                .into()
        })
        .collect()
}

async fn verify_audio(path: &Path, environment: &RemoteEnvironment) {
    let output = run(
        Command::new(environment.ffprobe())
            .args(["-v", "error", "-select_streams", "a:0"])
            .arg(path)
            .args([
                "-show_entries",
                "stream=codec_name,sample_rate,channels",
                "-of",
                "json",
            ]),
        "probe assembled audio",
    )
    .await;
    let response: AudioProbe =
        serde_json::from_slice(&output.stdout).expect("ffprobe audio output is JSON");
    let [stream]: [AudioStream; 1] = response
        .streams
        .try_into()
        .expect("the output has exactly one audio stream");
    assert_eq!(stream.codec_name.as_ref(), "aac");
    assert_eq!(stream.sample_rate.as_ref(), "48000");
    assert_eq!(stream.channels, 2);

    let decoded = run(
        Command::new(environment.ffmpeg())
            .args(["-nostdin", "-v", "error", "-i"])
            .arg(path)
            .args([
                "-map",
                "0:a:0",
                "-f",
                "s16le",
                "-acodec",
                "pcm_s16le",
                "-ar",
            ])
            .arg(AUDIO_SAMPLE_RATE.to_string())
            .args(["-ac", "1", "-"]),
        "decode assembled audio",
    )
    .await;
    let sample = first_audible_sample(&decoded.stdout)
        .expect("the assembled output contains audible PCM samples");
    let actual = u64::try_from(sample).expect("the bounded fixture length fits in u64") * 1_000_000
        / AUDIO_SAMPLE_RATE;
    assert!(
        actual.abs_diff(EXPECTED_AUDIO_START_MICROS) <= AAC_PACKET_TOLERANCE_MICROS,
        "audio starts at {actual}µs instead of {EXPECTED_AUDIO_START_MICROS}µs",
    );
}

async fn run(command: &mut Command, operation: &str) -> Output {
    command.kill_on_drop(true);
    let output = timeout(PROCESS_TIMEOUT, command.output())
        .await
        .unwrap_or_else(|_| panic!("{operation} exceeded its conformance deadline"))
        .unwrap_or_else(|error| panic!("failed to start {operation}: {error}"));
    assert!(
        output.status.success(),
        "{operation} failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

fn first_audible_sample(pcm: &[u8]) -> Option<usize> {
    let mut samples = pcm.chunks_exact(2);
    let audible = samples.position(|sample| {
        i16::from_le_bytes([sample[0], sample[1]]).unsigned_abs() >= AUDIBLE_SAMPLE_THRESHOLD
    });
    assert!(
        samples.remainder().is_empty(),
        "decoded PCM is frame-aligned"
    );
    audible
}

#[derive(Deserialize)]
struct VideoProbe {
    streams: Vec<VideoStream>,
}

#[derive(Debug, Deserialize)]
struct VideoStream {
    codec_name: Box<str>,
    width: u32,
    height: u32,
    avg_frame_rate: Box<str>,
    nb_read_frames: Box<str>,
}

#[derive(Deserialize)]
struct AudioProbe {
    streams: Vec<AudioStream>,
}

#[derive(Debug, Deserialize)]
struct AudioStream {
    codec_name: Box<str>,
    sample_rate: Box<str>,
    channels: u32,
}

#[cfg(test)]
mod tests {
    use super::first_audible_sample;

    #[test]
    fn finds_the_first_audible_pcm_sample() {
        let pcm = [0, 0, 255, 0, 0, 1];
        assert_eq!(first_audible_sample(&pcm), Some(2));
    }
}
