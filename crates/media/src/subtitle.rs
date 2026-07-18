//! Bounded normalization of authored standalone subtitle files.
//!
//! Parsers own external format syntax and byte offsets. They emit only exact
//! core caption facts and format-local errors; filesystem access, diagnostics,
//! browser layout, and screenplay spelling belong to later boundaries.

use std::error::Error;
use std::fmt;
use std::iter::Peekable;
use std::str;

use onmark_core::model::{ByteOffset, CaptionCue, CaptionTrack, Duration, SourceId, SourceSpan};

mod ass;
mod subrip;
mod webvtt;

pub use ass::parse_ass;
pub use subrip::parse_subrip;
pub use webvtt::parse_webvtt;

const UTF8_BOM: &str = "\u{feff}";
const NANOS_PER_MILLISECOND: u64 = 1_000_000;
const MILLIS_PER_SECOND: u64 = 1_000;
const SECONDS_PER_MINUTE: u64 = 60;
const MINUTES_PER_HOUR: u64 = 60;

/// Resource bounds applied before and during subtitle parsing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubtitleLimits {
    input_bytes: usize,
    cues: usize,
    cue_text_bytes: usize,
}

impl SubtitleLimits {
    /// Largest subtitle file admitted by this parser boundary.
    pub const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;
    /// Largest number of normalized cues admitted from one file.
    pub const MAX_CUES: usize = 100_000;
    /// Largest retained UTF-8 payload admitted for one cue.
    pub const MAX_CUE_TEXT_BYTES: usize = 256 * 1024;
    /// Largest number of format errors retained from one source.
    pub const MAX_ERRORS: usize = 4_096;

    /// Creates non-zero limits within the parser's fixed safety ceilings.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidSubtitleLimits`] when a limit is zero or exceeds its
    /// fixed ceiling.
    pub const fn new(
        max_input_bytes: usize,
        max_cues: usize,
        max_cue_text_bytes: usize,
    ) -> Result<Self, InvalidSubtitleLimits> {
        if max_input_bytes == 0 {
            return Err(InvalidSubtitleLimits::ZeroInputBytes);
        }
        if max_input_bytes > Self::MAX_INPUT_BYTES {
            return Err(InvalidSubtitleLimits::InputBytesTooLarge);
        }
        if max_cues == 0 {
            return Err(InvalidSubtitleLimits::ZeroCues);
        }
        if max_cues > Self::MAX_CUES {
            return Err(InvalidSubtitleLimits::CueLimitTooLarge);
        }
        if max_cue_text_bytes == 0 {
            return Err(InvalidSubtitleLimits::ZeroCueTextBytes);
        }
        if max_cue_text_bytes > Self::MAX_CUE_TEXT_BYTES {
            return Err(InvalidSubtitleLimits::CueTextBytesTooLarge);
        }

        Ok(Self {
            input_bytes: max_input_bytes,
            cues: max_cues,
            cue_text_bytes: max_cue_text_bytes,
        })
    }

    /// Returns the maximum admitted input byte length.
    #[must_use]
    pub const fn max_input_bytes(self) -> usize {
        self.input_bytes
    }

    /// Returns the maximum admitted cue count.
    #[must_use]
    pub const fn max_cues(self) -> usize {
        self.cues
    }

    /// Returns the maximum retained UTF-8 bytes in one cue payload.
    #[must_use]
    pub const fn max_cue_text_bytes(self) -> usize {
        self.cue_text_bytes
    }

    /// Returns the fixed retained format-error ceiling.
    #[must_use]
    pub const fn max_errors(self) -> usize {
        Self::MAX_ERRORS
    }
}

/// Reason subtitle parsing cannot be bounded by the requested limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidSubtitleLimits {
    /// Zero bytes cannot carry valid input.
    ZeroInputBytes,
    /// The requested input limit exceeds the fixed safety ceiling.
    InputBytesTooLarge,
    /// Zero cues cannot carry a valid track.
    ZeroCues,
    /// The requested cue count exceeds the fixed safety ceiling.
    CueLimitTooLarge,
    /// Zero bytes cannot carry valid cue text.
    ZeroCueTextBytes,
    /// The requested cue-text limit exceeds the fixed safety ceiling.
    CueTextBytesTooLarge,
}

