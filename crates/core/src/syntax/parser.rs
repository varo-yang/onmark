//! Span-preserving HTML token adaptation and bounded syntax recovery.
//!
//! `html5gum` owns WHATWG tokenization and raw-text state switches. Onmark owns
//! the strict element stack: browser error recovery must never silently change
//! screenplay structure before semantic binding.

use html5gum::emitters::callback::{Callback, CallbackEmitter, CallbackEvent};
use html5gum::{Error as HtmlError, Span, Tokenizer};

use crate::model::{ByteOffset, SourceId, SourceSpan};

use super::builder::TreeBuilder;
use super::{
    Attribute, AttributeName, ElementName, MAX_SCREENPLAY_BYTES, Node, SourceDocument, SyntaxError,
    SyntaxErrorKind, SyntaxResource, TextNode,
};

const MAX_RETAINED_ITEMS: usize = 65_536;
const MAX_NESTING_DEPTH: usize = 32;

/// Internal syntax output consumed by the compiler facade.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SyntaxReport {
    document: SourceDocument,
    errors: Vec<SyntaxError>,
}

impl SyntaxReport {
    pub(crate) fn into_parts(self) -> (SourceDocument, Vec<SyntaxError>) {
        (self.document, self.errors)
    }
}

#[must_use]
pub(crate) fn parse(source: SourceId, text: &str) -> SyntaxReport {
    Parser::new(source, text, SyntaxLimits::DEFAULT).parse()
}

#[derive(Clone, Copy)]
struct SyntaxLimits {
    source_bytes: usize,
    retained_items: usize,
    nesting_depth: usize,
}

impl SyntaxLimits {
    const DEFAULT: Self = Self {
        source_bytes: MAX_SCREENPLAY_BYTES,
        retained_items: MAX_RETAINED_ITEMS,
        nesting_depth: MAX_NESTING_DEPTH,
    };
}

/// Single owner of the recovered tree and syntax errors for one source buffer.
struct Parser<'a> {
    source: SourceText<'a>,
    tree: TreeBuilder,
    errors: Vec<SyntaxError>,
    limits: SyntaxLimits,
    retained_items: usize,
}

impl<'a> Parser<'a> {
    fn new(source: SourceId, text: &'a str, limits: SyntaxLimits) -> Self {
        Self {
            source: SourceText::new(source, text),
            tree: TreeBuilder::new(),
            errors: Vec::new(),
            limits,
            retained_items: 0,
        }
    }

    fn parse(mut self) -> SyntaxReport {
        if self.source.text.len() > self.limits.source_bytes {
            let offset = source_limit_offset(self.source.text, self.limits.source_bytes);
            self.reject_resource(
                SyntaxResource::SourceBytes,
                self.source.range(offset, offset),
            );
            return self.report();
        }

        let completed = self.consume_tokens();
        self.finish_open_elements(completed);
        self.report()
    }

    fn consume_tokens(&mut self) -> bool {
        let mut emitter = CallbackEmitter::new(HtmlEmitter::default());
        emitter.naively_switch_states(true);

        for token in Tokenizer::new_with_emitter(self.source.text, emitter) {
            let token = token.expect("tokenizing an in-memory UTF-8 string is infallible");
            if !self.consume(token) {
                return false;
            }
        }
        true
    }

    fn consume(&mut self, token: HtmlToken) -> bool {
        match token {
            HtmlToken::StartTag(tag) => self.start_element(&tag),
            HtmlToken::EndTag(tag) => {
                self.close_element(&tag);
                true
            }
            HtmlToken::Text(text) => self.add_text(text),
            HtmlToken::Doctype(doctype) => {
                self.consume_doctype(&doctype);
                true
            }
            HtmlToken::Error(error) => {
                self.reject_html_error(&error);
                true
            }
        }
    }

