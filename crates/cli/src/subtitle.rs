//! Bounded CLI ingestion and diagnostic translation for imported subtitles.

use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use onmark_core::diagnostics::{Diagnostic, DiagnosticCode};
use onmark_core::model::{CaptionTrack, SourceId};
use onmark_media::{SubtitleErrorKind, SubtitleLimits, parse_ass, parse_subrip, parse_webvtt};

const INPUT_LIMIT: usize = SubtitleLimits::MAX_INPUT_BYTES;
const CUE_LIMIT: usize = 10_000;
const CUE_TEXT_LIMIT: usize = 64 * 1024;

pub(super) enum SubtitleImport {
    Track(CaptionTrack),
    Rejected(RejectedSubtitle),
}

impl SubtitleImport {
    pub(super) fn load(path: &Path) -> Result<Self, SubtitleLoadError> {
        let limits = SubtitleLimits::new(INPUT_LIMIT, CUE_LIMIT, CUE_TEXT_LIMIT)
            .expect("the CLI subtitle limits stay inside the media safety envelope");
        let format = SubtitleFormat::from_path(path)?;
        let source = read_bounded(path)?;
        let report = format.parse(SourceId::new(1), &source, limits);
        let (track, errors) = report.into_parts();
        if errors.is_empty() {
            return Ok(Self::Track(
                track.expect("a valid subtitle report retains one track"),
            ));
        }
        let diagnostics = errors
            .into_iter()
            .map(|error| {
                Diagnostic::new(code(error.kind()), error.span(), error.to_string())
                    .expect("subtitle errors have non-blank messages")
            })
            .collect();
        Ok(Self::Rejected(RejectedSubtitle {
            path: path.to_owned(),
            source: String::from_utf8_lossy(&source).into_owned(),
            diagnostics,
        }))
    }
}

#[derive(Clone, Copy)]
enum SubtitleFormat {
    SubRip,
    WebVtt,
    Ass,
}

impl SubtitleFormat {
    fn from_path(path: &Path) -> Result<Self, SubtitleLoadError> {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        match extension.as_deref() {
            Some("srt") => Ok(Self::SubRip),
            Some("vtt") => Ok(Self::WebVtt),
            Some("ass") => Ok(Self::Ass),
            _ => Err(SubtitleLoadError::UnsupportedFormat(path.to_owned())),
        }
    }

    fn parse(
        self,
        source: SourceId,
        bytes: &[u8],
        limits: SubtitleLimits,
    ) -> onmark_media::SubtitleReport {
        match self {
            Self::SubRip => parse_subrip(source, bytes, limits),
            Self::WebVtt => parse_webvtt(source, bytes, limits),
            Self::Ass => parse_ass(source, bytes, limits),
        }
    }
}

pub(super) struct RejectedSubtitle {
    path: PathBuf,
    source: String,
    diagnostics: Vec<Diagnostic>,
}

impl RejectedSubtitle {
    pub(super) fn into_parts(self) -> (PathBuf, String, Vec<Diagnostic>) {
        (self.path, self.source, self.diagnostics)
    }
}

#[derive(Debug)]
pub(super) enum SubtitleLoadError {
    UnsupportedFormat(PathBuf),
    Open { path: PathBuf, source: io::Error },
    Read { path: PathBuf, source: io::Error },
}

impl fmt::Display for SubtitleLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat(path) => write!(
                formatter,
                "subtitle {} must use the .srt, .vtt, or .ass extension",
                path.display(),
            ),
            Self::Open { path, .. } => {
                write!(formatter, "failed to open subtitle {}", path.display())
            }
            Self::Read { path, .. } => {
                write!(formatter, "failed to read subtitle {}", path.display())
            }
        }
    }
}

impl Error for SubtitleLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Open { source, .. } | Self::Read { source, .. } => Some(source),
            Self::UnsupportedFormat(_) => None,
        }
    }
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, SubtitleLoadError> {
    let file = File::open(path).map_err(|source| SubtitleLoadError::Open {
        path: path.to_owned(),
        source,
    })?;
    let retained =
        u64::try_from(INPUT_LIMIT).expect("the fixed subtitle input limit fits in u64") + 1;
    let mut bytes = Vec::new();
    file.take(retained)
        .read_to_end(&mut bytes)
        .map_err(|source| SubtitleLoadError::Read {
            path: path.to_owned(),
            source,
        })?;
    Ok(bytes)
}

fn code(kind: SubtitleErrorKind) -> DiagnosticCode {
    match kind {
        SubtitleErrorKind::InputTooLarge
        | SubtitleErrorKind::TooManyCues
        | SubtitleErrorKind::CueTextTooLarge
        | SubtitleErrorKind::TooManyErrors => DiagnosticCode::SubtitleResourceLimit,
        SubtitleErrorKind::UnsupportedWebVttBlock
        | SubtitleErrorKind::UnsupportedWebVttCueSettings
        | SubtitleErrorKind::UnsupportedWebVttCueMarkup
        | SubtitleErrorKind::UnsupportedAssSection
        | SubtitleErrorKind::UnsupportedAssEventFields
        | SubtitleErrorKind::UnsupportedAssText => DiagnosticCode::UnsupportedSubtitleFeature,
        _ => DiagnosticCode::InvalidSubtitleFile,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{SubtitleImport, SubtitleLoadError};

    #[test]
    fn loads_valid_tracks_and_retains_bad_files_as_authored_diagnostics() {
        let directory = tempdir().expect("the fixture directory is available");
        let valid = directory.path().join("captions.srt");
        let invalid = directory.path().join("captions.vtt");
        fs::write(&valid, "1\n00:00:00,000 --> 00:00:01,000\nHello\n")
            .expect("the valid fixture is writable");
        fs::write(&invalid, "WEBVTT\n\n00:02.000 --> 00:01.000\nBad\n")
            .expect("the invalid fixture is writable");

        assert!(matches!(
            SubtitleImport::load(&valid).expect("the valid fixture is readable"),
            SubtitleImport::Track(_),
        ));
        let SubtitleImport::Rejected(rejected) =
            SubtitleImport::load(&invalid).expect("the invalid fixture is readable")
        else {
            panic!("the malformed fixture must retain authored diagnostics");
        };
        let (path, _, diagnostics) = rejected.into_parts();
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("captions.vtt")
        );
        assert_eq!(diagnostics[0].code().as_str(), "ONM-CAPTION-001");
    }

    #[test]
    fn rejects_an_unrecognized_subtitle_container() {
        let directory = tempdir().expect("the fixture directory is available");
        let path = directory.path().join("captions.txt");
        fs::write(&path, "captions").expect("the fixture is writable");

        assert!(matches!(
            SubtitleImport::load(&path),
            Err(SubtitleLoadError::UnsupportedFormat(_)),
        ));
    }
}
