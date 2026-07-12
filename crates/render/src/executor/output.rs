use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::{RenderError, RenderErrorKind};
use crate::EncodedVideo;

#[derive(Debug)]
pub(super) struct StagedOutput {
    directory: TempDir,
    path: PathBuf,
}

impl StagedOutput {
    pub(super) fn new(output: &Path) -> Result<Self, RenderError> {
        if output.exists() {
            return Err(RenderError::new(
                RenderErrorKind::Output,
                output,
                "output already exists",
            ));
        }
        let parent = output_parent(output);
        let directory = tempfile::Builder::new()
            .prefix(".onmark-render-")
            .tempdir_in(parent)
            .map_err(|source| {
                RenderError::output_io(output, "failed to create output staging directory", source)
            })?;
        let path = directory.path().join("video.mp4");
        Ok(Self { directory, path })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn publish(
        self,
        video: EncodedVideo,
        output: &Path,
    ) -> Result<EncodedVideo, RenderError> {
        std::fs::hard_link(&self.path, output).map_err(|source| {
            RenderError::output_io(output, "failed to publish encoded video", source)
        })?;
        drop(self.directory);
        Ok(video.published_at(output.to_owned()))
    }
}

fn output_parent(output: &Path) -> &Path {
    output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[cfg(test)]
mod tests {
    use super::{StagedOutput, output_parent};
    use crate::RenderErrorKind;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn treats_a_relative_output_as_a_current_directory_artifact() {
        assert_eq!(output_parent(Path::new("video.mp4")), Path::new("."));
    }

    #[test]
    fn refuses_to_replace_an_existing_artifact() {
        let directory = tempdir().expect("the fixture directory is available");
        let output = directory.path().join("video.mp4");
        std::fs::write(&output, b"existing").expect("the fixture output is writable");

        let error = StagedOutput::new(&output).expect_err("publication is no-clobber");

        assert_eq!(error.kind(), RenderErrorKind::Output);
        assert_eq!(
            std::fs::read(output).expect("the original remains"),
            b"existing"
        );
    }

    #[test]
    fn removes_the_private_directory_when_staging_is_abandoned() {
        let directory = tempdir().expect("the fixture directory is available");
        let output = directory.path().join("video.mp4");
        let staging = StagedOutput::new(&output).expect("staging can be created");
        let staging_directory = staging.directory.path().to_owned();

        drop(staging);

        assert!(!staging_directory.exists());
        assert!(!output.exists());
    }
}