    fn start_element(&mut self, tag: &StartTag) -> bool {
        if self.tree.open_depth() >= self.limits.nesting_depth {
            self.reject_resource(
                SyntaxResource::NestingDepth,
                self.source.span(tag.name_span),
            );
            return false;
        }
        if !self.retain_item(tag.span) {
            return false;
        }

        let name = self.element_name(&tag.name, tag.name_span);
        self.tree.start_element(name, byte_offset(tag.span.start));

        for (index, attribute) in tag.attributes.iter().enumerate() {
            if !self.add_attribute(attribute, tag.attribute_end(index)) {
                return false;
            }
        }

        if is_void_element(&tag.name) {
            self.tree
                .finish_empty_element(self.source.id, byte_offset(tag.span.end));
            return true;
        }

        if tag.self_closing {
            self.errors.push(SyntaxError::new(
                SyntaxErrorKind::SelfClosingNonVoid {
                    name: tag.name.clone(),
                },
                self.source.span(tag.span),
            ));
        }
        // Browsers ignore a trailing solidus on non-void elements. Keep the
        // element open as well, so recovered ownership cannot disagree with DOM.
        self.tree.open_element();
        true
    }

    fn add_attribute(&mut self, token: &AttributeToken, authored_end: usize) -> bool {
        let authored = self.source.attribute(token, authored_end);
        if !self.retain_item(authored.span) {
            return false;
        }

        let name = AttributeName::new(token.name.clone(), self.source.span(token.name_span));
        let attribute = Attribute::new(
            name,
            token.value.clone(),
            self.source.span(authored.span),
            self.source.span(authored.value_span),
        );
        if let Some(error) = self.tree.add_attribute(attribute) {
            self.errors.push(error);
        }
        true
    }

    fn close_element(&mut self, tag: &EndTag) {
        let found = self.element_name(&tag.name, tag.name_span);
        let error = self
            .tree
            .close_element(self.source.id, &found, byte_offset(tag.span.end));
        if let Some(error) = error {
            self.errors.push(error);
        }
    }

    fn add_text(&mut self, text: TextToken) -> bool {
        if text.value.is_empty() {
            return true;
        }
        if !self.retain_item(text.span) {
            return false;
        }
        let node = TextNode::new(text.value, self.source.span(text.span));
        self.tree.append(Node::Text(node));
        true
    }

    fn consume_doctype(&mut self, doctype: &DoctypeToken) {
        if doctype.name.as_deref() == Some("html")
            && doctype.public_identifier.is_none()
            && doctype.system_identifier.is_none()
            && !doctype.force_quirks
        {
            return;
        }
        self.errors.push(SyntaxError::new(
            SyntaxErrorKind::UnsupportedDocumentType,
            self.source.span(doctype.span),
        ));
    }

    fn reject_html_error(&mut self, error: &HtmlParseError) {
        if error.kind == HtmlError::DuplicateAttribute {
            return;
        }

        let (kind, span) = if is_character_reference_error(error.kind) {
            let reference = self.source.character_reference(error.span);
            (
                SyntaxErrorKind::InvalidCharacterReference {
                    reference: Box::from(self.source.slice(reference)),
                },
                reference,
            )
        } else {
            (SyntaxErrorKind::MalformedMarkup, error.span)
        };
        self.errors
            .push(SyntaxError::new(kind, self.source.span(span)));
    }

    fn retain_item(&mut self, span: HtmlSpan) -> bool {
        if self.retained_items == self.limits.retained_items {
            self.reject_resource(SyntaxResource::Items, self.source.span(span));
            return false;
        }
        self.retained_items += 1;
        true
    }

    fn reject_resource(&mut self, resource: SyntaxResource, span: SourceSpan) {
        self.errors.push(SyntaxError::new(
            SyntaxErrorKind::ResourceLimit { resource },
            span,
        ));
    }

    fn finish_open_elements(&mut self, completed: bool) {
        if completed {
            let offset = self.source.end();
            let ended_at = self.source.range(offset, offset);
            self.errors.extend(self.tree.unclosed_elements(ended_at));
        }
        self.tree.finish(self.source.id, self.source.end());
    }

    fn element_name(&self, name: &str, span: HtmlSpan) -> ElementName {
        ElementName::new(Box::from(name), self.source.span(span))
    }

