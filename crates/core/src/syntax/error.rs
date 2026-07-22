//! Syntax-local failure vocabulary before stable diagnostic translation.
//!
//! Third-party parser errors cannot cross this module boundary.

use crate::model::SourceSpan;

/// One authored markup error before the compiler assigns a stable code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SyntaxError {
    kind: SyntaxErrorKind,
    span: SourceSpan,
}

impl SyntaxError {
    pub(super) const fn new(kind: SyntaxErrorKind, span: SourceSpan) -> Self {
        Self { kind, span }
    }

    #[must_use]
    pub(crate) const fn kind(&self) -> &SyntaxErrorKind {
        &self.kind
    }

    #[must_use]
    pub(crate) const fn span(&self) -> SourceSpan {
        self.span
    }
}

/// Closed reasons keep `xmlparser` types from crossing the syntax boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SyntaxErrorKind {
    /// Tokenization failed before a trustworthy token could be produced.
    MalformedMarkup,
    /// A closing name differs from the currently open element.
    MismatchedClosingTag {
        expected: Box<str>,
        found: Box<str>,
        opened_at: SourceSpan,
    },
    /// An element contains the same qualified attribute name twice.
    DuplicateAttribute {
        name: Box<str>,
        first_declared_at: SourceSpan,
    },
    /// A character or entity reference is malformed or unsupported.
    InvalidCharacterReference { reference: Box<str> },
    /// The source ended before the current element was closed.
    UnclosedElement {
        name: Box<str>,
        ended_at: SourceSpan,
    },
    /// A closing tag appeared without an open element.
    UnexpectedClosingTag { found: Box<str> },
    /// XML machinery outside the screenplay surface was authored.
    UnsupportedDirective { directive: UnsupportedDirective },
    /// Retaining more source structure would exceed a compiler safety bound.
    ResourceLimit { resource: SyntaxResource },
}

/// Bounded syntax resource exhausted by authored input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SyntaxResource {
    /// UTF-8 source bytes.
    SourceBytes,
    /// Retained elements, attributes, or text nodes.
    Items,
    /// Open element nesting.
    NestingDepth,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UnsupportedDirective {
    /// An XML processing instruction such as `<?render now?>`.
    ProcessingInstruction,
    /// An XML declaration such as `<?xml version="1.0"?>`.
    XmlDeclaration,
    /// A document type declaration, including any internal subset.
    DocumentTypeDeclaration,
}
