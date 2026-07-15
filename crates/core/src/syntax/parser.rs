//! Span-preserving XML token adaptation and bounded syntax recovery.
//!
//! `xmlparser` owns lexical well-formedness; Onmark owns the recovered tree and
//! error vocabulary. Fatal tokenizer failures stop recovery because no later
//! fragment boundary can be trusted.

use crate::model::{ByteOffset, SourceId, SourceSpan};

use xmlparser::{ElementEnd, StrSpan, Token, Tokenizer};

use super::builder::TreeBuilder;
use super::reference;
use super::{
    Attribute, AttributeName, ElementName, Node, SourceDocument, SyntaxError, SyntaxErrorKind,
    TextNode, UnsupportedDirective,
};

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
    Parser::new(source, text).parse()
}

/// Single owner of the recovered tree and syntax errors for one source buffer.
struct Parser<'a> {
    source: SourceText<'a>,
    tree: TreeBuilder,
    errors: Vec<SyntaxError>,
}

impl<'a> Parser<'a> {
    fn new(source: SourceId, text: &'a str) -> Self {
        Self {
            source: SourceText::new(source, text),
            tree: TreeBuilder::new(),
            errors: Vec::new(),
        }
    }

    fn parse(mut self) -> SyntaxReport {
        let completed = match self.leading_markup() {
            Tokenization::ContinueAt(start) => self.consume_tokens(start),
            Tokenization::Stop => false,
        };
        self.finish_open_elements(completed);

        let source_span = self.source.range(ByteOffset::new(0), self.source.end());

        SyntaxReport {
            document: self.tree.into_document(source_span),
            errors: self.errors,
        }
    }

    fn leading_markup(&mut self) -> Tokenization {
        if !starts_with_doctype(self.source.text) {
            return Tokenization::ContinueAt(0);
        }

        for token in Tokenizer::from(self.source.text) {
            match token {
                Ok(Token::DtdStart { span, .. }) => {
                    self.reject_directive(UnsupportedDirective::DocumentTypeDeclaration, span);
                }
                Ok(Token::EmptyDtd { span, .. }) => {
                    self.reject_directive(UnsupportedDirective::DocumentTypeDeclaration, span);
                    return Tokenization::ContinueAt(span.end());
                }
                Ok(Token::DtdEnd { span }) => return Tokenization::ContinueAt(span.end()),
                // The opening token already rejected this declaration; its
                // internal tokens carry no additional authored mistake.
                Ok(_) => {}
                Err(error) => {
                    self.reject_tokenizer_error(error);
                    return Tokenization::Stop;
                }
            }
        }

        // Without a DTD end token there is no trustworthy fragment boundary.
        // The entire remaining source belongs to the rejected declaration.
        Tokenization::Stop
    }

    fn consume_tokens(&mut self, start: usize) -> bool {
        let text = self.source.text;
        for token in Tokenizer::from_fragment(text, start..text.len()) {
            match token {
                Ok(token) => self.consume(token),
                Err(error) => {
                    self.reject_tokenizer_error(error);
                    return false;
                }
            }
        }
        true
    }

    fn consume(&mut self, token: Token<'a>) {
        match token {
            Token::ElementStart {
                prefix,
                local,
                span,
            } => self.start_element(prefix, local, span),
            Token::Attribute {
                prefix,
                local,
                value,
                span,
            } => self.add_attribute(prefix, local, value, span),
            Token::ElementEnd { end, span } => self.end_element(end, span),
            Token::Text { text } => self.add_text(text),
            Token::Cdata { text, .. } => self.add_cdata(text),
            Token::ProcessingInstruction { span, .. } => {
                self.reject_directive(UnsupportedDirective::ProcessingInstruction, span);
            }
            Token::Declaration { span, .. } => {
                self.reject_directive(UnsupportedDirective::XmlDeclaration, span);
            }
            Token::DtdStart { span, .. } | Token::EmptyDtd { span, .. } => {
                self.reject_directive(UnsupportedDirective::DocumentTypeDeclaration, span);
            }
            // Comments have no syntax-tree representation. `DtdStart` already
            // rejects the whole declaration, so its remaining tokens are noise.
            Token::Comment { .. } | Token::EntityDeclaration { .. } | Token::DtdEnd { .. } => {}
        }
    }