    fn report(self) -> SyntaxReport {
        let source_span = self.source.range(ByteOffset::ZERO, self.source.end());
        SyntaxReport {
            document: self.tree.into_document(source_span),
            errors: self.errors,
        }
    }
}

// The callback emitter preserves authored order and separate name/value spans.
// The default html5gum token map sorts attributes and therefore cannot satisfy
// Onmark's source-location contract.
#[derive(Default)]
struct HtmlEmitter {
    start: Option<PendingStartTag>,
    attribute: Option<PendingAttribute>,
}

impl Callback<HtmlToken, usize> for HtmlEmitter {
    fn handle_event(&mut self, event: CallbackEvent<'_>, span: Span<usize>) -> Option<HtmlToken> {
        match event {
            CallbackEvent::OpenStartTag { name } => {
                self.start = Some(PendingStartTag::new(name, span.into()));
                None
            }
            CallbackEvent::AttributeName { name } => {
                self.finish_attribute();
                if self.start.is_some() {
                    self.attribute = Some(PendingAttribute::new(name, span.into()));
                }
                None
            }
            CallbackEvent::AttributeValue { value } => {
                if let Some(attribute) = &mut self.attribute {
                    attribute.value = decoded(value);
                    attribute.value_span = Some(span.into());
                }
                None
            }
            CallbackEvent::CloseStartTag { self_closing } => {
                self.finish_attribute();
                self.start
                    .take()
                    .map(|tag| HtmlToken::StartTag(tag.finish(self_closing, span.into())))
            }
            CallbackEvent::EndTag { name } => {
                Some(HtmlToken::EndTag(EndTag::new(name, span.into())))
            }
            CallbackEvent::String { value } => Some(HtmlToken::Text(TextToken {
                value: decoded(value),
                span: span.into(),
            })),
            CallbackEvent::Comment { .. } => None,
            CallbackEvent::Doctype {
                name,
                public_identifier,
                system_identifier,
                force_quirks,
            } => Some(HtmlToken::Doctype(DoctypeToken {
                name: optional_decoded(name),
                public_identifier: public_identifier.map(decoded),
                system_identifier: system_identifier.map(decoded),
                force_quirks,
                span: span.into(),
            })),
            CallbackEvent::Error(kind) => Some(HtmlToken::Error(HtmlParseError {
                kind,
                span: span.into(),
            })),
        }
    }
}

impl HtmlEmitter {
    fn finish_attribute(&mut self) {
        let Some(attribute) = self.attribute.take() else {
            return;
        };
        if let Some(start) = &mut self.start {
            start.attributes.push(attribute.finish());
        }
    }
}

enum HtmlToken {
    StartTag(StartTag),
    EndTag(EndTag),
    Text(TextToken),
    Doctype(DoctypeToken),
    Error(HtmlParseError),
}

struct PendingStartTag {
    name: Box<str>,
    name_span: HtmlSpan,
    start: usize,
    attributes: Vec<AttributeToken>,
}

impl PendingStartTag {
    fn new(name: &[u8], span: HtmlSpan) -> Self {
        let name = decoded(name);
        let name_span = HtmlSpan {
            start: span.start + 1,
            end: span.start + 1 + name.len(),
        };
        Self {
            name,
            name_span,
            start: span.start,
            attributes: Vec::new(),
        }
    }

    fn finish(self, self_closing: bool, close: HtmlSpan) -> StartTag {
        StartTag {
            name: self.name,
            name_span: self.name_span,
            span: HtmlSpan {
                start: self.start,
                end: close.end,
            },
            close_start: close.start,
            attributes: self.attributes,
            self_closing,
        }
    }
}

struct PendingAttribute {
    name: Box<str>,
    name_span: HtmlSpan,
    value: Box<str>,
    value_span: Option<HtmlSpan>,
}

impl PendingAttribute {
    fn new(name: &[u8], name_span: HtmlSpan) -> Self {
        Self {
            name: decoded(name),
            name_span,
            value: Box::from(""),
            value_span: None,
        }
    }

    fn finish(self) -> AttributeToken {
        AttributeToken {
            name: self.name,
            name_span: self.name_span,
            value: self.value,
            tokenizer_value_span: self.value_span,
        }
    }
}

