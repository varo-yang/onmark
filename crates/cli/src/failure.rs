//! Final translation from typed library failures to stable CLI exit behavior.
//!
//! Lower layers retain structured causes; only this process boundary chooses
//! terminal wording and exit status.

use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use onmark_core::compiler::{CaptionProjectionError, SolveError};
use onmark_core::render_graph::InvalidRenderGraph;
use onmark_render::{
    InvalidFfmpeg, InvalidRenderProfile, InvalidRenderUnit, RenderError, UnitRootError,
};
use serde::Serialize;
use tokio::task::JoinError;

use crate::arguments::InvalidOutputExtension;
use crate::artifact_cache::ArtifactCacheError;
use crate::assets::AssetError;
use crate::bundler::BundleError;
use crate::environment::EnvironmentError;
use crate::input::BoundedReadError;
use crate::subtitle::SubtitleLoadError;

#[derive(Debug)]
pub(super) enum CliError {
    Environment(EnvironmentError),
    ReadScreenplay {
        path: PathBuf,
        source: BoundedReadError,
    },
    ReadWorkerRequest {
        path: PathBuf,
        source: BoundedReadError,
    },
    ParseWorkerRequest {
        path: PathBuf,
        source: serde_json::Error,
    },
    WorkerTask(JoinError),
    WriteProgress(io::Error),
    BenchmarkWorkspace(io::Error),
    BenchmarkDrift(&'static str),
    Doctor(crate::doctor::DoctorError),
    CreateOutputDirectory {
        path: PathBuf,
        source: io::Error,
    },
    OutputExists(PathBuf),
    InvalidOutputExtension(InvalidOutputExtension),
    InvalidProfile(InvalidRenderProfile),
    InvalidFfmpeg(InvalidFfmpeg),
    ArtifactCache(ArtifactCacheError),
    Assets(AssetError),
    Solve(SolveError),
    Subtitle(SubtitleLoadError),
    CaptionProjection(CaptionProjectionError),
    Bundle(BundleError),
    RenderGraph(InvalidRenderGraph),
    RenderUnit(InvalidRenderUnit),
    UnitRoot(UnitRootError),
    Render(RenderError),
}

impl CliError {
    pub(super) fn read_screenplay(path: &Path, source: BoundedReadError) -> Self {
        Self::ReadScreenplay {
            path: path.to_owned(),
            source,
        }
    }

    pub(super) fn read_worker_request(path: &Path, source: BoundedReadError) -> Self {
        Self::ReadWorkerRequest {
            path: path.to_owned(),
            source,
        }
    }

    pub(super) fn parse_worker_request(path: &Path, source: serde_json::Error) -> Self {
        Self::ParseWorkerRequest {
            path: path.to_owned(),
            source,
        }
    }

    pub(super) fn create_output_directory(path: &Path, source: io::Error) -> Self {
        Self::CreateOutputDirectory {
            path: path.to_owned(),
            source,
        }
    }

    pub(super) fn benchmark_workspace(source: io::Error) -> Self {
        Self::BenchmarkWorkspace(source)
    }

    pub(super) const fn benchmark_drift(fact: &'static str) -> Self {
        Self::BenchmarkDrift(fact)
    }

    pub(super) fn write_progress(source: io::Error) -> Self {
        Self::WriteProgress(source)
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Environment(source) => source.fmt(formatter),
            Self::ReadScreenplay { path, .. } => {
                write!(formatter, "failed to read screenplay {}", path.display())
            }
            Self::ReadWorkerRequest { path, .. } => {
                write!(
                    formatter,
                    "failed to read worker request {}",
                    path.display()
                )
            }
            Self::ParseWorkerRequest { path, .. } => {
                write!(
                    formatter,
                    "failed to parse worker request {}",
                    path.display()
                )
            }
            Self::WorkerTask(_) => formatter.write_str("worker materialization did not finish"),
            Self::WriteProgress(_) => formatter.write_str("failed to write render progress"),
            Self::BenchmarkWorkspace(_) => {
                formatter.write_str("failed to create the private benchmark workspace")
            }
            Self::BenchmarkDrift(fact) => {
                write!(formatter, "benchmark samples disagree on {fact}")
            }
            Self::Doctor(source) => source.fmt(formatter),
            Self::CreateOutputDirectory { path, .. } => {
                write!(
                    formatter,
                    "failed to create output directory {}",
                    path.display()
                )
            }
            Self::OutputExists(path) => {
                write!(formatter, "output {} already exists", path.display())
            }
            Self::InvalidOutputExtension(source) => source.fmt(formatter),
            Self::InvalidProfile(source) => source.fmt(formatter),
            Self::InvalidFfmpeg(source) => source.fmt(formatter),
            Self::ArtifactCache(source) => source.fmt(formatter),
            Self::Assets(source) => source.fmt(formatter),
            Self::Solve(source) => source.fmt(formatter),
            Self::Subtitle(source) => source.fmt(formatter),
            Self::CaptionProjection(source) => source.fmt(formatter),
            Self::Bundle(source) => source.fmt(formatter),
            Self::RenderGraph(source) => source.fmt(formatter),
            Self::RenderUnit(source) => source.fmt(formatter),
            Self::UnitRoot(source) => source.fmt(formatter),
            Self::Render(source) => source.fmt(formatter),
        }
    }
}

