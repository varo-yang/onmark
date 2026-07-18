//! Lossless normalization of the plain-event ASS subset.

use std::iter::Peekable;
use std::str::FromStr as _;

use onmark_core::model::{CaptionCue, CaptionInterval, Duration, SourceId, SourceSpan};

use super::{
    SourceLine, SourceLines, SubtitleError, SubtitleErrorKind, SubtitleLimits, SubtitleReport,
    SubtitleSource, duration_from_clock, finish_report, insert_error, is_decimal, push_cue,
    push_error, span,
};

const SCRIPT_INFO: &str = "[Script Info]";
const EVENTS: &str = "[Events]";
const FORMAT_PREFIX: &str = "Format:";
const DIALOGUE_PREFIX: &str = "Dialogue:";

/// Parses the lossless plain-event ASS subset into exact caption facts.
///
/// The subset accepts `ScriptType: v4.00+` and event records whose format is
/// exactly `Start, End, Text`. Presentation resolution, styles, event layout
/// fields, effects, override tags, and drawings remain explicit unsupported
/// errors until the caption fact model can preserve their semantics.
#[must_use]
pub fn parse_ass(source: SourceId, bytes: &[u8], limits: SubtitleLimits) -> SubtitleReport {
    let source = match SubtitleSource::decode(source, bytes, limits) {
        Ok(source) => source,
        Err(report) => return report,
    };
    AssParser::new(source, limits).parse()
}

#[derive(Clone, Copy)]
enum Section {
    ScriptInfo,
    Events,
    Unsupported,
}

#[derive(Clone, Copy)]
enum EventFormat {
    Plain,
    Rejected,
}

struct AssParser<'a> {
    source: SourceId,
    source_end: usize,
    lines: Peekable<SourceLines<'a>>,
    limits: SubtitleLimits,
    section: Option<Section>,
    format: Option<EventFormat>,
    script_info_span: Option<SourceSpan>,
    script_type_span: Option<SourceSpan>,
    unsupported_script_info_span: Option<SourceSpan>,
    events_span: Option<SourceSpan>,
    dialogues_seen: usize,
    cues: Vec<CaptionCue>,
    errors: Vec<SubtitleError>,
}

