//! Golden normalization of standalone subtitle formats into core caption facts.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use onmark_core::model::{CaptionTrack, SourceId};
use onmark_media::{
    SubtitleError, SubtitleErrorKind, SubtitleLimits, SubtitleReport, parse_ass, parse_subrip,
    parse_webvtt,
};

#[test]
fn normalizes_subrip_into_exact_caption_facts() {
    let source = read("valid/basic.srt");
    let report = parse_subrip(SourceId::new(0), source.as_bytes(), fixture_limits());
    assert!(report.errors().is_empty());

    let actual = render_track(report.track().expect("the fixture is valid"));
    assert_eq!(actual, read("valid/basic.captions.txt"));
}

#[test]
fn reports_independent_subrip_errors_in_source_order() {
    let source = read("invalid/cue-errors.srt");
    let report = parse_subrip(SourceId::new(0), source.as_bytes(), fixture_limits());
    assert!(report.track().is_none());

    let actual = render_errors(&report);
    assert_eq!(actual, read("invalid/cue-errors.errors.txt"));
}

#[test]
fn normalizes_plain_webvtt_into_exact_caption_facts() {
    let source = read("valid/basic.vtt");
    let report = parse_webvtt(SourceId::new(0), source.as_bytes(), fixture_limits());
    assert!(report.errors().is_empty());

    let actual = render_track(report.track().expect("the fixture is valid"));
    assert_eq!(actual, read("valid/basic-vtt.captions.txt"));
}

#[test]
fn rejects_webvtt_semantics_that_cannot_be_normalized_losslessly() {
    let source = read("invalid/webvtt-errors.vtt");
    let report = parse_webvtt(SourceId::new(0), source.as_bytes(), fixture_limits());
    assert!(report.track().is_none());

    let actual = render_errors(&report);
    assert_eq!(actual, read("invalid/webvtt-errors.errors.txt"));
}

#[test]
fn normalizes_plain_ass_events_into_exact_caption_facts() {
    let source = read("valid/basic.ass");
    let report = parse_ass(SourceId::new(0), source.as_bytes(), fixture_limits());
    assert!(report.errors().is_empty());

    let actual = render_track(report.track().expect("the fixture is valid"));
    assert_eq!(actual, read("valid/basic-ass.captions.txt"));
}

#[test]
fn rejects_ass_presentation_semantics_instead_of_discarding_them() {
    let source = read("invalid/ass-errors.ass");
    let report = parse_ass(SourceId::new(0), source.as_bytes(), fixture_limits());
    assert!(report.track().is_none());

    let actual = render_errors(&report);
    assert_eq!(actual, read("invalid/ass-errors.errors.txt"));
}

fn render_track(track: &CaptionTrack) -> String {
    let mut output = String::from("caption-track\n");
    for cue in track.cues() {
        let interval = cue.interval();
        writeln!(
            output,
            "  cue {}..{}",
            interval.start().as_nanos(),
            interval.end().as_nanos(),
        )
        .expect("writing into a String cannot fail");
        writeln!(
            output,
            "    timing {}..{}",
            cue.timing_span().start().get(),
            cue.timing_span().end().get(),
        )
        .expect("writing into a String cannot fail");
        writeln!(
            output,
            "    text {}..{} \"{}\"",
            cue.text_span().start().get(),
            cue.text_span().end().get(),
            escape_text(cue.text()),
        )
        .expect("writing into a String cannot fail");
    }
    output
}

fn render_errors(report: &SubtitleReport) -> String {
    let mut output = String::new();
    for error in report.errors() {
        writeln!(
            output,
            "{} {}..{}",
            error_name(error),
            error.span().start().get(),
            error.span().end().get(),
        )
        .expect("writing into a String cannot fail");
    }
    output
}

fn error_name(error: &SubtitleError) -> &'static str {
    match error.kind() {
        SubtitleErrorKind::InputTooLarge => "input-too-large",
        SubtitleErrorKind::InvalidUtf8 => "invalid-utf8",
        SubtitleErrorKind::EmptyTrack => "empty-track",
        SubtitleErrorKind::InvalidSubRipIndex => "invalid-subrip-index",
        SubtitleErrorKind::MissingTiming => "missing-timing",
        SubtitleErrorKind::InvalidSubRipTiming => "invalid-subrip-timing",
        SubtitleErrorKind::NonPositiveInterval => "non-positive-interval",
        SubtitleErrorKind::MissingText => "missing-text",
        SubtitleErrorKind::TooManyCues => "too-many-cues",
        SubtitleErrorKind::CueTextTooLarge => "cue-text-too-large",
        SubtitleErrorKind::TooManyErrors => "too-many-errors",
        SubtitleErrorKind::InvalidWebVttHeader => "invalid-webvtt-header",
        SubtitleErrorKind::InvalidWebVttTiming => "invalid-webvtt-timing",
        SubtitleErrorKind::OutOfOrderCue => "out-of-order-cue",
        SubtitleErrorKind::UnsupportedWebVttBlock => "unsupported-webvtt-block",
        SubtitleErrorKind::UnsupportedWebVttCueSettings => "unsupported-webvtt-cue-settings",
        SubtitleErrorKind::UnsupportedWebVttCueMarkup => "unsupported-webvtt-cue-markup",
        SubtitleErrorKind::InvalidAssHeader => "invalid-ass-header",
        SubtitleErrorKind::MissingAssEvents => "missing-ass-events",
        SubtitleErrorKind::InvalidAssFormat => "invalid-ass-format",
        SubtitleErrorKind::InvalidAssDialogue => "invalid-ass-dialogue",
        SubtitleErrorKind::InvalidAssTiming => "invalid-ass-timing",
        SubtitleErrorKind::UnsupportedAssSection => "unsupported-ass-section",
        SubtitleErrorKind::UnsupportedAssEventFields => "unsupported-ass-event-fields",
        SubtitleErrorKind::UnsupportedAssText => "unsupported-ass-text",
    }
}

fn escape_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '"' => output.push_str("\\\""),
            character => output.push(character),
        }
    }
    output
}

fn read(relative: &str) -> String {
    fs::read_to_string(fixture(relative)).expect("the checked-in subtitle fixture is readable")
}

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/subtitle")
        .join(relative)
}

fn fixture_limits() -> SubtitleLimits {
    SubtitleLimits::new(4_096, 16, 256).expect("fixture limits are valid")
}
