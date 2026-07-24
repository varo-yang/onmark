//! Checked command-line surface for local rendering and portable worker capture.

use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};
use onmark_core::model::FrameRate;
use onmark_render::{BrowserGraphicsBackend, EncodeLimits};

use crate::execution::LOCAL_VIDEO_ENCODER_THREADS;

/// Native entry point for deterministic screenplay rendering.
#[derive(Debug, Parser)]
#[command(name = "onmark", version, about)]
pub(super) struct Cli {
    #[command(subcommand)]
    pub(super) command: Command,
}

#[derive(Debug, Subcommand)]
pub(super) enum Command {
    /// Compile and render one screenplay into an H.264 MP4.
    Render(RenderArgs),
    /// Execute one already-planned worker task without recompiling source.
    Worker(WorkerArgs),
}

#[derive(Debug, Args)]
pub(super) struct WorkerArgs {
    #[command(subcommand)]
    pub(super) command: WorkerCommand,
}

#[derive(Debug, Subcommand)]
pub(super) enum WorkerCommand {
    /// Capture one portable worker request into a verified frame artifact.
    Capture(WorkerCaptureArgs),
}

#[derive(Debug, Args)]
pub(super) struct WorkerCaptureArgs {
    /// Directory containing request.json, bundle/, and frozen assets/sha256/ bytes.
    #[arg(long)]
    pub(super) input: PathBuf,

    /// Immutable frame-artifact destination.
    #[arg(long)]
    pub(super) output: PathBuf,

    /// Chrome for Testing headless-shell executable pinned by the worker environment.
    #[arg(long)]
    pub(super) browser: PathBuf,

    /// `FFmpeg` executable pinned by the worker environment.
    #[arg(long, default_value = "ffmpeg")]
    pub(super) ffmpeg: PathBuf,
}

#[derive(Debug, Args)]
pub(super) struct RenderArgs {
    /// Authored HTML document to compile and render.
    pub(super) screenplay: PathBuf,

    /// MP4 destination. Defaults to `renders/<screenplay>.mp4`.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Exact output frame rate, such as 30 or 30000/1001.
    #[arg(long = "fps", default_value = "30", value_parser = parse_frame_rate)]
    pub(super) frame_rate: FrameRate,

    /// Output width in CSS pixels.
    #[arg(long, default_value_t = 1_920)]
    pub(super) width: u32,

    /// Output height in CSS pixels.
    #[arg(long, default_value_t = 1_080)]
    pub(super) height: u32,

    /// Browser executable. Defaults to headless shell on Linux and Chrome elsewhere.
    #[arg(long, help_heading = "Execution overrides")]
    pub(super) browser: Option<PathBuf>,

    /// Browser graphics implementation. Omit to use the admitted host default.
    #[arg(long, value_enum, help_heading = "Execution overrides")]
    graphics: Option<GraphicsBackend>,

    /// Threads assigned to the final H.264 encoder.
    #[arg(
        long,
        default_value_t = LOCAL_VIDEO_ENCODER_THREADS,
        value_parser = parse_video_encoder_threads,
        help_heading = "Execution overrides"
    )]
    video_encoder_threads: usize,

    /// Presentation bundler executable.
    #[arg(
        long,
        env = "ONMARK_BUNDLER",
        hide_env = true,
        default_value = "onmark-bundle",
        help_heading = "Execution overrides"
    )]
    pub(super) bundler: PathBuf,

    /// `FFmpeg` executable.
    #[arg(
        long,
        env = "ONMARK_FFMPEG",
        hide_env = true,
        default_value = "ffmpeg",
        help_heading = "Execution overrides"
    )]
    pub(super) ffmpeg: PathBuf,

    /// ffprobe executable.
    #[arg(
        long,
        env = "ONMARK_FFPROBE",
        hide_env = true,
        default_value = "ffprobe",
        help_heading = "Execution overrides"
    )]
    pub(super) ffprobe: PathBuf,

    /// Standalone SRT, `WebVTT`, or ASS file.
    #[arg(long = "subtitle", value_name = "FILE")]
    pub(super) subtitle: Option<PathBuf>,
}

impl RenderArgs {
    pub(super) fn output(&self) -> PathBuf {
        self.output.clone().unwrap_or_else(|| {
            let stem = self
                .screenplay
                .file_stem()
                .unwrap_or_else(|| self.screenplay.as_os_str());
            Path::new("renders").join(stem).with_extension("mp4")
        })
    }

    pub(super) fn graphics_backend(&self) -> Option<BrowserGraphicsBackend> {
        self.graphics.map(GraphicsBackend::into_render_backend)
    }