struct StartTag {
    name: Box<str>,
    name_span: HtmlSpan,
    span: HtmlSpan,
    close_start: usize,
    attributes: Vec<AttributeToken>,
    self_closing: bool,
}

impl StartTag {
    fn attribute_end(&self, index: usize) -> usize {
        self.attributes
            .get(index + 1)
            .map_or(self.close_start, |attribute| attribute.name_span.start)
    }
}

struct AttributeToken {
    name: Box<str>,
    name_span: HtmlSpan,
    value: Box<str>,
    tokenizer_value_span: Option<HtmlSpan>,
}

struct EndTag {
    name: Box<str>,
    name_span: HtmlSpan,
    span: HtmlSpan,
}

impl EndTag {
    fn new(name: &[u8], span: HtmlSpan) -> Self {
        let name = decoded(name);
        let name_span = HtmlSpan {
            start: span.start + 2,
            end: span.start + 2 + name.len(),
        };
        Self {
            name,
            name_span,
            span,
        }
    }
}

struct TextToken {
    value: Box<str>,
    span: HtmlSpan,
}

struct DoctypeToken {
    name: Option<Box<str>>,
    public_identifier: Option<Box<str>>,
    system_identifier: Option<Box<str>>,
    force_quirks: bool,
    span: HtmlSpan,
}

struct HtmlParseError {
    kind: HtmlError,
    span: HtmlSpan,
}

#[derive(Clone, Copy)]
struct HtmlSpan {
    start: usize,
    end: usize,
}

impl From<Span<usize>> for HtmlSpan {
    fn from(span: Span<usize>) -> Self {
        Self {
            start: span.start,
            end: span.end,
        }
    }
}

struct AuthoredAttribute {
    span: HtmlSpan,
    value_span: HtmlSpan,
}

/// Owns every conversion from tokenizer coordinates into Onmark source facts.
#[derive(Clone, Copy)]
struct SourceText<'a> {
    id: SourceId,
    text: &'a str,
}

impl<'a> SourceText<'a> {
    const fn new(id: SourceId, text: &'a str) -> Self {
        Self { id, text }
    }

    fn attribute(self, token: &AttributeToken, authored_end: usize) -> AuthoredAttribute {
        let end = trim_ascii_whitespace_end(self.text, token.name_span.start, authored_end);
        let value_span = token
            .tokenizer_value_span
            .unwrap_or_else(|| empty_attribute_value(self.text, token.name_span, end));
        AuthoredAttribute {
            span: HtmlSpan {
                start: token.name_span.start,
                end,
            },
            value_span,
        }
    }

    fn slice(self, span: HtmlSpan) -> &'a str {
        &self.text[span.start..span.end]
    }

    fn character_reference(self, error: HtmlSpan) -> HtmlSpan {
        let error = self.byte_aligned(error);
        let start = self.text[..error.start].rfind('&').unwrap_or(error.start);
        HtmlSpan {
            start,
            end: error.end,
        }
    }

    fn span(self, span: HtmlSpan) -> SourceSpan {
        let span = self.byte_aligned(span);
        self.range(byte_offset(span.start), byte_offset(span.end))
    }

    fn end(self) -> ByteOffset {
        byte_offset(self.text.len())
    }

    fn range(self, start: ByteOffset, end: ByteOffset) -> SourceSpan {
        SourceSpan::new(self.id, start, end).expect("tokenizer spans have ordered bounds")
    }

    fn byte_aligned(self, span: HtmlSpan) -> HtmlSpan {
        // html5gum normally reports byte offsets, but recovery can move an
        // error position one input byte at a time through a UTF-8 scalar.
        // Product spans must remain safe Rust string boundaries.
        let mut start = span.start.min(self.text.len());
        while !self.text.is_char_boundary(start) {
            start -= 1;
        }

        let mut end = span.end.min(self.text.len());
        while !self.text.is_char_boundary(end) {
            end += 1;
        }

        HtmlSpan { start, end }
    }
}

