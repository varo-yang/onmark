//! Bounded CLI ingestion and diagnostic translation for imported subtitles.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use onmark_core::compiler::ResolvedCaptionTrack;
use onmark_core::diagnostics::{Diagnostic, DiagnosticCode};
use onmark_core::model::{CaptionTrackId, ImportedCaptionTrack, SourceId};
use onmark_media::{SubtitleErrorKind, SubtitleLimits, parse_ass, parse_subrip, parse_webvtt};

const INPUT_LIMIT: usize = SubtitleLimits::MAX_INPUT_BYTES;
const CUE_LIMIT: usize = 10_000;
const CUE_TEXT_LIMIT: usize = 64 * 1024;

pub(super) enum CaptionImport {
    Ready(Vec<ImportedCaptionTrack>),
    Rejected(RejectedCaptions),
}

impl CaptionImport {
    pub(super) fn load(
        declarations: &[ResolvedCaptionTrack],
        selected: &[CaptionTrackId],
        source_directory: &Path,
    ) -> Result<Self, CaptionLoadError> {
        let declarations = select_declarations(declarations, selected)?;
        let mut tracks = Vec::with_capacity(declarations.len());

        for (index, declaration) in declarations.into_iter().enumerate() {
            let source_id = u32::try_from(index + 1)
                .expect("screenplay syntax bounds caption declaration count");
            match load_track(declaration, source_directory, SourceId::new(source_id))? {
                TrackImport::Imported(track) => tracks.push(track),
                TrackImport::Rejected(rejected) => return Ok(Self::Rejected(rejected)),
            }
        }

        Ok(Self::Ready(tracks))
    }
}

enum TrackImport {
    Imported(ImportedCaptionTrack),
    Rejected(RejectedCaptions),
}

fn load_track(
    declaration: &ResolvedCaptionTrack,
    source_directory: &Path,
    source_id: SourceId,
) -> Result<TrackImport, CaptionLoadError> {
    let path = source_directory.join(declaration.source().value().as_str());
    let limits = SubtitleLimits::new(INPUT_LIMIT, CUE_LIMIT, CUE_TEXT_LIMIT)
        .expect("the CLI subtitle limits stay inside the media safety envelope");
    let format = SubtitleFormat::from_path(&path)?;
    let source = read_bounded(&path)?;
    let report = format.parse(source_id, &source, limits);
    let (track, errors) = report.into_parts();
    if errors.is_empty() {
        let track = ImportedCaptionTrack::new(
            declaration.id().value().clone(),
            declaration.language().value().clone(),
            track.expect("a valid subtitle report retains one track"),
        );
        return Ok(TrackImport::Imported(track));
    }

    let diagnostics = errors
        .into_iter()
        .map(|error| {
            Diagnostic::new(code(error.kind()), error.span(), error.to_string())
                .expect("subtitle errors have non-blank messages")
        })
        .collect();
    Ok(TrackImport::Rejected(RejectedCaptions {
        path,
        source: String::from_utf8_lossy(&source).into_owned(),
        diagnostics,
    }))
}

fn select_declarations<'a>(
    declarations: &'a [ResolvedCaptionTrack],
    selected: &[CaptionTrackId],
) -> Result<Vec<&'a ResolvedCaptionTrack>, CaptionLoadError> {
    if selected.is_empty() {
        return Ok(declarations.iter().collect());
    }

    let index = declarations
        .iter()
        .map(|declaration| (declaration.id().value(), declaration))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    let mut chosen = Vec::with_capacity(selected.len());
    for id in selected {
        if !seen.insert(id) {
            return Err(CaptionLoadError::DuplicateTrack(id.clone()));
        }
        let declaration = index
            .get(id)
            .copied()
            .ok_or_else(|| CaptionLoadError::UnknownTrack(id.clone()))?;
        chosen.push(declaration);
    }
    Ok(chosen)
}

#[derive(Clone, Copy)]
enum SubtitleFormat {
    SubRip,
    WebVtt,
    Ass,
}

