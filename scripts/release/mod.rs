//! Deterministic assembly of generated desktop release artifacts.

mod artifact;
mod error;
mod media;
mod sidecar;
mod target;
mod version;

use std::path::Path;

use self::error::PackageError;

pub(super) fn run_sidecar(
    repository: &Path,
    arguments: impl Iterator<Item = String>,
) -> Result<(), PackageError> {
    sidecar::run(repository, arguments)
}

pub(super) fn prepare_version(
    repository: &Path,
    version: &str,
) -> Result<(), version::VersionError> {
    version::prepare(repository, version)
}

pub(super) fn verify_version(
    repository: &Path,
    expected: Option<&str>,
) -> Result<(), version::VersionError> {
    version::verify(repository, expected)
}
