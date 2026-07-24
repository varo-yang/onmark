//! Pure projection of normalized external captions onto a solved Timeline.

use std::error::Error;
use std::fmt;

use crate::model::{CaptionTrack, FrameConversionOverflow, FrameInterval, Rounding};
use crate::timeline::{TimelineCaption, TimelineIr};

/// Attaches normalized caption tracks to one solved Timeline.
///
/// Both cue boundaries use ceiling projection: a cue becomes visible at the
/// first frame whose timestamp reaches its start and remains visible for every
/// frame whose timestamp precedes its end. Cues outside the film are omitted;
/// cues crossing an edge are clipped to the executable film interval.
///
/// # Errors
///
/// Returns [`CaptionProjectionError`] when a cue time exceeds the frame domain.
///
/// # Panics
///
/// Panics only if one [`crate::model::Timebase`] projection reverses an already
/// ordered caption interval, which would violate the timebase's monotonicity
/// invariant.
pub fn import_captions(
    mut timeline: TimelineIr,
    tracks: impl IntoIterator<Item = CaptionTrack>,
) -> Result<TimelineIr, CaptionProjectionError> {
    let film = timeline.interval();
    let timebase = timeline.timebase();
    let mut captions = Vec::new();

    for cue in tracks.into_iter().flat_map(CaptionTrack::into_cues) {
        let (interval, text, timing_span, text_span) = cue.into_parts();
        let start = timebase.frame_at(interval.start(), Rounding::Ceil)?;
        let end = timebase.frame_at(interval.end(), Rounding::Ceil)?;
        let start = start.max(film.start());
        let end = end.min(film.end());
        if start >= end {
            continue;
        }
        let interval =
            FrameInterval::new(start, end).expect("clipped caption frame bounds are ordered");
        captions.push(TimelineCaption::new(interval, text, timing_span, text_span));
    }

    timeline.replace_captions(captions);
    Ok(timeline)
}

/// A normalized caption time outside the Timeline frame domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptionProjectionError(FrameConversionOverflow);

impl fmt::Display for CaptionProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "caption time cannot enter Timeline IR: {}",
            self.0
        )
    }
}

impl Error for CaptionProjectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

impl From<FrameConversionOverflow> for CaptionProjectionError {
    fn from(source: FrameConversionOverflow) -> Self {
        Self(source)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::compiler;
    use crate::model::{
        ByteOffset, CaptionCue, CaptionInterval, CaptionTrack, Duration, FrameRate, SourceId,
        SourceSpan, Timebase,
    };

    use super::import_captions;

    #[test]
    fn projects_and_clips_exact_caption_times_once() {
        let timeline = solved_timeline();
        let track = CaptionTrack::new(vec![
            cue(500_000_001, 1_500_000_001, "visible", 0),
            cue(1_500_000_000, 3_000_000_000, "clipped", 2),
            cue(3_000_000_000, 4_000_000_000, "outside", 4),
        ])
        .expect("the fixture cues are ordered");

        let timeline =
            import_captions(timeline, [track]).expect("caption times fit the frame grid");

        assert_eq!(timeline.captions().len(), 2);
        assert_eq!(timeline.captions()[0].interval().start().get(), 16);
        assert_eq!(timeline.captions()[0].interval().end().get(), 46);
        assert_eq!(timeline.captions()[1].interval().start().get(), 45);
        assert_eq!(timeline.captions()[1].interval().end().get(), 60);
    }

    fn solved_timeline() -> crate::timeline::TimelineIr {
        let parsed = compiler::parse(
            SourceId::new(0),
            r#"<om-film><om-scene><om-shot duration="2s"></om-shot></om-scene></om-film>"#,
        );
        let (document, diagnostics) = parsed.into_parts();
        assert!(diagnostics.is_empty());
        let (film, diagnostics) = compiler::bind(document).into_parts();
        assert!(diagnostics.is_empty());
        let (film, diagnostics) = compiler::resolve(film.expect("the film binds")).into_parts();
        assert!(diagnostics.is_empty());
        let rate = FrameRate::new(30, 1).expect("30 fps is valid");
        let report = compiler::solve(
            film.expect("the film resolves"),
            &BTreeMap::new(),
            Timebase::new(rate),
        )
        .expect("the fixture references no assets");
        assert!(report.diagnostics().is_empty());
        report.into_parts().0.expect("the film solves")
    }

    fn cue(start: u64, end: u64, text: &str, offset: u64) -> CaptionCue {
        let interval = CaptionInterval::new(Duration::from_nanos(start), Duration::from_nanos(end))
            .expect("the fixture interval is positive");
        let timing = span(offset, offset + 1);
        let text_span = span(offset + 1, offset + 2);
        CaptionCue::new(interval, text, timing, text_span).expect("the fixture cue is valid")
    }

    fn span(start: u64, end: u64) -> SourceSpan {
        SourceSpan::new(
            SourceId::new(1),
            ByteOffset::new(start),
            ByteOffset::new(end),
        )
        .expect("the fixture span is ordered")
    }
}
