//! Strict normalization of authored `SubRip` files.

use std::iter::Peekable;
use std::str::FromStr as _;

use onmark_core::model::{CaptionCue, CaptionInterval, Duration, SourceId};

use super::{
    BlockSeparator, SourceLine, SourceLines, SubtitleError, SubtitleErrorKind, SubtitleLimits,
    SubtitleReport, SubtitleSource, duration_from_clock, finish_report, insertion_span, is_decimal,
    push_cue, push_error, read_payload,
};

/// Parses strict UTF-8 `SubRip` into exact, source-located caption facts.
///
/// A UTF-8 BOM and either LF or CRLF line endings are accepted. Cue indices
/// are syntactic only; authored order is preserved because it remains the
/// deterministic order for simultaneously active captions. Payload text is
/// retained verbatim and no styling syntax is interpreted.
#[must_use]
pub fn parse_subrip(source: SourceId, bytes: &[u8], limits: SubtitleLimits) -> SubtitleReport {
    let source = match SubtitleSource::decode(source, bytes, limits) {
        Ok(source) => source,
        Err(report) => return report,
    };
    SubRipParser::new(source, limits).parse()
}

struct SubRipParser<'a> {
    source: SourceId,
    source_end: usize,
    lines: Peekable<SourceLines<'a>>,
    limits: SubtitleLimits,
    blocks_seen: usize,
    cues: Vec<CaptionCue>,
    errors: Vec<SubtitleError>,
}

impl<'a> SubRipParser<'a> {
    fn new(source: SubtitleSource<'a>, limits: SubtitleLimits) -> Self {
        Self {
            source: source.id,
            source_end: source.end(),
            lines: source.lines(),
            limits,
            blocks_seen: 0,
            cues: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn parse(mut self) -> SubtitleReport {
        while let Some(index) = self.next_block_start() {
            if self.blocks_seen == self.limits.max_cues() {
                self.reject(SubtitleErrorKind::TooManyCues, index.span);
                break;
            }
            self.blocks_seen += 1;
            self.parse_cue(index);
        }

        finish_report(self.source, self.source_end, self.cues, self.errors)
    }

    fn next_block_start(&mut self) -> Option<SourceLine<'a>> {
        self.lines.find(|line| !line.is_blank())
    }

    fn parse_cue(&mut self, index: SourceLine<'a>) {
        if !is_decimal(index.text) {
            self.reject(SubtitleErrorKind::InvalidSubRipIndex, index.span);
        }

        let Some(timing) = self.lines.next() else {
            self.reject(SubtitleErrorKind::MissingTiming, insertion_span(index.span));
            return;
        };
        if timing.is_blank() {
            self.reject(SubtitleErrorKind::MissingTiming, timing.span);
            return;
        }

        let interval = self.normalize_timing(timing);
        let payload = read_payload(
            &mut self.lines,
            timing.span,
            self.limits.max_cue_text_bytes(),
            BlockSeparator::BlankLine,
        );
        let text_span = payload.span;
        let text = self.normalize_payload(payload);
        let (Some(interval), Some(text)) = (interval, text) else {
            return;
        };

        let cue = CaptionCue::new(interval, text, timing.span, text_span)
            .expect("SubRip timing and text share one non-blank source block");
        push_cue(
            &mut self.cues,
            &mut self.errors,
            self.limits.max_errors(),
            cue,
        );
    }

    fn normalize_timing(&mut self, line: SourceLine<'_>) -> Option<CaptionInterval> {
        match parse_timing(line.text) {
            Ok(interval) => Some(interval),
            Err(kind) => {
                self.reject(kind, line.span);
                None
            }
        }
    }

    fn normalize_payload(&mut self, payload: super::CuePayload) -> Option<String> {
        let span = payload.span;
        match payload.into_text() {
            Ok(text) => Some(text),
            Err(kind) => {
                self.reject(kind, span);
                None
            }
        }
    }

    fn reject(&mut self, kind: SubtitleErrorKind, span: onmark_core::model::SourceSpan) {
        push_error(&mut self.errors, self.limits.max_errors(), kind, span);
    }
}

fn parse_timing(value: &str) -> Result<CaptionInterval, SubtitleErrorKind> {
    let Some((start, end)) = value.split_once("-->") else {
        return Err(SubtitleErrorKind::InvalidSubRipTiming);
    };
    if end.contains("-->") {
        return Err(SubtitleErrorKind::InvalidSubRipTiming);
    }

    let start = parse_timestamp(start.trim()).ok_or(SubtitleErrorKind::InvalidSubRipTiming)?;
    let end = parse_timestamp(end.trim()).ok_or(SubtitleErrorKind::InvalidSubRipTiming)?;
    CaptionInterval::new(start, end).map_err(|_| SubtitleErrorKind::NonPositiveInterval)
}

fn parse_timestamp(value: &str) -> Option<Duration> {
    let mut clock = value.split(':');
    let hours = clock.next()?;
    let minutes = clock.next()?;
    let seconds = clock.next()?;
    if clock.next().is_some() || hours.is_empty() || !is_decimal(hours) {
        return None;
    }
    if minutes.len() != 2 || !is_decimal(minutes) {
        return None;
    }
    let (seconds, milliseconds) = seconds.split_once(',')?;
    if seconds.len() != 2 || !is_decimal(seconds) {
        return None;
    }
    if milliseconds.len() != 3 || !is_decimal(milliseconds) {
        return None;
    }

    let hours = u64::from_str(hours).ok()?;
    let minutes = u64::from_str(minutes).ok()?;
    let seconds = u64::from_str(seconds).ok()?;
    let milliseconds = u64::from_str(milliseconds).ok()?;
    duration_from_clock(hours, minutes, seconds, milliseconds)
}

#[cfg(test)]
mod tests {
    use onmark_core::model::{Duration, SourceId};