impl fmt::Display for InvalidSubtitleLimits {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::ZeroInputBytes => "subtitle input limit cannot be zero",
            Self::InputBytesTooLarge => "subtitle input limit exceeds sixteen MiB",
            Self::ZeroCues => "subtitle cue limit cannot be zero",
            Self::CueLimitTooLarge => "subtitle cue limit exceeds 100,000",
            Self::ZeroCueTextBytes => "subtitle cue-text limit cannot be zero",
            Self::CueTextBytesTooLarge => "subtitle cue-text limit exceeds 256 KiB",
        };
        formatter.write_str(message)
    }
}

impl Error for InvalidSubtitleLimits {}

/// Result of parsing one authored subtitle source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubtitleReport {
    track: Option<CaptionTrack>,
    errors: Vec<SubtitleError>,
}

impl SubtitleReport {
    /// Returns the normalized track when the complete source is valid.
    #[must_use]
    pub const fn track(&self) -> Option<&CaptionTrack> {
        self.track.as_ref()
    }

    /// Returns format errors in stable source order.
    #[must_use]
    pub fn errors(&self) -> &[SubtitleError] {
        &self.errors
    }

    /// Separates the candidate track from its format errors.
    #[must_use]
    pub fn into_parts(self) -> (Option<CaptionTrack>, Vec<SubtitleError>) {
        (self.track, self.errors)
    }

    fn failed(error: SubtitleError) -> Self {
        Self {
            track: None,
            errors: vec![error],
        }
    }
}

/// One source-located failure in an authored subtitle file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubtitleError {
    kind: SubtitleErrorKind,
    span: SourceSpan,
}

impl SubtitleError {
    /// Returns the closed format-error reason.
    #[must_use]
    pub const fn kind(&self) -> SubtitleErrorKind {
        self.kind
    }

    /// Returns the offending UTF-8 byte range.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }

    fn new(kind: SubtitleErrorKind, span: SourceSpan) -> Self {
        Self { kind, span }
    }
}

impl fmt::Display for SubtitleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(formatter)
    }
}

impl Error for SubtitleError {}

/// Reason an authored subtitle source cannot become exact caption facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubtitleErrorKind {
    /// The byte input exceeds the configured retained-input limit.
    InputTooLarge,
    /// The source is not valid UTF-8.
    InvalidUtf8,
    /// No caption cue appears in the source.
    EmptyTrack,
    /// A `SubRip` cue does not begin with a decimal index.
    InvalidSubRipIndex,
    /// A subtitle cue identifier has no following timing line.
    MissingTiming,
    /// A timing line does not use exact `hours:minutes:seconds,milliseconds` values.
    InvalidSubRipTiming,
    /// A cue end is not strictly after its start.
    NonPositiveInterval,
    /// A cue has no non-whitespace payload.
    MissingText,
    /// The number of authored cues exceeds the configured limit.
    TooManyCues,
    /// One retained cue payload exceeds the configured byte limit.
    CueTextTooLarge,
    /// Further independent format errors were suppressed at the fixed ceiling.
    TooManyErrors,
    /// A `WebVTT` source does not begin with a complete `WEBVTT` header block.
    InvalidWebVttHeader,
    /// A `WebVTT` cue timing line is malformed.
    InvalidWebVttTiming,
    /// A cue begins before its predecessor in authored order.
    OutOfOrderCue,
    /// A `WebVTT` block carries presentation semantics not yet normalized by Onmark.
    UnsupportedWebVttBlock,
    /// A `WebVTT` cue uses position, alignment, region, or writing-mode settings.
    UnsupportedWebVttCueSettings,
    /// A `WebVTT` cue payload uses markup or escapes not represented by caption facts.
    UnsupportedWebVttCueMarkup,
    /// An ASS source does not declare a supported script header.
    InvalidAssHeader,
    /// An ASS source contains no events section.
    MissingAssEvents,
    /// An ASS events format omits or reorders required fields.
    InvalidAssFormat,
    /// An ASS dialogue record does not match its declared events format.
    InvalidAssDialogue,
    /// An ASS dialogue timestamp is malformed.
    InvalidAssTiming,
    /// An ASS section or script property carries unsupported presentation semantics.
    UnsupportedAssSection,
    /// An ASS events format carries fields outside the plain-event subset.
    UnsupportedAssEventFields,
    /// ASS dialogue text uses override or escape semantics that cannot be preserved.
    UnsupportedAssText,
}

