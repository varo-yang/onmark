//! Frozen language-admission evaluation entry points.

use std::error::Error;
use std::path::Path;

mod audio;
mod html;
mod video;

pub(super) fn grade_audio(repository: &Path) -> Result<(), Box<dyn Error>> {
    audio::grade(repository)
}

pub(super) fn grade_html(repository: &Path) -> Result<(), Box<dyn Error>> {
    html::grade(repository)
}

pub(super) fn grade_video(repository: &Path) -> Result<(), Box<dyn Error>> {
    video::grade(repository)
}
