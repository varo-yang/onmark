//! Lossless normalization of the plain-text `WebVTT` subset.

use std::iter::Peekable;
use std::str::FromStr as _;

use onmark_core::model::{CaptionCue, CaptionInterval, Duration, SourceId, SourceSpan};

use super::{
    BlockSeparator, SourceLine, SourceLines, SubtitleError, SubtitleErrorKind, SubtitleLimits,
    SubtitleReport, SubtitleSource, consume_separator, duration_from_clock, finish_report,
    insertion_span, is_decimal, push_cue, push_error, read_payload,
};

/// Parses the lossless plain-text `WebVTT` subset into exact caption facts.
///
/// The parser accepts the required `WEBVTT` header, comments, optional cue
/// identifiers, and exact millisecond timestamps. Regions, style blocks, cue
/// settings, markup, and character escapes remain explicit unsupported errors
/// until the caption fact model can preserve their semantics.
#[must_use]
pub fn parse_webvtt(source: SourceId, bytes: &[u8], limits: SubtitleLimits) -> SubtitleReport {
    let source = match SubtitleSource::decode(source, bytes, limits) {
        Ok(source) => source,
        Err(report) => return report,
    };
    WebVttParser::new(source, limits).parse()
}

struct WebVttParser<'a> {
    source: SourceId,
    source_end: usize,
    lines: Peekable<SourceLines<'a>>,
    limits: SubtitleLimits,
    blocks_seen: usize,
    cues: Vec<CaptionCue>,
    errors: Vec<SubtitleError>,
}

impl<'a> WebVttParser<'a> {
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
        if !self.consume_header() {
            return finish_report(self.source, self.source_end, self.cues, self.errors);
        }

        while let Some(first) = self.next_block_start() {
            if is_note(first.text) {
                self.skip_block();
                continue;
            }
            if !self.admit_block(first.span) {
                break;
            }
            if is_presentation_block(first.text) {
                self.reject(SubtitleErrorKind::UnsupportedWebVttBlock, first.span);
                self.skip_block();
                continue;
            }
            self.parse_cue(first);
        }

        finish_report(self.source, self.source_end, self.cues, self.errors)
    }

    fn consume_header(&mut self) -> bool {
        let Some(header) = self.lines.next() else {
            self.reject_header(super::span(self.source, 0, 0));
            return false;
        };
        if !is_header(header.text) {
            self.reject_header(header.span);
            return false;
        }

        let Some(separator) = self.lines.next() else {
            self.reject_header(insertion_span(header.span));
            return false;
        };
        if !separator.is_empty() {
            self.reject_header(separator.span);
            return false;
        }
        true
    }

    fn reject_header(&mut self, span: SourceSpan) {
        self.reject(SubtitleErrorKind::InvalidWebVttHeader, span);
    }

    fn next_block_start(&mut self) -> Option<SourceLine<'a>> {
        self.lines.find(|line| !line.is_empty())
    }

    fn admit_block(&mut self, span: SourceSpan) -> bool {
        // Invalid and unsupported blocks consume the cue budget so malformed
        // input cannot grow diagnostics beyond the normalized-track capacity.
        if self.blocks_seen == self.limits.max_cues() {
            self.reject(SubtitleErrorKind::TooManyCues, span);
            return false;
        }
        self.blocks_seen += 1;
        true
    }

    fn skip_block(&mut self) {
        while self.lines.peek().is_some_and(|line| !line.is_empty()) {
            self.lines.next();
        }
        consume_separator(&mut self.lines, BlockSeparator::EmptyLine);
    }

    fn parse_cue(&mut self, first: SourceLine<'a>) {
        let Some(timing_line) = self.cue_timing(first) else {
            return;
        };
        let interval = self.normalize_timing(timing_line);
        let payload = read_payload(
            &mut self.lines,
            timing_line.span,
            self.limits.max_cue_text_bytes(),
            BlockSeparator::EmptyLine,
        );
        let text_span = payload.span;
        let text = self.normalize_payload(payload);
        let (Some(interval), Some(text)) = (interval, text) else {
            return;
        };

        let cue = CaptionCue::new(interval, text, timing_line.span, text_span)
            .expect("WebVTT timing and text share one non-blank source block");
        push_cue(
            &mut self.cues,
            &mut self.errors,
            self.limits.max_errors(),
            cue,
        );
    }

    fn cue_timing(&mut self, first: SourceLine<'a>) -> Option<SourceLine<'a>> {
        if first.text.contains("-->") {
            return Some(first);
        }

        let Some(timing) = self.lines.next() else {
            self.reject(SubtitleErrorKind::MissingTiming, insertion_span(first.span));
            return None;
        };
        if timing.is_empty() {
            self.reject(SubtitleErrorKind::MissingTiming, timing.span);
            return None;
        }
        Some(timing)
    }

    fn normalize_timing(&mut self, line: SourceLine<'_>) -> Option<CaptionInterval> {
        let timing = match parse_timing(line.text) {
            Ok(timing) => timing,
            Err(kind) => {
                self.reject(kind, line.span);
                return None;
            }
        };

        let mut supported = true;
        if timing.has_settings {
            self.reject(SubtitleErrorKind::UnsupportedWebVttCueSettings, line.span);
            supported = false;
        }
        supported.then_some(timing.interval)
    }

    fn normalize_payload(&mut self, payload: super::CuePayload) -> Option<String> {
        let span = payload.span;
        let text = match payload.into_text() {
            Ok(text) => text,
            Err(kind) => {
                self.reject(kind, span);
                return None;
            }
        };
        if has_unsupported_markup(&text) {
            self.reject(SubtitleErrorKind::UnsupportedWebVttCueMarkup, span);
            return None;
        }
        Some(text)
    }

    fn reject(&mut self, kind: SubtitleErrorKind, span: SourceSpan) {
        push_error(&mut self.errors, self.limits.max_errors(), kind, span);
    }
}