impl fmt::Display for SubtitleErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InputTooLarge => "subtitle input exceeds its configured byte limit",
            Self::InvalidUtf8 => "subtitle input must be valid UTF-8",
            Self::EmptyTrack => "subtitle file must contain at least one cue",
            Self::InvalidSubRipIndex => "SubRip cue must begin with a decimal index",
            Self::MissingTiming => "subtitle cue identifier must be followed by a timing line",
            Self::InvalidSubRipTiming => "SubRip timing must use H+:MM:SS,mmm --> H+:MM:SS,mmm",
            Self::NonPositiveInterval => "subtitle cue end must be after its start",
            Self::MissingText => "subtitle cue must contain non-whitespace text",
            Self::TooManyCues => "subtitle cue count exceeds its configured limit",
            Self::CueTextTooLarge => "subtitle cue text exceeds its configured byte limit",
            Self::TooManyErrors => "subtitle format errors exceed the retained diagnostic limit",
            Self::InvalidWebVttHeader => "WebVTT input must begin with a complete WEBVTT header",
            Self::InvalidWebVttTiming => "WebVTT cue timing is malformed",
            Self::OutOfOrderCue => "caption cue starts must be nondecreasing in authored order",
            Self::UnsupportedWebVttBlock => "WebVTT STYLE and REGION blocks are not supported yet",
            Self::UnsupportedWebVttCueSettings => {
                "WebVTT cue presentation settings are not supported yet"
            }
            Self::UnsupportedWebVttCueMarkup => {
                "WebVTT cue markup and escapes are not supported yet"
            }
            Self::InvalidAssHeader => "ASS input must declare ScriptType v4.00+",
            Self::MissingAssEvents => "ASS input must contain an Events section",
            Self::InvalidAssFormat => "ASS events must declare Start, End, and Text in order",
            Self::InvalidAssDialogue => "ASS dialogue does not match its events format",
            Self::InvalidAssTiming => "ASS timing must use H+:MM:SS.cc",
            Self::UnsupportedAssSection => {
                "ASS style, resolution, attachment, and other presentation sections are not supported yet"
            }
            Self::UnsupportedAssEventFields => {
                "ASS event presentation fields are not supported yet"
            }
            Self::UnsupportedAssText => {
                "ASS override tags, drawings, and non-text escapes are not supported yet"
            }
        };
        formatter.write_str(message)
    }
}

#[derive(Clone, Copy)]
struct SubtitleSource<'a> {
    id: SourceId,
    text: &'a str,
    base: usize,
}

impl<'a> SubtitleSource<'a> {
    fn decode(
        id: SourceId,
        bytes: &'a [u8],
        limits: SubtitleLimits,
    ) -> Result<Self, SubtitleReport> {
        if bytes.len() > limits.max_input_bytes() {
            return Err(SubtitleReport::failed(SubtitleError::new(
                SubtitleErrorKind::InputTooLarge,
                span(id, 0, bytes.len()),
            )));
        }

        let text = str::from_utf8(bytes).map_err(|error| {
            let start = error.valid_up_to();
            let end = error
                .error_len()
                .map_or(bytes.len(), |length| start + length);
            SubtitleReport::failed(SubtitleError::new(
                SubtitleErrorKind::InvalidUtf8,
                span(id, start, end),
            ))
        })?;
        let (text, base) = text
            .strip_prefix(UTF8_BOM)
            .map_or((text, 0), |text| (text, UTF8_BOM.len()));
        Ok(Self { id, text, base })
    }

    fn end(self) -> usize {
        self.base + self.text.len()
    }

    fn lines(self) -> Peekable<SourceLines<'a>> {
        SourceLines::new(self.id, self.text, self.base).peekable()
    }
}

struct CuePayload {
    text: Option<String>,
    span: SourceSpan,
    too_large: bool,
}

impl CuePayload {
    fn into_text(self) -> Result<String, SubtitleErrorKind> {
        if self.too_large {
            return Err(SubtitleErrorKind::CueTextTooLarge);
        }
        self.text.ok_or(SubtitleErrorKind::MissingText)
    }
}