impl<'a> AssParser<'a> {
    fn new(source: SubtitleSource<'a>, limits: SubtitleLimits) -> Self {
        Self {
            source: source.id,
            source_end: source.end(),
            lines: source.lines(),
            limits,
            section: None,
            format: None,
            script_info_span: None,
            script_type_span: None,
            unsupported_script_info_span: None,
            events_span: None,
            dialogues_seen: 0,
            cues: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn parse(mut self) -> SubtitleReport {
        while let Some(line) = self.lines.next() {
            if line.is_blank() || is_comment(line.text) {
                continue;
            }
            if is_section(line.text) {
                self.enter_section(line);
                continue;
            }
            self.parse_section_line(line);
        }

        self.finish_header();
        finish_report(self.source, self.source_end, self.cues, self.errors)
    }

    fn enter_section(&mut self, line: SourceLine<'_>) {
        if self.section.is_none() && line.text != SCRIPT_INFO {
            self.reject(SubtitleErrorKind::InvalidAssHeader, line.span);
        }
        self.format = None;
        self.section = Some(match line.text {
            SCRIPT_INFO => {
                self.script_info_span.get_or_insert(line.span);
                Section::ScriptInfo
            }
            EVENTS => {
                self.events_span.get_or_insert(line.span);
                Section::Events
            }
            _ => {
                self.reject(SubtitleErrorKind::UnsupportedAssSection, line.span);
                Section::Unsupported
            }
        });
    }

    fn parse_section_line(&mut self, line: SourceLine<'_>) {
        match self.section {
            Some(Section::ScriptInfo) => self.parse_script_info(line),
            Some(Section::Events) => self.parse_event(line),
            Some(Section::Unsupported) => {}
            None => {
                self.reject(SubtitleErrorKind::InvalidAssHeader, line.span);
                self.section = Some(Section::Unsupported);
            }
        }
    }

    fn parse_script_info(&mut self, line: SourceLine<'_>) {
        if line.text == "ScriptType: v4.00+" {
            self.script_type_span.get_or_insert(line.span);
            return;
        }
        if is_script_metadata(line.text) || self.unsupported_script_info_span.is_some() {
            return;
        }
        self.reject(SubtitleErrorKind::UnsupportedAssSection, line.span);
        self.unsupported_script_info_span = Some(line.span);
    }

    fn parse_event(&mut self, line: SourceLine<'_>) {
        if let Some(value) = line.text.strip_prefix(FORMAT_PREFIX) {
            self.parse_format(line, trim_space(value));
            return;
        }
        if line.text.starts_with("Comment:") {
            return;
        }
        if line.text.starts_with(DIALOGUE_PREFIX) {
            self.parse_dialogue(line);
            return;
        }
        self.reject(SubtitleErrorKind::InvalidAssDialogue, line.span);
    }

    fn parse_format(&mut self, line: SourceLine<'_>, value: &str) {
        self.format = Some(match classify_format(value) {
            FormatKind::Plain => EventFormat::Plain,
            FormatKind::Incomplete => {
                self.reject(SubtitleErrorKind::InvalidAssFormat, line.span);
                EventFormat::Rejected
            }
            FormatKind::Presentation => {
                self.reject(SubtitleErrorKind::UnsupportedAssEventFields, line.span);
                EventFormat::Rejected
            }
        });
    }

    fn parse_dialogue(&mut self, line: SourceLine<'_>) {
        if !self.admit_dialogue(line.span) {
            return;
        }
        match self.format {
            Some(EventFormat::Plain) => self.normalize_dialogue(line),
            Some(EventFormat::Rejected) => {}
            None => self.reject(SubtitleErrorKind::InvalidAssDialogue, line.span),
        }
    }

    fn admit_dialogue(&mut self, span: SourceSpan) -> bool {
        if self.dialogues_seen == self.limits.max_cues() {
            self.reject(SubtitleErrorKind::TooManyCues, span);
            return false;
        }
        self.dialogues_seen += 1;
        true
    }

    fn normalize_dialogue(&mut self, line: SourceLine<'_>) {
        let Some(fields) = DialogueFields::parse(line) else {
            self.reject(SubtitleErrorKind::InvalidAssDialogue, line.span);
            return;
        };
        let interval = self.normalize_timing(fields.start, fields.end, fields.timing_span);
        let text = self.normalize_text(fields.text, fields.text_span);
        let (Some(interval), Some(text)) = (interval, text) else {
            return;
        };

        let cue = CaptionCue::new(interval, text, fields.timing_span, fields.text_span)
            .expect("ASS timing and text share one non-blank dialogue record");
        push_cue(
            &mut self.cues,
            &mut self.errors,
            self.limits.max_errors(),
            cue,
        );
    }

    fn normalize_timing(
        &mut self,
        start: &str,
        end: &str,
        span: SourceSpan,
    ) -> Option<CaptionInterval> {
        let timing = parse_timestamp(trim_space(start)).zip(parse_timestamp(trim_space(end)));
        let Some((start, end)) = timing else {
            self.reject(SubtitleErrorKind::InvalidAssTiming, span);
            return None;
        };
        let Ok(interval) = CaptionInterval::new(start, end) else {
            self.reject(SubtitleErrorKind::NonPositiveInterval, span);
            return None;
        };
        Some(interval)
    }

    fn normalize_text(&mut self, value: &str, span: SourceSpan) -> Option<String> {
        if value.len() > self.limits.max_cue_text_bytes() {
            self.reject(SubtitleErrorKind::CueTextTooLarge, span);
            return None;
        }
        let Ok(text) = decode_text(value) else {
            self.reject(SubtitleErrorKind::UnsupportedAssText, span);
            return None;
        };
        if text.trim().is_empty() {
            self.reject(SubtitleErrorKind::MissingText, span);
            return None;
        }
        Some(text)
    }

    fn finish_header(&mut self) {
        if self.script_type_span.is_none() {
            let span = self
                .script_info_span
                .unwrap_or_else(|| super::span(self.source, 0, 0));
            self.reject_in_source_order(SubtitleErrorKind::InvalidAssHeader, span);
        }
        if self.events_span.is_none() {
            self.reject(
                SubtitleErrorKind::MissingAssEvents,
                span(self.source, self.source_end, self.source_end),
            );
        }
    }

    fn reject(&mut self, kind: SubtitleErrorKind, span: SourceSpan) {
        push_error(&mut self.errors, self.limits.max_errors(), kind, span);
    }

    fn reject_in_source_order(&mut self, kind: SubtitleErrorKind, span: SourceSpan) {
        insert_error(&mut self.errors, self.limits.max_errors(), kind, span);
    }
}

struct DialogueFields<'a> {
    start: &'a str,
    end: &'a str,
    text: &'a str,
    timing_span: SourceSpan,
    text_span: SourceSpan,
}

impl<'a> DialogueFields<'a> {
    fn parse(line: SourceLine<'a>) -> Option<Self> {
        let value = line.text.strip_prefix(DIALOGUE_PREFIX)?;
        let value = trim_space_start(value);
        let value_start = line.text.len() - value.len();
        let first_comma = value.find(',')?;
        let second_comma = value[first_comma + 1..].find(',')? + first_comma + 1;
        let text_start = second_comma + 1;

        Some(Self {
            start: &value[..first_comma],
            end: &value[first_comma + 1..second_comma],
            text: &value[text_start..],
            timing_span: line.subspan(value_start, value_start + second_comma),
            text_span: line.subspan(value_start + text_start, line.text.len()),
        })
    }
}

#[derive(Clone, Copy)]
enum FormatKind {
    Plain,
    Incomplete,
    Presentation,
}

fn classify_format(value: &str) -> FormatKind {
    let mut start = None;
    let mut end = None;
    let mut text = None;
    let mut count = 0;
    let mut malformed = false;
    for (index, field) in value.split(',').map(trim_space).enumerate() {
        count += 1;
        match field {
            "Start" if start.is_none() => start = Some(index),
            "End" if end.is_none() => end = Some(index),
            "Text" if text.is_none() => text = Some(index),
            "Start" | "End" | "Text" | "" => malformed = true,
            _ => {}
        }
    }

    if malformed {
        return FormatKind::Incomplete;
    }
    match (start, end, text) {
        (Some(0), Some(1), Some(2)) if count == 3 => FormatKind::Plain,
        (Some(start), Some(end), Some(text)) if start < end && end < text => {
            FormatKind::Presentation
        }
        _ => FormatKind::Incomplete,
    }
}

fn parse_timestamp(value: &str) -> Option<Duration> {
    let mut clock = value.split(':');
    let hours = clock.next()?;
    let minutes = clock.next()?;
    let seconds = clock.next()?;
    if clock.next().is_some() || !is_decimal(hours) || !is_clock_component(minutes) {
        return None;
    }
    let (seconds, centiseconds) = seconds.split_once('.')?;
    if !is_clock_component(seconds) || centiseconds.len() != 2 || !is_decimal(centiseconds) {
        return None;
    }

    let hours = u64::from_str(hours).ok()?;
    let minutes = u64::from_str(minutes).ok()?;
    let seconds = u64::from_str(seconds).ok()?;
    let milliseconds = u64::from_str(centiseconds).ok()?.checked_mul(10)?;
    duration_from_clock(hours, minutes, seconds, milliseconds)
}

fn decode_text(value: &str) -> Result<String, ()> {
    if value.contains(['{', '}']) {
        return Err(());
    }

    let mut output = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        match characters.next() {
            Some('N') => output.push('\n'),
            Some('h') => output.push('\u{a0}'),
            _ => return Err(()),
        }
    }
    Ok(output)
}

