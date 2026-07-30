//! Frozen language-admission evaluation entry points.

use std::error::Error;
use std::path::Path;

mod audio;
mod caption;
mod continuity;
mod envelope;
mod html;
mod transition;
mod variant;
mod video;

pub(super) fn grade_audio(repository: &Path) -> Result<(), Box<dyn Error>> {
    audio::grade(repository)
}

pub(super) fn grade_audio_envelope(repository: &Path) -> Result<(), Box<dyn Error>> {
    envelope::grade(repository)
}

pub(super) fn grade_captions(repository: &Path) -> Result<(), Box<dyn Error>> {
    caption::grade(repository)
}

pub(super) fn grade_html(repository: &Path) -> Result<(), Box<dyn Error>> {
    html::grade(repository)
}

pub(super) fn grade_transition(repository: &Path) -> Result<(), Box<dyn Error>> {
    transition::grade(repository)
}

pub(super) fn grade_variant(repository: &Path) -> Result<(), Box<dyn Error>> {
    variant::grade(repository)
}

pub(super) fn grade_video(repository: &Path) -> Result<(), Box<dyn Error>> {
    video::grade(repository)?;
    continuity::grade(repository)
}