#[derive(Clone, Copy)]
struct ParsedTiming {
    interval: CaptionInterval,
    has_settings: bool,
}

fn parse_timing(value: &str) -> Result<ParsedTiming, SubtitleErrorKind> {
    let Some((start, remainder)) = value.split_once("-->") else {
        return Err(SubtitleErrorKind::InvalidWebVttTiming);
    };
    if remainder.contains("-->") || !ends_with_space(start) || !starts_with_space(remainder) {
        return Err(SubtitleErrorKind::InvalidWebVttTiming);
    }

    let start =
        parse_timestamp(trim_space_end(start)).ok_or(SubtitleErrorKind::InvalidWebVttTiming)?;
    let remainder = trim_space_start(remainder);
    let end_length = remainder
        .bytes()
        .position(is_space)
        .unwrap_or(remainder.len());
    let end =
        parse_timestamp(&remainder[..end_length]).ok_or(SubtitleErrorKind::InvalidWebVttTiming)?;
    let interval =
        CaptionInterval::new(start, end).map_err(|_| SubtitleErrorKind::NonPositiveInterval)?;
    Ok(ParsedTiming {
        interval,
        has_settings: !trim_space(&remainder[end_length..]).is_empty(),
    })
}

fn parse_timestamp(value: &str) -> Option<Duration> {
    let mut parts = value.split(':');
    let first = parts.next()?;
    let second = parts.next()?;
    let third = parts.next();
    if parts.next().is_some() {
        return None;
    }

    let (hours, minutes, seconds) = match third {
        Some(seconds) if first.len() >= 2 => (first, second, seconds),
        Some(_) => return None,
        None => ("0", first, second),
    };
    if !is_clock_component(minutes) {
        return None;
    }
    let (seconds, milliseconds) = seconds.split_once('.')?;
    if !is_clock_component(seconds) || milliseconds.len() != 3 || !is_decimal(milliseconds) {
        return None;
    }
    if !is_decimal(hours) {
        return None;
    }

    let hours = u64::from_str(hours).ok()?;
    let minutes = u64::from_str(minutes).ok()?;
    let seconds = u64::from_str(seconds).ok()?;
    let milliseconds = u64::from_str(milliseconds).ok()?;
    duration_from_clock(hours, minutes, seconds, milliseconds)
}

fn is_header(value: &str) -> bool {
    value == "WEBVTT"
        || value
            .strip_prefix("WEBVTT")
            .is_some_and(|suffix| starts_with_space(suffix) && !suffix.contains("-->"))
}

fn is_note(value: &str) -> bool {
    value == "NOTE" || value.strip_prefix("NOTE").is_some_and(starts_with_space)
}

fn is_presentation_block(value: &str) -> bool {
    matches!(trim_space_end(value), "STYLE" | "REGION")
}

fn is_clock_component(value: &str) -> bool {
    value.len() == 2 && is_decimal(value)
}

fn starts_with_space(value: &str) -> bool {
    value.as_bytes().first().is_some_and(|byte| is_space(*byte))
}

