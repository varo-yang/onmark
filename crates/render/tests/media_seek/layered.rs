//! Bounded native composition for the opt-in layered-media experiment.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use onmark_core::model::{FrameRate, VideoColorProfile};
use onmark_render::EncodedPng;
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};
use tokio::process::{Child, ChildStdin, Command};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::{PROCESS_DEADLINE, required_path};

const MAX_STDERR_BYTES: usize = 64 * 1024;

pub(super) struct LayeredOutput {
    pub(super) fingerprints: Vec<[u8; 32]>,
    samples: Vec<PixelSample>,
}

impl LayeredOutput {
    pub(super) fn sample(&self, probe: PixelProbe) -> [u8; 4] {
        self.samples
            .iter()
            .find(|sample| sample.probe == probe)
            .map(|sample| sample.rgba)
            .expect("the layered output must contain every requested pixel sample")
    }
}

#[derive(Clone, Copy)]
pub(super) struct LayeredSegment {
    start: u64,
    frames: u64,
}

impl LayeredSegment {
    pub(super) fn new(start: u64, frames: u64) -> Self {
        assert!(frames > 0, "a layered segment must contain a frame");
        start
            .checked_add(frames)
            .expect("a layered segment must fit the frame domain");
        Self { start, frames }
    }

    fn end(self) -> u64 {
        self.start
            .checked_add(self.frames)
            .expect("the layered segment constructor checked its end")
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct PixelProbe {
    local_frame: u64,
    x: u32,
    y: u32,
}

impl PixelProbe {
    pub(super) const fn new(local_frame: u64, x: u32, y: u32) -> Self {
        Self { local_frame, x, y }
    }
}

struct PixelSample {
    probe: PixelProbe,
    rgba: [u8; 4],
}

pub(super) struct LayeredJob<'a> {
    media: &'a Path,
    output: PathBuf,
    source_frame_rate: FrameRate,
    output_frame_rate: FrameRate,
    color_profile: VideoColorProfile,
    dimensions: (u32, u32),
    segment: LayeredSegment,
    probes: Vec<PixelProbe>,
}

impl<'a> LayeredJob<'a> {
    pub(super) fn new(
        media: &'a Path,
        output: PathBuf,
        source_frame_rate: FrameRate,
        output_frame_rate: FrameRate,
        color_profile: VideoColorProfile,
        dimensions: (u32, u32),
        segment: LayeredSegment,
    ) -> Self {
        Self {
            media,
            output,
            source_frame_rate,
            output_frame_rate,
            color_profile,
            dimensions,
            segment,
            probes: Vec::new(),
        }
    }

    pub(super) fn with_probes(mut self, probes: &[PixelProbe]) -> Self {
        for &probe in probes {
            assert!(
                probe.local_frame < self.segment.frames,
                "a pixel probe must lie inside its layered segment",
            );
            assert!(
                probe.x < self.dimensions.0 && probe.y < self.dimensions.1,
                "a pixel probe must lie inside the output frame",
            );
            assert!(
                !self.probes.contains(&probe),
                "a layered pixel probe cannot be repeated",
            );
            self.probes.push(probe);
        }
        self
    }
}

pub(super) struct LayeredCompositor {
    child: Child,
    input: Option<ChildStdin>,
    frames: JoinHandle<FrameObservation>,
    stderr: JoinHandle<CapturedStderr>,
    output: PathBuf,
    segment: LayeredSegment,
    submitted: u64,
}

impl LayeredCompositor {
    pub(super) fn start(job: LayeredJob<'_>) -> Self {
        let mut child = compositor_command(&job)
            .spawn()
            .expect("FFmpeg must start the layered composition stream");
        let input = child
            .stdin
            .take()
            .expect("the layered composition stream must expose stdin");
        let stdout = child
            .stdout
            .take()
            .expect("the layered composition stream must expose stdout");
        let stderr = child
            .stderr
            .take()
            .expect("the layered composition stream must expose stderr");

        Self {
            child,
            input: Some(input),
            frames: tokio::spawn(read_frames(
                stdout,
                job.dimensions,
                job.segment.frames,
                job.probes,
            )),
            stderr: tokio::spawn(read_bounded(stderr)),
            output: job.output,
            segment: job.segment,
            submitted: 0,
        }
    }

    pub(super) async fn write_overlay(&mut self, frame: &EncodedPng) {
        let input = self
            .input
            .as_mut()
            .expect("the layered composition input must remain open");
        timeout(PROCESS_DEADLINE, input.write_all(frame.as_bytes()))
            .await
            .expect("the layered input must accept a frame before its deadline")
            .expect("FFmpeg must accept the transparent presentation frame");
        self.submitted = self
            .submitted
            .checked_add(1)
            .expect("the submitted-frame count must fit its domain");
    }