    fn reject_tokenizer_error(&mut self, error: xmlparser::Error) {
        self.errors.push(SyntaxError::new(
            SyntaxErrorKind::MalformedMarkup,
            self.source.point(error.pos()),
        ));
    }

    fn reject_directive(&mut self, directive: UnsupportedDirective, span: StrSpan<'a>) {
        self.errors.push(SyntaxError::new(
            SyntaxErrorKind::UnsupportedDirective { directive },
            self.source.span(span),
        ));
    }

    fn start_element(&mut self, prefix: StrSpan<'a>, local: StrSpan<'a>, span: StrSpan<'a>) {
        let name = self.element_name(prefix, local);
        self.tree.start_element(name, byte_offset(span.start()));
    }

    fn add_attribute(
        &mut self,
        prefix: StrSpan<'a>,
        local: StrSpan<'a>,
        value: StrSpan<'a>,
        span: StrSpan<'a>,
    ) {
        let name = self.attribute_name(prefix, local);
        let value_span = self.source.span(value);
        let attribute_span = self.source.span(span);
        let (decoded, mut errors) =
            reference::decode(self.source.id, value.as_str(), value_span.start());
        self.errors.append(&mut errors);

        let attribute = Attribute::new(name, decoded, attribute_span, value_span);

        if let Some(error) = self.tree.add_attribute(attribute) {
            self.errors.push(error);
        }
    }

    fn end_element(&mut self, end: ElementEnd<'a>, span: StrSpan<'a>) {
        match end {
            ElementEnd::Open => self.tree.open_element(),
            ElementEnd::Empty => self
                .tree
                .finish_empty_element(self.source.id, byte_offset(span.end())),
            ElementEnd::Close(prefix, local) => self.close_element(prefix, local, span),
        }
    }

    fn close_element(&mut self, prefix: StrSpan<'a>, local: StrSpan<'a>, span: StrSpan<'a>) {
        let found = self.element_name(prefix, local);
        let error = self
            .tree
            .close_element(self.source.id, &found, byte_offset(span.end()));

        if let Some(error) = error {
            self.errors.push(error);
        }
    }

    fn add_text(&mut self, text: StrSpan<'a>) {
        let span = self.source.span(text);
        let (decoded, mut errors) = reference::decode(self.source.id, text.as_str(), span.start());
        self.errors.append(&mut errors);
        self.tree.append(Node::Text(TextNode::new(decoded, span)));
    }

    fn add_cdata(&mut self, text: StrSpan<'a>) {
        let text = TextNode::new(Box::from(text.as_str()), self.source.span(text));
        self.tree.append(Node::Text(text));
    }

    fn finish_open_elements(&mut self, completed: bool) {
        if completed {
            let offset = self.source.end();
            let ended_at = self.source.range(offset, offset);
            self.errors.extend(self.tree.unclosed_elements(ended_at));
        }

        self.tree.finish(self.source.id, self.source.end());
    }

    fn element_name(&self, prefix: StrSpan<'a>, local: StrSpan<'a>) -> ElementName {
        ElementName::new(
            optional_prefix(prefix),
            Box::from(local.as_str()),
            self.source.qualified_name_span(prefix, local),
        )
    }

    fn attribute_name(&self, prefix: StrSpan<'a>, local: StrSpan<'a>) -> AttributeName {
        AttributeName::new(
            optional_prefix(prefix),
            Box::from(local.as_str()),
            self.source.qualified_name_span(prefix, local),
        )
    }
}

/// Whether a trustworthy fragment remains after consuming leading markup.
enum Tokenization {
    ContinueAt(usize),
    Stop,
}

fn optional_prefix(prefix: StrSpan<'_>) -> Option<Box<str>> {
    // `xmlparser` represents an absent prefix as an empty span. Normalize that
    // external convention before constructing an owned syntax name.
    (!prefix.is_empty()).then(|| Box::from(prefix.as_str()))
}