fn ends_with_space(value: &str) -> bool {
    value.as_bytes().last().is_some_and(|byte| is_space(*byte))
}

fn trim_space(value: &str) -> &str {
    trim_space_end(trim_space_start(value))
}

fn trim_space_start(value: &str) -> &str {
    value.trim_start_matches([' ', '\t'])
}

fn trim_space_end(value: &str) -> &str {
    value.trim_end_matches([' ', '\t'])
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t')
}

fn has_unsupported_markup(value: &str) -> bool {
    value.contains(['<', '&']) || value.contains("-->")
}

#[cfg(test)]
mod tests {
    use onmark_core::model::SourceId;

    use super::{parse_timestamp, parse_timing, parse_webvtt};
    use crate::{SubtitleErrorKind, SubtitleLimits};

    #[test]
    fn accepts_bom_comments_identifiers_and_both_timestamp_forms() {
        let source = concat!(
            "\u{feff}WEBVTT\r\n\r\n",
            "NOTE context\r\nignored\r\n\r\n",
            "first\r\n00:00.500 --> 00:01.000\r\nHello\r\n\r\n",
            "00:00:01.000 --> 00:00:02.000\r\nWorld",
        );
        let report = parse_webvtt(SourceId::new(7), source.as_bytes(), limits());

        assert!(report.errors().is_empty());
        let track = report.track().expect("the plain WebVTT source is valid");
        assert_eq!(track.cues().len(), 2);
        assert_eq!(track.cues()[0].text(), "Hello");
        assert_eq!(track.cues()[1].text(), "World");
    }

    #[test]
    fn rejects_invalid_headers_and_out_of_order_cues() {
        let invalid_header = parse_webvtt(SourceId::new(0), b"VTT\n\n", limits());
        assert_eq!(
            invalid_header.errors()[0].kind(),
            SubtitleErrorKind::InvalidWebVttHeader,
        );
        let whitespace_separator = parse_webvtt(SourceId::new(0), b"WEBVTT\n \n", limits());
        assert_eq!(
            whitespace_separator.errors()[0].kind(),
            SubtitleErrorKind::InvalidWebVttHeader,
        );

        let out_of_order = parse_webvtt(
            SourceId::new(0),
            b"WEBVTT\n\n00:02.000 --> 00:03.000\nLater\n\n00:01.000 --> 00:02.000\nEarlier",
            limits(),
        );
        assert_eq!(
            out_of_order.errors()[0].kind(),
            SubtitleErrorKind::OutOfOrderCue,
        );
    }

    #[test]
    fn preserves_whitespace_only_payload_lines() {
        let report = parse_webvtt(
            SourceId::new(0),
            b"WEBVTT\n\n00:00.000 --> 00:01.000\nFirst\n \nSecond",
            limits(),
        );

        assert!(report.errors().is_empty());
        assert_eq!(
            report.track().expect("the cue is valid").cues()[0].text(),
            "First\n \nSecond",
        );
    }

    #[test]
    fn timestamps_are_exact_and_reject_ambiguous_widths() {
        assert_eq!(
            parse_timestamp("00:01.250").map(onmark_core::model::Duration::as_nanos),
            Some(1_250_000_000),
        );
        assert_eq!(
            parse_timestamp("100:00:01.250").map(onmark_core::model::Duration::as_nanos),
            Some(360_001_250_000_000),
        );
        assert_eq!(parse_timestamp("0:01.250"), None);
        assert_eq!(parse_timestamp("1:00:01.250"), None);
        assert_eq!(parse_timestamp("00:60.000"), None);
        assert!(matches!(
            parse_timing("00:00.000\u{a0}-->\u{a0}00:01.000"),
            Err(SubtitleErrorKind::InvalidWebVttTiming),
        ));
    }

    #[test]
    fn invalid_blocks_count_toward_the_bounded_cue_budget() {
        let limits = SubtitleLimits::new(4_096, 1, 256).expect("limits are valid");
        let report = parse_webvtt(
            SourceId::new(0),
            b"WEBVTT\n\nSTYLE\nred\n\n00:00.000 --> 00:01.000\ncaption",
            limits,
        );

        assert_eq!(report.errors().len(), 2);
        assert_eq!(
            report.errors()[0].kind(),
            SubtitleErrorKind::UnsupportedWebVttBlock,
        );
        assert_eq!(report.errors()[1].kind(), SubtitleErrorKind::TooManyCues,);
    }

    fn limits() -> SubtitleLimits {
        SubtitleLimits::new(4_096, 16, 256).expect("fixture limits are valid")
    }
}