fn is_section(value: &str) -> bool {
    value.starts_with('[') && value.ends_with(']')
}

fn is_comment(value: &str) -> bool {
    value.starts_with(';')
}

fn is_script_metadata(value: &str) -> bool {
    let Some((name, _)) = value.split_once(':') else {
        return false;
    };
    matches!(
        name,
        "Title"
            | "Original Script"
            | "Original Translation"
            | "Original Editing"
            | "Original Timing"
            | "Synch Point"
            | "Script Updated By"
            | "Update Details"
    )
}

fn is_clock_component(value: &str) -> bool {
    value.len() == 2 && is_decimal(value)
}

fn trim_space(value: &str) -> &str {
    value.trim_matches([' ', '\t'])
}

fn trim_space_start(value: &str) -> &str {
    value.trim_start_matches([' ', '\t'])
}

#[cfg(test)]
mod tests {
    use onmark_core::model::SourceId;

    use super::{FormatKind, classify_format, decode_text, parse_ass, parse_timestamp};
    use crate::{SubtitleErrorKind, SubtitleLimits};

    #[test]
    fn accepts_bom_crlf_comments_commas_and_plain_text_escapes() {
        let source = concat!(
            "\u{feff}[Script Info]\r\n",
            "; authored comment\r\n",
            "ScriptType: v4.00+\r\n\r\n",
            "[Events]\r\n",
            "Format: Start, End, Text\r\n",
            "Dialogue: 0:00:00.50,0:00:01.00,Hello\\Nworld\\h!\r\n",
            "Dialogue: 0:00:01.00,0:00:02.00,Comma, remains",
        );
        let report = parse_ass(SourceId::new(3), source.as_bytes(), limits());

        assert!(report.errors().is_empty());
        let track = report.track().expect("the plain ASS source is valid");
        assert_eq!(track.cues().len(), 2);
        assert_eq!(track.cues()[0].text(), "Hello\nworld\u{a0}!");
        assert_eq!(track.cues()[1].text(), "Comma, remains");
    }

