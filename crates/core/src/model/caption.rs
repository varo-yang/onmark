//! Exact caption facts shared by external-format parsers and compilation.
//!
//! File syntax and browser presentation stay outside this module. A caption
//! track contains only validated timing, authored text, and source provenance.

use std::error::Error;
use std::fmt;

use super::{Duration, SourceSpan};

/// A non-empty sequence of normalized caption cues.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptionTrack {
    cues: Vec<CaptionCue>,
}

impl CaptionTrack {
    /// Creates a caption track containing at least one validated cue.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidCaptionTrack`] when no cue was supplied or cue starts
    /// move backwards in authored order.
    pub fn new(cues: Vec<CaptionCue>) -> Result<Self, InvalidCaptionTrack> {
        if cues.is_empty() {
            return Err(InvalidCaptionTrack::Empty);
        }
        for pair in cues.windows(2) {
            let previous = pair[0].interval().start();
            let current = pair[1].interval().start();
            if current < previous {
                return Err(InvalidCaptionTrack::OutOfOrder { previous, current });
            }
        }
        Ok(Self { cues })
    }

    /// Returns cues in their authored order.
    #[must_use]
    pub fn cues(&self) -> &[CaptionCue] {
        &self.cues
    }

    /// Consumes the track into cues in validated authored order.
    #[must_use]
    pub fn into_cues(self) -> Vec<CaptionCue> {
        self.cues
    }
}

/// One normalized caption with exact time and source provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptionCue {
    interval: CaptionInterval,
    text: Box<str>,
    timing_span: SourceSpan,
    text_span: SourceSpan,
}

impl CaptionCue {
    /// Creates a caption cue from a positive interval and non-blank text.
    ///
    /// Text is retained verbatim after the owning format parser has removed
    /// structural line endings. Styling syntax is not interpreted here.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidCaptionCue`] when the text contains no visible authored
    /// content.
    pub fn new(
        interval: CaptionInterval,
        text: impl Into<Box<str>>,
        timing_span: SourceSpan,
        text_span: SourceSpan,
    ) -> Result<Self, InvalidCaptionCue> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(InvalidCaptionCue::BlankText);
        }
        if timing_span.source() != text_span.source() {
            return Err(InvalidCaptionCue::MismatchedSources);
        }
        Ok(Self {
            interval,
            text,
            timing_span,
            text_span,
        })
    }

    /// Returns the exact half-open time interval occupied by the cue.
    #[must_use]
    pub const fn interval(&self) -> CaptionInterval {
        self.interval
    }

    /// Returns caption text with authored internal line breaks preserved.
    #[must_use]
    pub const fn text(&self) -> &str {
        &self.text
    }

    /// Returns the source range containing the authored timing expression.
    #[must_use]
    pub const fn timing_span(&self) -> SourceSpan {
        self.timing_span
    }

    /// Returns the source range containing the authored caption text.
    #[must_use]
    pub const fn text_span(&self) -> SourceSpan {
        self.text_span
    }

    /// Consumes the cue into normalized facts without copying its text.
    #[must_use]
    pub fn into_parts(self) -> (CaptionInterval, Box<str>, SourceSpan, SourceSpan) {
        (self.interval, self.text, self.timing_span, self.text_span)
    }
}

/// Exact half-open time interval occupied by one caption cue.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CaptionInterval {
    start: Duration,
    end: Duration,
}

impl CaptionInterval {
    /// Creates an interval whose end is strictly after its start.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidCaptionInterval`] for an empty or reversed interval.
    pub const fn new(start: Duration, end: Duration) -> Result<Self, InvalidCaptionInterval> {
        if end.as_nanos() <= start.as_nanos() {
            return Err(InvalidCaptionInterval { start, end });
        }
        Ok(Self { start, end })
    }

    /// Returns the inclusive exact start time.
    #[must_use]
    pub const fn start(self) -> Duration {
        self.start
    }

    /// Returns the exclusive exact end time.
    #[must_use]
    pub const fn end(self) -> Duration {
        self.end
    }
}

/// Reason a caption track cannot represent normalized content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidCaptionTrack {
    /// A track without cues has no renderable authored content.
    Empty,
    /// A later-authored cue begins before its predecessor.
    OutOfOrder {
        /// Start of the preceding cue.
        previous: Duration,
        /// Start of the rejected cue.
        current: Duration,
    },
}