fn empty_attribute_value(text: &str, name: HtmlSpan, end: usize) -> HtmlSpan {
    let tail = &text[name.end..end];
    let Some(equals) = tail.find('=') else {
        return HtmlSpan {
            start: name.end,
            end: name.end,
        };
    };
    let after_equals = name.end + equals + 1;
    let value_start = skip_ascii_whitespace(text, after_equals, end);
    let start = match text.as_bytes().get(value_start) {
        Some(b'"' | b'\'') => value_start + 1,
        _ => value_start,
    };
    HtmlSpan { start, end: start }
}

fn trim_ascii_whitespace_end(text: &str, start: usize, mut end: usize) -> usize {
    while end > start && text.as_bytes()[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    end
}

fn skip_ascii_whitespace(text: &str, mut start: usize, end: usize) -> usize {
    while start < end && text.as_bytes()[start].is_ascii_whitespace() {
        start += 1;
    }
    start
}

fn decoded(bytes: &[u8]) -> Box<str> {
    Box::from(
        std::str::from_utf8(bytes).expect("html5gum emits UTF-8 after tokenizing a Rust string"),
    )
}

fn optional_decoded(bytes: &[u8]) -> Option<Box<str>> {
    (!bytes.is_empty()).then(|| decoded(bytes))
}

fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "source"
            | "track"
            | "wbr"
    )
}

fn is_character_reference_error(error: HtmlError) -> bool {
    matches!(
        error,
        HtmlError::AbsenceOfDigitsInNumericCharacterReference
            | HtmlError::CharacterReferenceOutsideUnicodeRange
            | HtmlError::ControlCharacterReference
            | HtmlError::MissingSemicolonAfterCharacterReference
            | HtmlError::NoncharacterCharacterReference
            | HtmlError::NullCharacterReference
            | HtmlError::SurrogateCharacterReference
            | HtmlError::UnknownNamedCharacterReference
    )
}

fn byte_offset(value: usize) -> ByteOffset {
    ByteOffset::new(u64::try_from(value).expect("Onmark source offsets fit in u64"))
}