    #[test]
    fn requires_the_script_header_and_events_section() {
        let report = parse_ass(SourceId::new(0), b"[Script Info]\n", limits());
        assert_eq!(report.errors().len(), 2);
        assert_eq!(
            report.errors()[0].kind(),
            SubtitleErrorKind::InvalidAssHeader
        );
        assert_eq!(
            report.errors()[1].kind(),
            SubtitleErrorKind::MissingAssEvents
        );
    }

    #[test]
    fn keeps_header_validation_independent_from_script_metadata() {
        let source = concat!(
            "[Script Info]\n",
            "Title: Example\n",
            "PlayResX: 1920\n",
            "ScriptType: v4.00+\n\n",
            "[Events]\n",
            "Format: Start, End, Text\n",
            "Dialogue: 0:00:00.00,0:00:01.00,caption",
        );
        let report = parse_ass(SourceId::new(0), source.as_bytes(), limits());

        assert_eq!(report.errors().len(), 1);
        assert_eq!(
            report.errors()[0].kind(),
            SubtitleErrorKind::UnsupportedAssSection,
        );

        let out_of_order = concat!(
            "[Events]\nFormat: Start, End, Text\n",
            "[Script Info]\nScriptType: v4.00+\n",
        );
        let report = parse_ass(SourceId::new(0), out_of_order.as_bytes(), limits());
        assert_eq!(
            report.errors()[0].kind(),
            SubtitleErrorKind::InvalidAssHeader
        );
    }

    #[test]
    fn timestamps_and_text_decoding_are_exact() {
        assert_eq!(
            parse_timestamp("12:34:56.78").map(onmark_core::model::Duration::as_nanos),
            Some(45_296_780_000_000),
        );
        assert_eq!(parse_timestamp("0:60:00.00"), None);
        assert_eq!(parse_timestamp("0:00:00.000"), None);
        assert_eq!(decode_text("a\\Nb\\hc"), Ok("a\nb\u{a0}c".into()));
        assert_eq!(decode_text("{\\i1}styled"), Err(()));
        assert_eq!(decode_text("soft\\nbreak"), Err(()));
    }

    #[test]
    fn classifies_event_fields_without_hiding_malformed_formats() {
        assert!(matches!(
            classify_format("Start, End, Text"),
            FormatKind::Plain,
        ));
        assert!(matches!(
            classify_format("Layer, Start, End, Style, Text"),
            FormatKind::Presentation,
        ));
        assert!(matches!(
            classify_format("Start, End, Text,"),
            FormatKind::Incomplete,
        ));
        assert!(matches!(
            classify_format("Start, End, Text, Start"),
            FormatKind::Incomplete,
        ));
    }

    #[test]
    fn invalid_dialogues_count_toward_the_cue_budget() {
        let limits = SubtitleLimits::new(4_096, 1, 256).expect("limits are valid");
        let report = parse_ass(
            SourceId::new(0),
            b"[Script Info]\nScriptType: v4.00+\n\n[Events]\nFormat: Start, End, Text\nDialogue: bad,also bad,text\nDialogue: 0:00:00.00,0:00:01.00,second",
            limits,
        );

        assert_eq!(report.errors().len(), 2);
        assert_eq!(
            report.errors()[0].kind(),
            SubtitleErrorKind::InvalidAssTiming
        );
        assert_eq!(report.errors()[1].kind(), SubtitleErrorKind::TooManyCues);
    }

    #[test]
    fn bounds_retained_errors_from_hostile_section_streams() {
        let mut source = String::from("[Script Info]\nScriptType: v4.00+\n");
        for _ in 0..SubtitleLimits::MAX_ERRORS + 8 {
            source.push_str("[Unsupported]\n");
        }

        let limits = SubtitleLimits::new(SubtitleLimits::MAX_INPUT_BYTES, 16, 256)
            .expect("limits are valid");
        let report = parse_ass(SourceId::new(0), source.as_bytes(), limits);
        assert_eq!(report.errors().len(), SubtitleLimits::MAX_ERRORS);
        assert_eq!(
            report
                .errors()
                .last()
                .expect("the limit is reported")
                .kind(),
            SubtitleErrorKind::TooManyErrors,
        );
    }

    fn limits() -> SubtitleLimits {
        SubtitleLimits::new(4_096, 16, 256).expect("fixture limits are valid")
    }
}
