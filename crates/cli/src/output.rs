//! Filesystem policy shared by commands that publish local artifacts.
//!
//! Commands retain ownership of their artifact format and atomic publication;
//! this module owns only destination existence and parent-directory rules.

use std::fs;
use std::path::Path;

use crate::failure::CliError;

pub(super) fn reject_existing(path: &Path) -> Result<(), CliError> {
    if path.exists() {
        return Err(CliError::OutputExists(path.to_owned()));
    }
    Ok(())
}

pub(super) fn create_parent(path: &Path) -> Result<(), CliError> {
    let parent = parent(path);
    fs::create_dir_all(parent).map_err(|source| CliError::create_output_directory(parent, source))
}

pub(super) fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}