impl Error for CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Environment(source) => Some(source),
            Self::ReadScreenplay { source, .. } | Self::ReadWorkerRequest { source, .. } => {
                Some(source)
            }
            Self::CreateOutputDirectory { source, .. }
            | Self::WriteProgress(source)
            | Self::BenchmarkWorkspace(source) => Some(source),
            Self::Doctor(source) => Some(source),
            Self::ParseWorkerRequest { source, .. } => Some(source),
            Self::WorkerTask(source) => Some(source),
            Self::OutputExists(_) | Self::BenchmarkDrift(_) => None,
            Self::InvalidOutputExtension(source) => Some(source),
            Self::InvalidProfile(source) => Some(source),
            Self::InvalidFfmpeg(source) => Some(source),
            Self::ArtifactCache(source) => Some(source),
            Self::Assets(source) => Some(source),
            Self::Solve(source) => Some(source),
            Self::Subtitle(source) => Some(source),
            Self::CaptionProjection(source) => Some(source),
            Self::Bundle(source) => Some(source),
            Self::RenderGraph(source) => Some(source),
            Self::RenderUnit(source) => Some(source),
            Self::UnitRoot(source) => Some(source),
            Self::Render(source) => Some(source),
        }
    }
}

impl From<EnvironmentError> for CliError {
    fn from(source: EnvironmentError) -> Self {
        Self::Environment(source)
    }
}

impl From<crate::doctor::DoctorError> for CliError {
    fn from(source: crate::doctor::DoctorError) -> Self {
        Self::Doctor(source)
    }
}

impl From<InvalidRenderProfile> for CliError {
    fn from(source: InvalidRenderProfile) -> Self {
        Self::InvalidProfile(source)
    }
}

impl From<InvalidOutputExtension> for CliError {
    fn from(source: InvalidOutputExtension) -> Self {
        Self::InvalidOutputExtension(source)
    }
}

impl From<InvalidFfmpeg> for CliError {
    fn from(source: InvalidFfmpeg) -> Self {
        Self::InvalidFfmpeg(source)
    }
}

impl From<ArtifactCacheError> for CliError {
    fn from(source: ArtifactCacheError) -> Self {
        Self::ArtifactCache(source)
    }
}

impl From<AssetError> for CliError {
    fn from(source: AssetError) -> Self {
        Self::Assets(source)
    }
}

impl From<SolveError> for CliError {
    fn from(source: SolveError) -> Self {
        Self::Solve(source)
    }
}

impl From<SubtitleLoadError> for CliError {
    fn from(source: SubtitleLoadError) -> Self {
        Self::Subtitle(source)
    }
}

impl From<CaptionProjectionError> for CliError {
    fn from(source: CaptionProjectionError) -> Self {
        Self::CaptionProjection(source)
    }
}

impl From<BundleError> for CliError {
    fn from(source: BundleError) -> Self {
        Self::Bundle(source)
    }
}

impl From<InvalidRenderGraph> for CliError {
    fn from(source: InvalidRenderGraph) -> Self {
        Self::RenderGraph(source)
    }
}

impl From<InvalidRenderUnit> for CliError {
    fn from(source: InvalidRenderUnit) -> Self {
        Self::RenderUnit(source)
    }
}

impl From<UnitRootError> for CliError {
    fn from(source: UnitRootError) -> Self {
        Self::UnitRoot(source)
    }
}

impl From<RenderError> for CliError {
    fn from(source: RenderError) -> Self {
        Self::Render(source)
    }
}

pub(super) fn write(writer: &mut impl Write, error: &CliError) -> io::Result<ExitCode> {
    let mut previous = error.to_string();
    writeln!(writer, "error: {previous}")?;
    let mut source = error.source();
    while let Some(cause) = source {
        let message = cause.to_string();
        // Composition wrappers retain typed sources, but transparent Display
        // implementations should not print the same sentence twice.
        if message != previous {
            writeln!(writer, "  caused by: {message}")?;
        }
        previous = message;
        source = cause.source();
    }
    Ok(ExitCode::from(2))
}

pub(super) fn write_json(writer: &mut impl Write, error: &CliError) -> io::Result<ExitCode> {
    let report = JsonFailure {
        version: 1,
        kind: "infrastructure",
        message: error.to_string(),
        causes: causes(error),
    };
    serde_json::to_writer_pretty(&mut *writer, &report)?;
    writeln!(writer)?;
    Ok(ExitCode::from(2))
}

fn causes(error: &CliError) -> Vec<String> {
    let mut causes = Vec::new();
    let mut previous = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let message = cause.to_string();
        if message != previous {
            causes.push(message.clone());
        }
        previous = message;
        source = cause.source();
    }
    causes
}

#[derive(Serialize)]
struct JsonFailure {
    version: u16,
    kind: &'static str,
    message: String,
    causes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::{AssetError, CliError, write};

    #[test]
    fn does_not_repeat_a_transparent_wrapper_message() {
        let error = CliError::Assets(AssetError::TooManyFiles);
        let mut output = Vec::new();

        write(&mut output, &error).expect("the failure is writable");

        assert_eq!(
            String::from_utf8(output).expect("failure output is UTF-8"),
            "error: screenplay exceeds the frozen-file limit\n",
        );
    }
}