fn source_limit_offset(text: &str, limit: usize) -> ByteOffset {
    let mut offset = limit.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    byte_offset(offset)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::model::{SourceId, SourceSpan};
    use crate::syntax::{Node, SourceDocument};

    use super::{Parser, SyntaxLimits, parse};

    #[test]
    fn preserves_multiple_top_level_elements_for_binding() {
        let source = "<om-film></om-film><om-film></om-film>";
        let (document, errors) = parse(SourceId::new(0), source).into_parts();

        assert!(errors.is_empty());
        assert_eq!(document.nodes().len(), 2);
    }

    #[test]
    fn rejects_non_void_self_closing_elements() {
        let (_, errors) = parse(SourceId::new(0), "<video />").into_parts();

        assert!(matches!(
            errors[0].kind(),
            super::SyntaxErrorKind::SelfClosingNonVoid { name } if name.as_ref() == "video"
        ));
    }

    #[test]
    fn reports_mismatched_custom_element_closes() {
        let source = "<om-film><om-title></om-film>";
        let (_, errors) = parse(SourceId::new(0), source).into_parts();

        assert!(errors.iter().any(|error| matches!(
            error.kind(),
            super::SyntaxErrorKind::MismatchedClosingTag { expected, found, .. }
                if expected.as_ref() == "om-title" && found.as_ref() == "om-film"
        )));
    }

    #[test]
    fn recovers_the_authored_character_reference_span() {
        let source = "<om-title>Tom &bogus;</om-title>";
        let (_, errors) = parse(SourceId::new(0), source).into_parts();
        let error = errors
            .iter()
            .find(|error| {
                matches!(
                    error.kind(),
                    super::SyntaxErrorKind::InvalidCharacterReference { .. }
                )
            })
            .expect("the source contains one invalid character reference");

        assert_eq!(error.span().start().get(), 14);
        assert_eq!(error.span().end().get(), 21);
        assert!(matches!(
            error.kind(),
            super::SyntaxErrorKind::InvalidCharacterReference { reference }
                if reference.as_ref() == "&bogus;"
        ));
    }

    #[test]
    fn aligns_invalid_reference_spans_to_utf8_boundaries() {
        let source = "&#¡a";
        let (_, errors) = parse(SourceId::new(0), source).into_parts();
        let error = errors
            .iter()
            .find(|error| {
                matches!(
                    error.kind(),
                    super::SyntaxErrorKind::InvalidCharacterReference { .. }
                )
            })
            .expect("the source contains one invalid character reference");

        assert_eq!(&source[span_range(error.span())], "&#¡");
    }

    #[test]
    fn stops_before_retaining_excessive_nesting() {
        let source = format!("{}{}", "<x>".repeat(5), "</x>".repeat(5));
        let limits = SyntaxLimits {
            source_bytes: source.len(),
            retained_items: 32,
            nesting_depth: 4,
        };
        let (_, errors) = Parser::new(SourceId::new(0), &source, limits)
            .parse()
            .into_parts();

        assert!(matches!(
            errors[0].kind(),
            super::SyntaxErrorKind::ResourceLimit {
                resource: super::SyntaxResource::NestingDepth,
            }
        ));
    }

    #[test]
    fn stops_before_retaining_excessive_items() {
        let source = "<om-film a=1 b=2></om-film>";
        let limits = SyntaxLimits {
            source_bytes: source.len(),
            retained_items: 2,
            nesting_depth: 4,
        };
        let (_, errors) = Parser::new(SourceId::new(0), source, limits)
            .parse()
            .into_parts();

        assert!(matches!(
            errors[0].kind(),
            super::SyntaxErrorKind::ResourceLimit {
                resource: super::SyntaxResource::Items,
            }
        ));
    }

    #[test]
    fn locates_the_source_byte_limit_at_a_utf8_boundary() {
        let limits = SyntaxLimits {
            source_bytes: 2,
            retained_items: 8,
            nesting_depth: 4,
        };
        let source = "a电";
        let (_, errors) = Parser::new(SourceId::new(0), source, limits)
            .parse()
            .into_parts();

        assert!(matches!(
            errors[0].kind(),
            super::SyntaxErrorKind::ResourceLimit {
                resource: super::SyntaxResource::SourceBytes,
            }
        ));
        assert!(source.is_char_boundary(
            usize::try_from(errors[0].span().start().get()).expect("the fixture offset fits usize"),
        ));
    }

    proptest! {
        #[test]
        fn recovered_spans_remain_inside_the_source(
            characters in proptest::collection::vec(any::<char>(), 0..128),
        ) {
            let source = characters.into_iter().collect::<String>();
            let (document, errors) = parse(SourceId::new(0), &source).into_parts();
            let source_len = u64::try_from(source.len()).expect("test strings fit in u64");

            prop_assert!(document_spans_are_bounded(&document, source_len));
            prop_assert!(errors
                .iter()
                .all(|error| span_is_bounded(error.span(), source_len)));
        }
    }

    fn document_spans_are_bounded(document: &SourceDocument, source_len: u64) -> bool {
        document
            .nodes()
            .iter()
            .all(|node| node_spans_are_bounded(node, source_len))
    }

    fn node_spans_are_bounded(node: &Node, source_len: u64) -> bool {
        match node {
            Node::Element(element) => {
                span_is_bounded(element.span(), source_len)
                    && span_is_bounded(element.name().span(), source_len)
                    && element.attributes().iter().all(|attribute| {
                        span_is_bounded(attribute.span(), source_len)
                            && span_is_bounded(attribute.name().span(), source_len)
                            && span_is_bounded(attribute.value_span(), source_len)
                    })
                    && element
                        .children()
                        .iter()
                        .all(|child| node_spans_are_bounded(child, source_len))
            }
            Node::Text(text) => span_is_bounded(text.span(), source_len),
        }
    }

    fn span_is_bounded(span: SourceSpan, source_len: u64) -> bool {
        span.start().get() <= span.end().get() && span.end().get() <= source_len
    }

    fn span_range(span: SourceSpan) -> std::ops::Range<usize> {
        usize::try_from(span.start().get()).expect("the test span fits usize")
            ..usize::try_from(span.end().get()).expect("the test span fits usize")
    }
}