    pub(super) async fn finish(mut self) -> LayeredOutput {
        assert_eq!(
            self.submitted, self.segment.frames,
            "the compositor must receive every segment frame",
        );
        self.input.take();

        let status = timeout(PROCESS_DEADLINE, self.child.wait())
            .await
            .expect("the layered compositor must finish before its deadline")
            .expect("FFmpeg must report its layered composition status");
        let stderr = self
            .stderr
            .await
            .expect("the layered diagnostic reader must complete");
        assert_process_succeeded(status, &stderr);
        let observation = self
            .frames
            .await
            .expect("the layered raw-frame reader must complete");

        assert_encoded_output(&self.output);
        LayeredOutput {
            fingerprints: observation.fingerprints,
            samples: observation.samples,
        }
    }
}

fn assert_process_succeeded(status: std::process::ExitStatus, stderr: &CapturedStderr) {
    let suffix = if stderr.truncated { " [truncated]" } else { "" };
    assert!(
        status.success(),
        "layered composition failed: {}{suffix}",
        String::from_utf8_lossy(&stderr.bytes),
    );
}

fn assert_encoded_output(output: &Path) {
    let bytes = std::fs::metadata(output)
        .expect("the layered compositor must publish its encoded output")
        .len();
    assert!(bytes > 0, "the layered encoded output cannot be empty");
}

async fn read_frames(
    mut output: impl AsyncRead + Unpin,
    dimensions: (u32, u32),
    frame_count: u64,
    probes: Vec<PixelProbe>,
) -> FrameObservation {
    let mut pixels = vec![0; frame_bytes(dimensions)];
    let capacity = usize::try_from(frame_count).expect("the segment must fit this process");
    let mut fingerprints = Vec::with_capacity(capacity);
    let mut samples = Vec::with_capacity(probes.len());
    for frame in 0..frame_count {
        output
            .read_exact(&mut pixels)
            .await
            .expect("FFmpeg must emit every layered RGBA frame");
        fingerprints.push(Sha256::digest(&pixels).into());
        sample_frame(&pixels, dimensions, frame, &probes, &mut samples);
    }

    let mut trailing = [0];
    let trailing = output
        .read(&mut trailing)
        .await
        .expect("the layered frame stream must remain readable");
    assert_eq!(trailing, 0, "FFmpeg emitted an unexpected partial frame");
    assert_eq!(
        samples.len(),
        probes.len(),
        "the layered stream must observe every requested pixel",
    );
    FrameObservation {
        fingerprints,
        samples,
    }
}

struct FrameObservation {
    fingerprints: Vec<[u8; 32]>,
    samples: Vec<PixelSample>,
}

fn sample_frame(
    pixels: &[u8],
    dimensions: (u32, u32),
    frame: u64,
    probes: &[PixelProbe],
    samples: &mut Vec<PixelSample>,
) {
    for &probe in probes.iter().filter(|probe| probe.local_frame == frame) {
        let offset = pixel_offset(dimensions.0, probe.x, probe.y);
        let rgba = pixels[offset..offset + 4]
            .try_into()
            .expect("one sampled RGBA pixel must contain four channels");
        samples.push(PixelSample { probe, rgba });
    }
}

fn pixel_offset(width: u32, x: u32, y: u32) -> usize {
    let pixel = u64::from(y)
        .checked_mul(u64::from(width))
        .and_then(|row| row.checked_add(u64::from(x)))
        .expect("a sampled pixel must fit the image domain");
    let byte = pixel
        .checked_mul(4)
        .expect("a sampled RGBA pixel must fit the byte domain");
    usize::try_from(byte).expect("a sampled RGBA pixel must fit this process")
}

fn compositor_command(job: &LayeredJob<'_>) -> Command {
    let rate = format!(
        "{}/{}",
        job.output_frame_rate.numerator(),
        job.output_frame_rate.denominator(),
    );
    let frames = job.segment.frames.to_string();
    let filter = composition_filter(
        job.source_frame_rate,
        job.output_frame_rate,
        job.color_profile,
        job.segment,
    );
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
        .arg(job.media);
    configure_overlay_input(&mut command, &rate, &filter);
    configure_canonical_output(&mut command, &frames);
    configure_encoded_output(&mut command, &job.output, &frames, job.color_profile);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

fn configure_overlay_input(command: &mut Command, rate: &str, filter: &str) {
    command.args([
        "-f",
        "image2pipe",
        "-framerate",
        rate,
        "-vcodec",
        "png",
        "-i",
        "pipe:0",
        "-filter_complex",
        filter,
    ]);
}

fn configure_canonical_output(command: &mut Command, frames: &str) {
    // This branch is drained concurrently so fingerprinting cannot back up the
    // overlay pipe or be omitted from the measured interval.
    command.args([
        "-map",
        "[canonical]",
        "-frames:v",
        frames,
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "pipe:1",
    ]);
}

fn configure_encoded_output(
    command: &mut Command,
    output: &Path,
    frames: &str,
    color_profile: VideoColorProfile,
) {
    command.args([
        "-map",
        "[encoded]",
        "-frames:v",
        frames,
        "-an",
        "-c:v",
        "libx264",
        // Match the production encoder's bounded reference-frame queue.
        "-threads",
        "1",
        "-pix_fmt",
        "yuv420p",
        "-movflags",
        "+faststart",
    ]);
    match color_profile {
        VideoColorProfile::Bt709Limited => {
            command.args([
                "-colorspace",
                "bt709",
                "-color_primaries",
                "bt709",
                "-color_trc",
                "bt709",
                "-color_range",
                "tv",
            ]);
        }
    }
    command.args(["-f", "mp4", "-n"]).arg(output);
}

fn composition_filter(
    source_frame_rate: FrameRate,
    output_frame_rate: FrameRate,
    color_profile: VideoColorProfile,
    segment: LayeredSegment,
) -> String {
    let selection = source_selection_filter(source_frame_rate, output_frame_rate);
    match color_profile {
        VideoColorProfile::Bt709Limited => format!(
            concat!(
                "[0:v]{selection},",
                "trim=start_frame={start}:end_frame={end},",
                "setpts=PTS-STARTPTS,",
                "scale=in_range=limited:in_color_matrix=bt709:",
                "out_range=full:out_color_matrix=bt709,",
                "format=rgba[base];",
                "[base][1:v]overlay=shortest=1:format=rgb,",
                "format=rgba,split=2[canonical][encoded]",
            ),
            selection = selection,
            start = segment.start,
            end = segment.end(),
        ),
    }
}

fn source_selection_filter(source: FrameRate, output: FrameRate) -> String {
    // Source frame N becomes visible at the first output-frame midpoint at or
    // after N/source. Projecting that boundary explicitly prevents FFmpeg's
    // default PTS rounding from becoming a second frame-selection policy.
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

fn frame_bytes((width, height): (u32, u32)) -> usize {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .expect("the experiment dimensions must fit the pixel domain");
    let bytes = pixels
        .checked_mul(4)
        .expect("an RGBA frame must fit the byte domain");
    usize::try_from(bytes).expect("an RGBA frame must fit this process")
}

struct CapturedStderr {
    bytes: Vec<u8>,
    truncated: bool,
}

async fn read_bounded(mut input: impl AsyncRead + Unpin) -> CapturedStderr {
    let mut retained = VecDeque::new();
    let mut chunk = [0; 4 * 1024];
    let mut truncated = false;
    loop {
        let count = input
            .read(&mut chunk)
            .await
            .expect("the layered diagnostic stream must remain readable");
        if count == 0 {
            return CapturedStderr {
                bytes: retained.into(),
                truncated,
            };
        }
        truncated |= retain_tail(&mut retained, &chunk[..count]);
    }
}

fn retain_tail(retained: &mut VecDeque<u8>, chunk: &[u8]) -> bool {
    let overflow = retained
        .len()
        .saturating_add(chunk.len())
        .saturating_sub(MAX_STDERR_BYTES);
    retained.drain(..overflow.min(retained.len()));
    if chunk.len() >= MAX_STDERR_BYTES {
        retained.clear();
        retained.extend(&chunk[chunk.len() - MAX_STDERR_BYTES..]);
    } else {
        retained.extend(chunk);
    }
    overflow > 0
}

#[test]
fn retains_only_the_layered_diagnostic_tail() {
    let mut retained = VecDeque::from(vec![b'a'; MAX_STDERR_BYTES - 2]);

    assert!(retain_tail(&mut retained, b"tail"));
    assert_eq!(retained.len(), MAX_STDERR_BYTES);
    assert_eq!(
        retained
            .range(MAX_STDERR_BYTES - 4..)
            .copied()
            .collect::<Vec<_>>(),
        b"tail"
    );
}
