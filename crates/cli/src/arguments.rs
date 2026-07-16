//! Checked command-line surface for local rendering and portable worker capture.

use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use onmark_core::model::FrameRate;

const DEFAULT_PRESENTATION: &str = "presentation.ts";

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
}

#[derive(Debug, Args)]
pub(super) struct RenderArgs {
    /// Screenplay to compile.
    pub(super) screenplay: PathBuf,

    /// Browser presentation entry. Defaults to presentation.ts beside the screenplay.
    #[arg(short, long)]
    presentation: Option<PathBuf>,

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

    /// Chrome for Testing headless-shell executable.
    #[arg(long, help_heading = "Execution overrides")]
    pub(super) browser: Option<PathBuf>,

    /// Presentation bundler executable.
    #[arg(
        long,
        default_value = "onmark-bundle",
        help_heading = "Execution overrides"
    )]
    pub(super) bundler: PathBuf,

    /// `FFmpeg` executable.
    #[arg(long, default_value = "ffmpeg", help_heading = "Execution overrides")]
    pub(super) ffmpeg: PathBuf,

    /// ffprobe executable.
    #[arg(long, default_value = "ffprobe", help_heading = "Execution overrides")]
    pub(super) ffprobe: PathBuf,
}

impl RenderArgs {
    pub(super) fn presentation(&self) -> PathBuf {
        self.presentation
            .clone()
            .unwrap_or_else(|| source_directory(&self.screenplay).join(DEFAULT_PRESENTATION))
    }

    pub(super) fn output(&self) -> PathBuf {
        self.output.clone().unwrap_or_else(|| {
            let stem = self
                .screenplay
                .file_stem()
                .unwrap_or_else(|| self.screenplay.as_os_str());
            Path::new("renders").join(stem).with_extension("mp4")
        })
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::Parser;

    use super::{Cli, Command, WorkerCommand};

    #[test]
    fn derives_stable_project_defaults_from_the_screenplay() {
        let cli = Cli::try_parse_from(["onmark", "render", "project/film.onmark"])
            .expect("the minimal command is valid");
        let Command::Render(args) = cli.command else {
            panic!("the fixture must parse as a render command");
        };

        assert_eq!(args.presentation(), Path::new("project/presentation.ts"));
        assert_eq!(args.output(), Path::new("renders/film.mp4"));
        assert_eq!(args.frame_rate.numerator(), 30);
        assert_eq!(args.frame_rate.denominator(), 1);
    }

    #[test]
    fn accepts_exact_rational_rates_and_rejects_decimals() {
        let cli = Cli::try_parse_from(["onmark", "render", "film.onmark", "--fps", "30000/1001"])
            .expect("an exact rational rate is valid");
        let Command::Render(args) = cli.command else {
            panic!("the fixture must parse as a render command");
        };
        assert_eq!(args.frame_rate.numerator(), 30_000);
        assert_eq!(args.frame_rate.denominator(), 1_001);

        assert!(
            Cli::try_parse_from(["onmark", "render", "film.onmark", "--fps", "29.97",]).is_err()
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