impl SubtitleFormat {
    fn from_path(path: &Path) -> Result<Self, CaptionLoadError> {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        match extension.as_deref() {
            Some("srt") => Ok(Self::SubRip),
            Some("vtt") => Ok(Self::WebVtt),
            Some("ass") => Ok(Self::Ass),
            _ => Err(CaptionLoadError::UnsupportedFormat(path.to_owned())),
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

pub(super) struct RejectedCaptions {
    path: PathBuf,
    source: String,
    diagnostics: Vec<Diagnostic>,
}

impl RejectedCaptions {
    pub(super) fn into_parts(self) -> (PathBuf, String, Vec<Diagnostic>) {
        (self.path, self.source, self.diagnostics)
    }
}

#[derive(Debug)]
pub(super) enum CaptionLoadError {
    UnknownTrack(CaptionTrackId),
    DuplicateTrack(CaptionTrackId),
    UnsupportedFormat(PathBuf),
    Open { path: PathBuf, source: io::Error },
    Read { path: PathBuf, source: io::Error },
}

impl fmt::Display for CaptionLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTrack(id) => write!(formatter, "caption track \"{id}\" is not declared"),
            Self::DuplicateTrack(id) => {
                write!(
                    formatter,
                    "caption track \"{id}\" is selected more than once"
                )
            }
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

impl Error for CaptionLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Open { source, .. } | Self::Read { source, .. } => Some(source),
            Self::UnknownTrack(_) | Self::DuplicateTrack(_) | Self::UnsupportedFormat(_) => None,
        }
    }
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, CaptionLoadError> {
    let file = File::open(path).map_err(|source| CaptionLoadError::Open {
        path: path.to_owned(),
        source,
    })?;
    let retained =
        u64::try_from(INPUT_LIMIT).expect("the fixed subtitle input limit fits in u64") + 1;
    let mut bytes = Vec::new();
    file.take(retained)
        .read_to_end(&mut bytes)
        .map_err(|source| CaptionLoadError::Read {
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

    use onmark_core::compiler;
    use onmark_core::model::{CaptionTrackId, ImportedCaptionTrack, SourceId};
    use tempfile::tempdir;

    use super::{CaptionImport, CaptionLoadError};

    #[test]
    fn loads_valid_tracks_and_retains_bad_files_as_authored_diagnostics() {
        let directory = tempdir().expect("the fixture directory is available");
        let valid = directory.path().join("captions.srt");
        let invalid = directory.path().join("captions.vtt");
        fs::write(&valid, "1\n00:00:00,000 --> 00:00:01,000\nHello\n")
            .expect("the valid fixture is writable");
        fs::write(&invalid, "WEBVTT\n\n00:02.000 --> 00:01.000\nBad\n")
            .expect("the invalid fixture is writable");

        let valid_film = resolved_film("captions.srt");
        assert!(matches!(
            CaptionImport::load(valid_film.captions(), &[], directory.path())
                .expect("the valid fixture is readable"),
            CaptionImport::Ready(_),
        ));
        let invalid_film = resolved_film("captions.vtt");
        let CaptionImport::Rejected(rejected) =
            CaptionImport::load(invalid_film.captions(), &[], directory.path())
                .expect("the invalid fixture is readable")
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
        let film = resolved_film("captions.txt");

        assert!(matches!(
            CaptionImport::load(film.captions(), &[], directory.path()),
            Err(CaptionLoadError::UnsupportedFormat(_)),
        ));
    }

    #[test]
    fn rejects_unknown_and_duplicate_track_selection() {
        let directory = tempdir().expect("the fixture directory is available");
        let film = resolved_film("captions.srt");
        let unknown = CaptionTrackId::parse("unknown").expect("the selection ID is valid");
        let en = CaptionTrackId::parse("en").expect("the selection ID is valid");

        assert!(matches!(
            CaptionImport::load(film.captions(), &[unknown], directory.path()),
            Err(CaptionLoadError::UnknownTrack(_)),
        ));
        assert!(matches!(
            CaptionImport::load(film.captions(), &[en.clone(), en], directory.path()),
            Err(CaptionLoadError::DuplicateTrack(_)),
        ));
    }

    #[test]
    fn preserves_authored_defaults_and_explicit_selection_order() {
        let directory = tempdir().expect("the fixture directory is available");
        let caption = "1\n00:00:00,000 --> 00:00:01,000\nHello\n";
        fs::write(directory.path().join("en.srt"), caption)
            .expect("the English fixture is writable");
        fs::write(directory.path().join("zh.srt"), caption)
            .expect("the Chinese fixture is writable");
        let film = resolved_film_with(
            "<om-captions id=\"en\" src=\"en.srt\" lang=\"en\"></om-captions>\
             <om-captions id=\"zh\" src=\"zh.srt\" lang=\"zh-CN\"></om-captions>",
        );

        let CaptionImport::Ready(defaults) =
            CaptionImport::load(film.captions(), &[], directory.path())
                .expect("both default tracks are readable")
        else {
            panic!("valid default tracks must import");
        };
        assert_eq!(track_ids(defaults), ["en", "zh"]);

        let selected = [
            CaptionTrackId::parse("zh").expect("the Chinese ID is valid"),
            CaptionTrackId::parse("en").expect("the English ID is valid"),
        ];
        let CaptionImport::Ready(selected) =
            CaptionImport::load(film.captions(), &selected, directory.path())
                .expect("both selected tracks are readable")
        else {
            panic!("valid selected tracks must import");
        };
        assert_eq!(track_ids(selected), ["zh", "en"]);
    }

    fn resolved_film(source: &str) -> compiler::ResolvedFilm {
        resolved_film_with(&format!(
            "<om-captions id=\"en\" src=\"{source}\" lang=\"en\"></om-captions>",
        ))
    }

    fn resolved_film_with(declarations: &str) -> compiler::ResolvedFilm {
        let source = format!(
            "<om-film>{declarations}\
             <om-scene><om-shot duration=\"1s\"></om-shot></om-scene></om-film>",
        );
        let (document, diagnostics) = compiler::parse(SourceId::new(0), &source).into_parts();
        assert!(diagnostics.is_empty());
        let (film, diagnostics) = compiler::bind(document).into_parts();
        assert!(diagnostics.is_empty());
        let (film, diagnostics) =
            compiler::resolve(film.expect("the fixture film binds")).into_parts();
        assert!(diagnostics.is_empty());
        film.expect("the fixture film resolves")
    }

    fn track_ids(tracks: Vec<ImportedCaptionTrack>) -> Vec<String> {
        tracks
            .into_iter()
            .map(|track| track.id().as_str().to_owned())
            .collect()
    }
}