impl fmt::Display for InvalidCaptionTrack {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("caption track must contain at least one cue"),
            Self::OutOfOrder { previous, current } => write!(
                formatter,
                "caption start {current} precedes the previous start {previous}",
            ),
        }
    }
}

impl Error for InvalidCaptionTrack {}

/// Reason normalized caption text cannot become a cue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidCaptionCue {
    /// The authored payload contains only whitespace.
    BlankText,
    /// Timing and text provenance name different source files.
    MismatchedSources,
}

impl fmt::Display for InvalidCaptionCue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BlankText => "caption text must contain non-whitespace content",
            Self::MismatchedSources => "caption timing and text must share one source",
        })
    }
}

impl Error for InvalidCaptionCue {}

/// An empty or reversed exact caption interval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidCaptionInterval {
    start: Duration,
    end: Duration,
}

impl InvalidCaptionInterval {
    /// Returns the rejected start time.
    #[must_use]
    pub const fn start(self) -> Duration {
        self.start
    }

    /// Returns the rejected end time.
    #[must_use]
    pub const fn end(self) -> Duration {
        self.end
    }
}

impl fmt::Display for InvalidCaptionInterval {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "caption end {} must be after start {}",
            self.end, self.start,
        )
    }
}

impl Error for InvalidCaptionInterval {}

#[cfg(test)]
mod tests {
    use super::{
        CaptionCue, CaptionInterval, CaptionTrack, InvalidCaptionCue, InvalidCaptionInterval,
        InvalidCaptionTrack,
    };
    use crate::model::{ByteOffset, Duration, SourceId, SourceSpan};

    #[test]
    fn constructs_one_exact_caption_track() {
        let interval = CaptionInterval::new(
            Duration::from_nanos(500_000_000),
            Duration::from_nanos(2_000_000_000),
        )
        .expect("the cue end follows its start");
        let timing_span = span(2, 31);
        let text_span = span(32, 44);
        let cue = CaptionCue::new(interval, "First\nline", timing_span, text_span)
            .expect("the cue contains text");
        let track = CaptionTrack::new(vec![cue]).expect("the track contains one cue");

        assert_eq!(track.cues()[0].interval(), interval);
        assert_eq!(track.cues()[0].text(), "First\nline");
        assert_eq!(track.cues()[0].timing_span(), timing_span);
        assert_eq!(track.cues()[0].text_span(), text_span);
    }

    #[test]
    fn rejects_empty_tracks_intervals_and_text() {
        assert_eq!(
            CaptionTrack::new(Vec::new()),
            Err(InvalidCaptionTrack::Empty)
        );
        assert_eq!(
            CaptionInterval::new(Duration::from_nanos(1), Duration::from_nanos(1)),
            Err(InvalidCaptionInterval {
                start: Duration::from_nanos(1),
                end: Duration::from_nanos(1),
            }),
        );

        let interval = CaptionInterval::new(Duration::ZERO, Duration::from_nanos(1))
            .expect("the interval is positive");
        assert_eq!(
            CaptionCue::new(interval, " \n", span(0, 1), span(1, 3)),
            Err(InvalidCaptionCue::BlankText),
        );
        let other_source =
            SourceSpan::new(SourceId::new(2), ByteOffset::new(1), ByteOffset::new(3))
                .expect("the fixture span is ordered");
        assert_eq!(
            CaptionCue::new(interval, "text", span(0, 1), other_source),
            Err(InvalidCaptionCue::MismatchedSources),
        );
    }

    #[test]
    fn rejects_cues_that_move_backwards_in_authored_order() {
        let cue = |start, end, offset| {
            let interval =
                CaptionInterval::new(Duration::from_nanos(start), Duration::from_nanos(end))
                    .expect("the fixture interval is positive");
            CaptionCue::new(
                interval,
                "text",
                span(offset, offset + 1),
                span(offset + 1, offset + 2),
            )
            .expect("the fixture cue is valid")
        };

        assert_eq!(
            CaptionTrack::new(vec![cue(2, 3, 0), cue(1, 4, 2)]),
            Err(InvalidCaptionTrack::OutOfOrder {
                previous: Duration::from_nanos(2),
                current: Duration::from_nanos(1),
            }),
        );
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
