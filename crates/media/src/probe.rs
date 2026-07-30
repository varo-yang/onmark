//! Public ffprobe boundary over one local immutable artifact.
//!
//! Process control and JSON normalization stay behind this value so consumers
//! receive only stable metadata or typed probe failures.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use onmark_core::model::AssetMetadata;

use crate::error::{InvalidFfprobe, ProbeError, Stream};
use crate::process::{OutputReaders, ProcessOutput, RunningProbe};
use crate::response::{
    FrameProbeRequest, FrameTimingProbe, ProbedVideoTiming, parse_frame_timing, parse_metadata,
    parse_terminal_packet_duration,
};

const FRAME_ENTRIES: &str = "frame=best_effort_timestamp,pkt_duration:stream=time_base,duration_ts";
const PACKET_ENTRIES: &str = "packet=pts,duration";

/// Configured boundary for probing local artifacts with ffprobe.
///
/// The executable must be ffprobe itself, or a wrapper that replaces its own
/// process without leaving descendants that inherit the output pipes. This
/// boundary deliberately does not own arbitrary process-tree cleanup.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use std::time::Duration;
///
/// use onmark_media::Ffprobe;
///
/// let ffprobe = Ffprobe::new("ffprobe", Duration::from_secs(30), 64 * 1024)
///     .expect("the probe limits are safe");
/// let metadata = ffprobe
///     .probe(Path::new("opening.mp4"))
///     .expect("the local artifact is probeable");
///
/// assert!(metadata.duration().as_nanos() > 0);
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ffprobe {
    executable: PathBuf,
    timeout: Duration,
    output_limit: usize,
}

impl Ffprobe {
    /// Maximum lifetime allowed for one ffprobe process.
    pub const MAX_TIMEOUT: Duration = Duration::from_mins(10);

    /// Maximum bytes retained independently from stdout and stderr.
    ///
    /// The selected video stream may contribute up to the browser contract's
    /// complete 100,000-boundary timing map. Sixteen MiB retains that evidence
    /// without making either process pipe unbounded.
    pub const MAX_OUTPUT_BYTES: usize = 16 * 1_048_576;

    /// Creates an ffprobe boundary with explicit process limits.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidFfprobe`] when the executable is empty, a limit is
    /// zero, or a limit exceeds its fixed safety ceiling.
    pub fn new(
        executable: impl Into<PathBuf>,
        timeout: Duration,
        output_limit: usize,
    ) -> Result<Self, InvalidFfprobe> {
        let executable = executable.into();
        if executable.as_os_str().is_empty() {
            return Err(InvalidFfprobe::EmptyExecutable);
        }
        if timeout.is_zero() {
            return Err(InvalidFfprobe::ZeroTimeout);
        }
        if timeout > Self::MAX_TIMEOUT {
            return Err(InvalidFfprobe::TimeoutTooLong);
        }
        if output_limit == 0 {
            return Err(InvalidFfprobe::ZeroOutputLimit);
        }
        if output_limit > Self::MAX_OUTPUT_BYTES {
            return Err(InvalidFfprobe::OutputLimitTooLarge);
        }

        Ok(Self {
            executable,
            timeout,
            output_limit,
        })
    }

    /// Probes one local media artifact into core-owned normalized metadata.
    ///
    /// # Errors
    ///
    /// Returns [`ProbeError`] when the process cannot be controlled, exceeds
    /// its limits, exits unsuccessfully, or emits unusable metadata.
    pub fn probe(&self, path: &Path) -> Result<AssetMetadata, ProbeError> {
        let metadata = self.capture(self.spawn_metadata(path)?, path)?;
        let pending = parse_metadata(path, &metadata.stdout.bytes)?;
        let timing = pending
            .frame_probe_request()
            .map(|request| self.probe_video_timing(path, request))
            .transpose()?;

        pending.finish(path, timing)
    }

    fn probe_video_timing(
        &self,
        path: &Path,
        request: FrameProbeRequest,
    ) -> Result<ProbedVideoTiming, ProbeError> {
        let frames = self.capture(self.spawn_frames(path, request)?, path)?;
        match parse_frame_timing(path, &frames.stdout.bytes)? {
            FrameTimingProbe::Complete(timing) => Ok(timing),
            FrameTimingProbe::NeedsPacketDuration(pending) => {
                let packets = self.capture(self.spawn_packets(path, request)?, path)?;
                let duration = parse_terminal_packet_duration(
                    path,
                    &packets.stdout.bytes,
                    pending.last_timestamp(),
                )?;
                pending.finish(path, duration)
            }
        }
    }

    fn capture(&self, mut process: RunningProbe, path: &Path) -> Result<ProcessOutput, ProbeError> {
        let readers = OutputReaders::spawn(&mut process, self.output_limit)?;
        let outcome = process.wait(self.timeout);
        let output = readers.finish(path);
        let status = outcome?;
        let output = output?;

        if !status.success() {
            return Err(ProbeError::failed(
                path,
                status,
                &output.stderr.bytes,
                output.stderr.truncated,
            ));
        }
        if output.stdout.truncated {
            return Err(ProbeError::output_limit(
                path,
                Stream::Stdout,
                self.output_limit,
            ));
        }
        if output.stderr.truncated {
            return Err(ProbeError::output_limit(
                path,
                Stream::Stderr,
                self.output_limit,
            ));
        }

        Ok(output)
    }

    fn spawn_metadata(&self, path: &Path) -> Result<RunningProbe, ProbeError> {
        let child = Command::new(&self.executable)
            .args([
                OsStr::new("-v"),
                OsStr::new("error"),
                OsStr::new("-show_entries"),
                OsStr::new(
                    "format=duration:stream=index,codec_type,codec_name,pix_fmt,width,height,color_range,\
                     color_space,color_transfer,color_primaries,duration,sample_rate,channels:\
                     stream_disposition=default,attached_pic",
                ),
                OsStr::new("-of"),
                OsStr::new("json"),
                OsStr::new("--"),
            ])
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| ProbeError::spawn(path, source))?;

        Ok(RunningProbe::new(child, path.to_owned()))
    }

    fn spawn_frames(
        &self,
        path: &Path,
        request: FrameProbeRequest,
    ) -> Result<RunningProbe, ProbeError> {
        self.spawn_selected(path, request, "-show_frames", FRAME_ENTRIES)
    }

    fn spawn_packets(
        &self,
        path: &Path,
        request: FrameProbeRequest,
    ) -> Result<RunningProbe, ProbeError> {
        self.spawn_selected(path, request, "-show_packets", PACKET_ENTRIES)
    }

    fn spawn_selected(
        &self,
        path: &Path,
        request: FrameProbeRequest,
        section: &str,
        entries: &str,
    ) -> Result<RunningProbe, ProbeError> {
        let stream = request.stream_index().to_string();
        let child = Command::new(&self.executable)
            .args([
                OsStr::new("-v"),
                OsStr::new("error"),
                OsStr::new("-select_streams"),
            ])
            .arg(stream)
            .args([
                OsStr::new(section),
                OsStr::new("-show_entries"),
                OsStr::new(entries),
                OsStr::new("-of"),
                OsStr::new("json"),
                OsStr::new("--"),
            ])
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| ProbeError::spawn(path, source))?;

        Ok(RunningProbe::new(child, path.to_owned()))
    }
}