fn starts_with_doctype(text: &str) -> bool {
    text.trim_start_matches([' ', '\t', '\r', '\n'])
        .starts_with("<!DOCTYPE")
}

/// Owns every conversion from tokenizer coordinates into Onmark source facts.
/// Keeping it separate prevents tree-building code from inventing span rules.
#[derive(Clone, Copy)]
struct SourceText<'a> {
    id: SourceId,
    text: &'a str,
}

impl<'a> SourceText<'a> {
    const fn new(id: SourceId, text: &'a str) -> Self {
        Self { id, text }
    }

    fn span(self, span: StrSpan<'_>) -> SourceSpan {
        self.range(byte_offset(span.start()), byte_offset(span.end()))
    }

    fn qualified_name_span(self, prefix: StrSpan<'_>, local: StrSpan<'_>) -> SourceSpan {
        let start = if prefix.is_empty() {
            local.start()
        } else {
            prefix.start()
        };

        self.range(byte_offset(start), byte_offset(local.end()))
    }

    fn point(self, position: xmlparser::TextPos) -> SourceSpan {
        // xmlparser 0.13 counts columns in Unicode scalar values, not bytes.
        // Reconstruct the byte offset with the same rule before storing it.
        let offset = byte_offset_at(self.text, position.row, position.col);
        self.range(offset, offset)
    }

    fn end(self) -> ByteOffset {
        byte_offset(self.text.len())
    }

    fn range(self, start: ByteOffset, end: ByteOffset) -> SourceSpan {
        SourceSpan::new(self.id, start, end).expect("tokenizer spans have ordered bounds")
    }
}

fn byte_offset(value: usize) -> ByteOffset {
    ByteOffset::new(u64::try_from(value).expect("Onmark source offsets fit in u64"))
}

fn byte_offset_at(text: &str, row: u32, column: u32) -> ByteOffset {
    let mut current_row = 1;
    let mut current_column = 1;

    for (offset, character) in text.char_indices() {
        if current_row == row && current_column == column {
            return byte_offset(offset);
        }

        if character == '\n' {
            current_row += 1;
            current_column = 1;
        } else {
            current_column += 1;
        }
    }

    byte_offset(text.len())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::model::{SourceId, SourceSpan};
    use crate::syntax::{Node, SourceDocument};

    use super::parse;

    #[test]
    fn preserves_multiple_top_level_elements_for_binding() {
        let (document, errors) = parse(SourceId::new(0), "<film/><film/>").into_parts();

        assert!(errors.is_empty());
        assert_eq!(document.nodes().len(), 2);
    }

    #[test]
    fn ignores_comments_and_preserves_cdata_as_literal_text() {
        let source = "<film><!-- note --><![CDATA[Tom &amp; Jerry]]></film>";
        let (document, errors) = parse(SourceId::new(0), source).into_parts();
        let Node::Element(film) = &document.nodes()[0] else {
            panic!("the fixture root must be an element");
        };
        let Node::Text(text) = &film.children()[0] else {
            panic!("CDATA must become a text node");
        };

        assert!(errors.is_empty());
        assert_eq!(film.children().len(), 1);
        assert_eq!(text.text(), "Tom &amp; Jerry");
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
        for node in document.nodes() {
            if !node_spans_are_bounded(node, source_len) {
                return false;
            }
        }

        true
    }

    fn node_spans_are_bounded(node: &Node, source_len: u64) -> bool {
        match node {
            Node::Element(element) => {
                if !span_is_bounded(element.span(), source_len)
                    || !span_is_bounded(element.name().span(), source_len)
                {
                    return false;
                }

                for attribute in element.attributes() {
                    if !span_is_bounded(attribute.span(), source_len)
                        || !span_is_bounded(attribute.name().span(), source_len)
                        || !span_is_bounded(attribute.value_span(), source_len)
                    {
                        return false;
                    }
                }

                element
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
}
