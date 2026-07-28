//! Frozen language-admission evaluation entry points.

use std::error::Error;
use std::path::Path;

mod audio;
mod continuity;
mod envelope;
mod html;
mod transition;
mod video;

pub(super) fn grade_audio(repository: &Path) -> Result<(), Box<dyn Error>> {
    audio::grade(repository)
}

pub(super) fn grade_audio_envelope(repository: &Path) -> Result<(), Box<dyn Error>> {
    envelope::grade(repository)
}

pub(super) fn grade_html(repository: &Path) -> Result<(), Box<dyn Error>> {
    html::grade(repository)
}

pub(super) fn grade_transition(repository: &Path) -> Result<(), Box<dyn Error>> {
    transition::grade(repository)
}

pub(super) fn grade_video(repository: &Path) -> Result<(), Box<dyn Error>> {
    video::grade(repository)?;
    continuity::grade(repository)
}