    use super::{parse_subrip, parse_timestamp};
    use crate::{InvalidSubtitleLimits, SubtitleErrorKind, SubtitleLimits};

    #[test]
    fn parses_bom_crlf_multiline_and_overlapping_cues() {
        let source = concat!(
            "\u{feff}1\r\n",
            "00:00:00,500 --> 00:00:02,000\r\n",
            "Hello,\r\n",
            "world.\r\n",
            "\r\n",
            "2\r\n",
            "00:00:01,750 --> 00:01:04,250\r\n",
            "Second cue\r\n",
        );

        let report = parse_subrip(SourceId::new(4), source.as_bytes(), limits());
        assert!(report.errors().is_empty());
        let track = report.track().expect("the complete source is valid");

        assert_eq!(track.cues().len(), 2);
        assert_eq!(track.cues()[0].text(), "Hello,\nworld.");
        assert_eq!(
            track.cues()[0].interval().start(),
            Duration::from_nanos(500_000_000),
        );
        assert_eq!(
            track.cues()[1].interval().end(),
            Duration::from_nanos(64_250_000_000),
        );
        assert_eq!(track.cues()[0].timing_span().start().get(), 6);
        assert_eq!(track.cues()[0].text_span().start().get(), 37);
    }

    #[test]
    fn aggregates_independent_well_framed_cue_errors() {
        let source = concat!(
            "first\n",
            "00:00:02,000 --> 00:00:01,000\n",
            "Text\n",
            "\n",
            "2\n",
            "not timing\n",
            "Other text\n",
            "\n",
            "3\n",
        );

        let report = parse_subrip(SourceId::new(0), source.as_bytes(), limits());
        let kinds = report
            .errors()
            .iter()
            .map(crate::SubtitleError::kind)
            .collect::<Vec<_>>();

        assert!(report.track().is_none());
        assert_eq!(
            kinds,
            [
                SubtitleErrorKind::InvalidSubRipIndex,
                SubtitleErrorKind::NonPositiveInterval,
                SubtitleErrorKind::InvalidSubRipTiming,
                SubtitleErrorKind::MissingTiming,
            ],
        );
    }