    pub(super) const fn video_encoder_threads(&self) -> usize {
        self.video_encoder_threads
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum GraphicsBackend {
    Software,
    #[cfg(target_os = "macos")]
    Metal,
}

impl GraphicsBackend {
    const fn into_render_backend(self) -> BrowserGraphicsBackend {
        match self {
            Self::Software => BrowserGraphicsBackend::SwiftShader,
            #[cfg(target_os = "macos")]
            Self::Metal => BrowserGraphicsBackend::Metal,
        }
    }
}

pub(super) fn source_directory(source: &Path) -> &Path {
    source
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn parse_frame_rate(value: &str) -> Result<FrameRate, String> {
    let (numerator, denominator) = match value.split_once('/') {
        Some((numerator, denominator)) => (
            parse_rate_component(numerator)?,
            parse_rate_component(denominator)?,
        ),
        None => (parse_rate_component(value)?, 1),
    };
    FrameRate::new(numerator, denominator).map_err(|error| error.to_string())
}

fn parse_rate_component(value: &str) -> Result<u32, String> {
    value
        .parse()
        .map_err(|_| "frame rate must be a positive integer or exact rational".to_owned())
}

fn parse_video_encoder_threads(value: &str) -> Result<usize, String> {
    let message = || {
        format!(
            "video encoder threads must be an integer from 1 through {}",
            EncodeLimits::MAX_VIDEO_ENCODER_THREADS,
        )
    };
    let threads = value.parse().map_err(|_| message())?;
    if !(1..=EncodeLimits::MAX_VIDEO_ENCODER_THREADS).contains(&threads) {
        return Err(message());
    }
    Ok(threads)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use onmark_render::{BrowserGraphicsBackend, EncodeLimits};

    use super::{Cli, Command, LOCAL_VIDEO_ENCODER_THREADS, WorkerCommand};
    use clap::Parser;

    #[test]
    fn derives_stable_project_defaults_from_the_screenplay() {
        let cli = Cli::try_parse_from(["onmark", "render", "project/film.html"])
            .expect("the minimal command is valid");
        let Command::Render(args) = cli.command else {
            panic!("the fixture must parse as a render command");
        };

        assert_eq!(args.output(), Path::new("renders/film.mp4"));
        assert_eq!(args.frame_rate.numerator(), 30);
        assert_eq!(args.frame_rate.denominator(), 1);
        assert_eq!(args.graphics_backend(), None);
        assert_eq!(args.video_encoder_threads(), LOCAL_VIDEO_ENCODER_THREADS);
    }

    #[test]
    fn accepts_explicit_browser_graphics_overrides() {
        let cli = Cli::try_parse_from(["onmark", "render", "film.html", "--graphics", "software"])
            .expect("the software graphics override is valid on every host");
        let Command::Render(args) = cli.command else {
            panic!("the fixture must parse as a render command");
        };
        assert_eq!(
            args.graphics_backend(),
            Some(BrowserGraphicsBackend::SwiftShader),
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn accepts_the_admitted_metal_override() {
        let cli = Cli::try_parse_from(["onmark", "render", "film.html", "--graphics", "metal"])
            .expect("the Metal graphics override is admitted on macOS");
        let Command::Render(args) = cli.command else {
            panic!("the fixture must parse as a render command");
        };
        assert_eq!(args.graphics_backend(), Some(BrowserGraphicsBackend::Metal));
    }

    #[test]
    fn accepts_bounded_video_encoder_threads() {
        for threads in [1, EncodeLimits::MAX_VIDEO_ENCODER_THREADS] {
            let spelling = threads.to_string();
            let cli = Cli::try_parse_from([
                "onmark",
                "render",
                "film.html",
                "--video-encoder-threads",
                &spelling,
            ])
            .expect("the thread count stays inside the render safety envelope");
            let Command::Render(args) = cli.command else {
                panic!("the fixture must parse as a render command");
            };
            assert_eq!(args.video_encoder_threads(), threads);
        }
    }

    #[test]
    fn rejects_unbounded_video_encoder_threads() {
        for threads in [0, EncodeLimits::MAX_VIDEO_ENCODER_THREADS + 1] {
            let spelling = threads.to_string();
            let result = Cli::try_parse_from([
                "onmark",
                "render",
                "film.html",
                "--video-encoder-threads",
                &spelling,
            ]);
            assert!(result.is_err());
        }
    }

    #[test]
    fn accepts_exact_rational_rates_and_rejects_decimals() {
        let cli = Cli::try_parse_from(["onmark", "render", "film.html", "--fps", "30000/1001"])
            .expect("an exact rational rate is valid");
        let Command::Render(args) = cli.command else {
            panic!("the fixture must parse as a render command");
        };
        assert_eq!(args.frame_rate.numerator(), 30_000);
        assert_eq!(args.frame_rate.denominator(), 1_001);

        assert!(Cli::try_parse_from(["onmark", "render", "film.html", "--fps", "29.97",]).is_err());
    }

    #[test]
    fn rejects_unproven_presentation_capabilities() {
        assert!(
            Cli::try_parse_from([
                "onmark",
                "render",
                "film.html",
                "--temporal-capability",
                "randomAccess",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "onmark",
                "render",
                "film.html",
                "--visual-capability",
                "separableOverlay",
            ])
            .is_err()
        );
    }

    #[test]
    fn accepts_one_standalone_subtitle_file() {
        let cli = Cli::try_parse_from([
            "onmark",
            "render",
            "film.html",
            "--subtitle",
            "captions.srt",
        ])
        .expect("one subtitle input is valid");
        let Command::Render(args) = cli.command else {
            panic!("the fixture must parse as a render command");
        };

        assert_eq!(args.subtitle.as_deref(), Some(Path::new("captions.srt")));

        assert!(
            Cli::try_parse_from([
                "onmark",
                "render",
                "film.html",
                "--subtitle",
                "captions.srt",
                "--subtitle",
                "translation.vtt",
            ])
            .is_err(),
            "multiple caption tracks require explicit selection semantics",
        );
    }

    #[test]
    fn accepts_one_explicit_worker_capture_contract() {
        let cli = Cli::try_parse_from([
            "onmark",
            "worker",
            "capture",
            "--input",
            "work",
            "--output",
            "artifact.onmark-frames",
            "--browser",
            "chrome",
        ])
        .expect("a complete worker capture command is valid");
        let Command::Worker(worker) = cli.command else {
            panic!("the fixture must parse as a worker command");
        };
        let WorkerCommand::Capture(capture) = worker.command;

        assert_eq!(capture.input, Path::new("work"));
        assert_eq!(capture.output, Path::new("artifact.onmark-frames"));
        assert_eq!(capture.browser, Path::new("chrome"));
    }
}
