//! Deterministic assembly of generated desktop release artifacts.

mod artifact;
mod error;
mod media;
mod sidecar;
mod target;

use std::path::Path;

use self::error::PackageError;

pub(super) fn run_sidecar(
    repository: &Path,
    arguments: impl Iterator<Item = String>,
) -> Result<(), PackageError> {
    sidecar::run(repository, arguments)
}