fn finish_report(
    source: SourceId,
    source_end: usize,
    cues: Vec<CaptionCue>,
    mut errors: Vec<SubtitleError>,
) -> SubtitleReport {
    if errors.is_empty() && cues.is_empty() {
        errors.push(SubtitleError::new(
            SubtitleErrorKind::EmptyTrack,
            span(source, source_end, source_end),
        ));
    }
    if !errors.is_empty() {
        return SubtitleReport {
            track: None,
            errors,
        };
    }

    let track = CaptionTrack::new(cues)
        .expect("the empty-track case is rejected before constructing caption facts");
    SubtitleReport {
        track: Some(track),
        errors: Vec::new(),
    }
}

fn push_cue(
    cues: &mut Vec<CaptionCue>,
    errors: &mut Vec<SubtitleError>,
    error_limit: usize,
    cue: CaptionCue,
) {
    let out_of_order = cues
        .last()
        .is_some_and(|previous| cue.interval().start() < previous.interval().start());
    if out_of_order {
        push_error(
            errors,
            error_limit,
            SubtitleErrorKind::OutOfOrderCue,
            cue.timing_span(),
        );
        return;
    }
    cues.push(cue);
}

fn push_error(
    errors: &mut Vec<SubtitleError>,
    limit: usize,
    kind: SubtitleErrorKind,
    span: SourceSpan,
) {
    if errors.len() < limit {
        errors.push(SubtitleError::new(kind, span));
        return;
    }
    mark_errors_suppressed(errors);
}

fn insert_error(
    errors: &mut Vec<SubtitleError>,
    limit: usize,
    kind: SubtitleErrorKind,
    span: SourceSpan,
) {
    if errors.len() == limit {
        mark_errors_suppressed(errors);
        return;
    }
    let error = SubtitleError::new(kind, span);
    let index = errors.partition_point(|error| error.span() <= span);
    errors.insert(index, error);
}

fn mark_errors_suppressed(errors: &mut [SubtitleError]) {
    let Some(last) = errors.last_mut() else {
        return;
    };
    last.kind = SubtitleErrorKind::TooManyErrors;
}

fn read_payload(
    lines: &mut Peekable<SourceLines<'_>>,
    timing_span: SourceSpan,
    max_bytes: usize,
    separator: BlockSeparator,
) -> CuePayload {
    let mut text = String::new();
    let mut text_span = None;
    let mut too_large = false;

    while lines.peek().is_some_and(|line| !separator.matches(*line)) {
        let line = lines
            .next()
            .expect("the peeked non-blank line remains available");
        text_span = Some(match text_span {
            Some(span) => join_spans(span, line.span),
            None => line.span,
        });

        let line_break_bytes = usize::from(!text.is_empty());
        let retained_bytes = text
            .len()
            .checked_add(line_break_bytes)
            .and_then(|length| length.checked_add(line.text.len()));
        if retained_bytes.is_none_or(|retained| retained > max_bytes) {
            too_large = true;
            continue;
        }
        if line_break_bytes == 1 {
            text.push('\n');
        }
        text.push_str(line.text);
    }

    consume_separator(lines, separator);

    let span = text_span.unwrap_or_else(|| insertion_span(timing_span));
    let text = (!text.trim().is_empty()).then_some(text);
    CuePayload {
        text,
        span,
        too_large,
    }
}

fn consume_separator(lines: &mut Peekable<SourceLines<'_>>, separator: BlockSeparator) {
    if lines.peek().is_some_and(|line| separator.matches(*line)) {
        lines.next();
    }
}

#[derive(Clone, Copy)]
struct SourceLine<'a> {
    text: &'a str,
    span: SourceSpan,
}

impl SourceLine<'_> {
    fn is_blank(self) -> bool {
        self.text.trim().is_empty()
    }

    fn is_empty(self) -> bool {
        self.text.is_empty()
    }

    fn subspan(self, start: usize, end: usize) -> SourceSpan {
        self.text
            .get(start..end)
            .expect("line-relative subtitle ranges stay inside the source line");
        let start = u64::try_from(start).expect("Rust byte offsets fit in u64");
        let end = u64::try_from(end).expect("Rust byte offsets fit in u64");
        let start = self
            .span
            .start()
            .get()
            .checked_add(start)
            .expect("line-relative subtitle offsets remain in the source domain");
        let end = self
            .span
            .start()
            .get()
            .checked_add(end)
            .expect("line-relative subtitle offsets remain in the source domain");
        SourceSpan::new(
            self.span.source(),
            ByteOffset::new(start),
            ByteOffset::new(end),
        )
        .expect("line-relative subtitle ranges are ordered")
    }
}