    #[test]
    fn rejects_invalid_utf8_and_every_resource_limit() {
        let utf8 = parse_subrip(SourceId::new(0), &[b'1', b'\n', 0xff], limits());
        assert_eq!(utf8.errors()[0].kind(), SubtitleErrorKind::InvalidUtf8);
        assert_eq!(utf8.errors()[0].span().start().get(), 2);

        let input_limits = SubtitleLimits::new(4, 16, 256).expect("limits are valid");
        let input = parse_subrip(SourceId::new(0), b"12345", input_limits);
        assert_eq!(input.errors()[0].kind(), SubtitleErrorKind::InputTooLarge);

        let cue_limits = SubtitleLimits::new(4_096, 1, 256).expect("limits are valid");
        let cues = parse_subrip(
            SourceId::new(0),
            b"1\n00:00:00,000 --> 00:00:01,000\na\n\n2\n00:00:01,000 --> 00:00:02,000\nb\n",
            cue_limits,
        );
        assert_eq!(cues.errors()[0].kind(), SubtitleErrorKind::TooManyCues);

        let text_limits = SubtitleLimits::new(4_096, 16, 3).expect("limits are valid");
        let text = parse_subrip(
            SourceId::new(0),
            b"1\n00:00:00,000 --> 00:00:01,000\nlong\n",
            text_limits,
        );
        assert_eq!(text.errors()[0].kind(), SubtitleErrorKind::CueTextTooLarge,);
    }

    #[test]
    fn counts_invalid_blocks_toward_the_cue_limit() {
        let limits = SubtitleLimits::new(4_096, 1, 256).expect("limits are valid");
        let report = parse_subrip(
            SourceId::new(0),
            b"bad\nnot timing\ntext\n\n2\n00:00:01,000 --> 00:00:02,000\nsecond\n",
            limits,
        );

        assert_eq!(
            report
                .errors()
                .last()
                .expect("the second block exceeds the limit")
                .kind(),
            SubtitleErrorKind::TooManyCues,
        );
    }

    #[test]
    fn rejects_cues_that_move_backwards_on_the_shared_track() {
        let report = parse_subrip(
            SourceId::new(0),
            b"1\n00:00:02,000 --> 00:00:03,000\nLater\n\n2\n00:00:01,000 --> 00:00:02,000\nEarlier\n",
            limits(),
        );

        assert!(report.track().is_none());
        assert_eq!(report.errors()[0].kind(), SubtitleErrorKind::OutOfOrderCue);
        assert_eq!(report.errors()[0].span().start().get(), 41);
    }

    #[test]
    fn rejects_invalid_limit_configuration() {
        assert_eq!(
            SubtitleLimits::new(0, 1, 1),
            Err(InvalidSubtitleLimits::ZeroInputBytes),
        );
        assert_eq!(
            SubtitleLimits::new(SubtitleLimits::MAX_INPUT_BYTES + 1, 1, 1),
            Err(InvalidSubtitleLimits::InputBytesTooLarge),
        );
        assert_eq!(
            SubtitleLimits::new(1, 0, 1),
            Err(InvalidSubtitleLimits::ZeroCues),
        );
        assert_eq!(
            SubtitleLimits::new(1, SubtitleLimits::MAX_CUES + 1, 1),
            Err(InvalidSubtitleLimits::CueLimitTooLarge),
        );
        assert_eq!(
            SubtitleLimits::new(1, 1, 0),
            Err(InvalidSubtitleLimits::ZeroCueTextBytes),
        );
        assert_eq!(
            SubtitleLimits::new(1, 1, SubtitleLimits::MAX_CUE_TEXT_BYTES + 1),
            Err(InvalidSubtitleLimits::CueTextBytesTooLarge),
        );
    }

    #[test]
    fn parses_only_exact_subrip_timestamps() {
        assert_eq!(
            parse_timestamp("123:45:56,789"),
            Some(Duration::from_nanos(445_556_789_000_000)),
        );
        assert_eq!(parse_timestamp("00:60:00,000"), None);
        assert_eq!(parse_timestamp("00:00:60,000"), None);
        assert_eq!(parse_timestamp("00:00:00.000"), None);
        assert_eq!(parse_timestamp("0:00:00,000"), Some(Duration::ZERO));
    }

    fn limits() -> SubtitleLimits {
        SubtitleLimits::new(4_096, 16, 256).expect("fixture limits are valid")
    }
}
