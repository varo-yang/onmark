use std::error::Error;
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use onmark_core::compiler::SolveError;
use onmark_render::{InvalidRenderProfile, InvalidRenderUnit, RenderError, UnitRootError};
use tokio::task::JoinError;

use crate::assets::AssetError;
use crate::bundler::BundleError;
use crate::environment::EnvironmentError;

#[derive(Debug)]
pub(super) enum CliError {
    Environment(EnvironmentError),
    ReadScreenplay {
        path: PathBuf,
        source: io::Error,
    },
    ReadWorkerRequest {
        path: PathBuf,
        source: io::Error,
    },
    ParseWorkerRequest {
        path: PathBuf,
        source: serde_json::Error,
    },
    WorkerTask(JoinError),
    InspectPresentation {
        path: PathBuf,
        source: io::Error,
    },
    InvalidPresentation(PathBuf),
    CreateOutputDirectory {
        path: PathBuf,
        source: io::Error,
    },
    OutputExists(PathBuf),
    InvalidProfile(InvalidRenderProfile),
    Assets(AssetError),
    Solve(SolveError),
    Bundle(BundleError),
    RenderUnit(InvalidRenderUnit),
    UnitRoot(UnitRootError),
    Render(RenderError),
}

impl CliError {
    pub(super) fn read_screenplay(path: &Path, source: io::Error) -> Self {
        Self::ReadScreenplay {
            path: path.to_owned(),
            source,
        }
    }

    pub(super) fn read_worker_request(path: &Path, source: io::Error) -> Self {
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

    pub(super) fn inspect_presentation(path: &Path, source: io::Error) -> Self {
        Self::InspectPresentation {
            path: path.to_owned(),
            source,
        }
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
            Self::InspectPresentation { path, .. } => {
                write!(
                    formatter,
                    "failed to inspect presentation {}",
                    path.display()
                )
            }
            Self::InvalidPresentation(path) => {
                write!(formatter, "presentation {} is not a file", path.display())
            }
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
            Self::InvalidProfile(source) => source.fmt(formatter),
            Self::Assets(source) => source.fmt(formatter),
            Self::Solve(source) => source.fmt(formatter),
            Self::Bundle(source) => source.fmt(formatter),
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
            Self::ReadScreenplay { source, .. }
            | Self::ReadWorkerRequest { source, .. }
            | Self::InspectPresentation { source, .. }
            | Self::CreateOutputDirectory { source, .. } => Some(source),
            Self::ParseWorkerRequest { source, .. } => Some(source),
            Self::WorkerTask(source) => Some(source),
            Self::InvalidPresentation(_) | Self::OutputExists(_) => None,
            Self::InvalidProfile(source) => Some(source),
            Self::Assets(source) => Some(source),
            Self::Solve(source) => Some(source),
            Self::Bundle(source) => Some(source),
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

impl From<InvalidRenderProfile> for CliError {
    fn from(source: InvalidRenderProfile) -> Self {
        Self::InvalidProfile(source)
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

impl From<BundleError> for CliError {
    fn from(source: BundleError) -> Self {
        Self::Bundle(source)
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