#[derive(Clone, Copy)]
enum BlockSeparator {
    BlankLine,
    EmptyLine,
}

impl BlockSeparator {
    fn matches(self, line: SourceLine<'_>) -> bool {
        match self {
            Self::BlankLine => line.is_blank(),
            Self::EmptyLine => line.is_empty(),
        }
    }
}

struct SourceLines<'a> {
    source: SourceId,
    text: &'a str,
    base: usize,
    cursor: usize,
}

impl<'a> SourceLines<'a> {
    fn new(source: SourceId, text: &'a str, base: usize) -> Self {
        Self {
            source,
            text,
            base,
            cursor: 0,
        }
    }
}

impl<'a> Iterator for SourceLines<'a> {
    type Item = SourceLine<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor == self.text.len() {
            return None;
        }

        let start = self.cursor;
        let remainder = &self.text[start..];
        let newline = remainder.find('\n');
        let raw_end = newline.map_or(self.text.len(), |offset| start + offset);
        let end = line_content_end(self.text, start, raw_end);
        self.cursor = newline.map_or(self.text.len(), |_| raw_end + 1);

        Some(SourceLine {
            text: &self.text[start..end],
            span: span(self.source, self.base + start, self.base + end),
        })
    }
}

fn line_content_end(text: &str, start: usize, raw_end: usize) -> usize {
    if raw_end > start && text.as_bytes()[raw_end - 1] == b'\r' {
        return raw_end - 1;
    }
    raw_end
}

fn is_decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn duration_from_clock(
    hours: u64,
    minutes: u64,
    seconds: u64,
    milliseconds: u64,
) -> Option<Duration> {
    if minutes >= MINUTES_PER_HOUR || seconds >= SECONDS_PER_MINUTE {
        return None;
    }

    let seconds = hours
        .checked_mul(MINUTES_PER_HOUR)?
        .checked_add(minutes)?
        .checked_mul(SECONDS_PER_MINUTE)?
        .checked_add(seconds)?;
    let milliseconds = seconds
        .checked_mul(MILLIS_PER_SECOND)?
        .checked_add(milliseconds)?;
    let nanoseconds = milliseconds.checked_mul(NANOS_PER_MILLISECOND)?;
    Some(Duration::from_nanos(nanoseconds))
}

fn join_spans(start: SourceSpan, end: SourceSpan) -> SourceSpan {
    SourceSpan::new(start.source(), start.start(), end.end())
        .expect("source lines are ordered within one subtitle source")
}

fn insertion_span(after: SourceSpan) -> SourceSpan {
    SourceSpan::new(after.source(), after.end(), after.end())
        .expect("equal source bounds form an insertion span")
}

fn span(source: SourceId, start: usize, end: usize) -> SourceSpan {
    let start = ByteOffset::new(u64::try_from(start).expect("Rust byte offsets fit in u64"));
    let end = ByteOffset::new(u64::try_from(end).expect("Rust byte offsets fit in u64"));
    SourceSpan::new(source, start, end).expect("subtitle byte ranges are ordered")
}

#[cfg(test)]
mod tests {
    use onmark_core::model::SourceId;

    use super::{SubtitleErrorKind, push_error, span};

    #[test]
    fn reports_suppression_only_after_the_retained_error_limit_is_crossed() {
        let mut errors = Vec::new();
        push_error(
            &mut errors,
            2,
            SubtitleErrorKind::MissingTiming,
            span(SourceId::new(0), 0, 1),
        );
        push_error(
            &mut errors,
            2,
            SubtitleErrorKind::MissingText,
            span(SourceId::new(0), 1, 2),
        );

        assert_eq!(errors[1].kind(), SubtitleErrorKind::MissingText);

        push_error(
            &mut errors,
            2,
            SubtitleErrorKind::InvalidSubRipTiming,
            span(SourceId::new(0), 2, 3),
        );

        assert_eq!(errors.len(), 2);
        assert_eq!(errors[1].kind(), SubtitleErrorKind::TooManyErrors);
    }
}
