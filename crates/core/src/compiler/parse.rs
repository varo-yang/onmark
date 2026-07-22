//! Compiler-owned translation from syntax failures to stable diagnostics.
//!
//! The syntax module deliberately knows neither diagnostic codes nor wording;
//! this facade is the one boundary where parser detail becomes product output.

use crate::diagnostics::{Diagnostic, DiagnosticCode, Diagnostics};
use crate::model::{SourceId, SourceSpan};
use crate::syntax::{
    self, SourceDocument, SyntaxError, SyntaxErrorKind, SyntaxResource, UnsupportedDirective,
};

use super::diagnostic::author_diagnostic;

/// Recovered source syntax and stable authored diagnostics.
///
/// The document remains available when diagnostics are present so later
/// compiler phases can aggregate independent authored mistakes safely.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseReport {
    document: SourceDocument,
    diagnostics: Diagnostics,
}

impl ParseReport {
    /// Returns the recovered source syntax tree.
    #[must_use]
    pub const fn document(&self) -> &SourceDocument {
        &self.document
    }

    /// Returns stable authored diagnostics.
    #[must_use]
    pub const fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// Consumes the report into its syntax tree and diagnostics.
    #[must_use]
    pub fn into_parts(self) -> (SourceDocument, Diagnostics) {
        (self.document, self.diagnostics)
    }
}

/// Parses screenplay markup without filesystem or environment access.
///
/// This phase checks markup structure only. Film roots, element vocabulary,
/// IDs, and references remain unresolved until binding.
#[must_use]
pub fn parse(source: SourceId, text: &str) -> ParseReport {
    let (document, errors) = syntax::parse(source, text).into_parts();
    let mut diagnostics = Diagnostics::new();

    for error in errors {
        diagnostics.push(translate_error(&error));
    }

    ParseReport {
        document,
        diagnostics,
    }
}

fn translate_error(error: &SyntaxError) -> Diagnostic {
    match error.kind() {
        SyntaxErrorKind::MalformedMarkup => malformed_markup(error.span()),
        SyntaxErrorKind::MismatchedClosingTag {
            expected,
            found,
            opened_at,
        } => mismatched_closing_tag(error.span(), expected, found, *opened_at),
        SyntaxErrorKind::DuplicateAttribute {
            name,
            first_declared_at,
        } => duplicate_attribute(error.span(), name, *first_declared_at),
        SyntaxErrorKind::InvalidCharacterReference { reference } => {
            invalid_character_reference(error.span(), reference)
        }
        SyntaxErrorKind::UnclosedElement { name, ended_at } => {
            unclosed_element(error.span(), name, *ended_at)
        }
        SyntaxErrorKind::UnexpectedClosingTag { found } => {
            unexpected_closing_tag(error.span(), found)
        }
        SyntaxErrorKind::UnsupportedDirective { directive } => {
            unsupported_directive(error.span(), *directive)
        }
        SyntaxErrorKind::ResourceLimit { resource } => resource_limit(error.span(), *resource),
    }
}

fn malformed_markup(primary: SourceSpan) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::MalformedSyntax,
        primary,
        "screenplay markup is malformed",
        "check tag delimiters, quotes, and other markup punctuation",
    )
}

fn mismatched_closing_tag(
    primary: SourceSpan,
    expected: &str,
    found: &str,
    opened_at: SourceSpan,
) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::MismatchedClosingTag,
        primary,
        format!("closing tag </{found}> does not match open element <{expected}>"),
        format!("replace </{found}> with </{expected}>"),
    )
    .with_related(opened_at, format!("<{expected}> is opened here"))
    .expect("a formatted related message is non-blank")
}

fn duplicate_attribute(
    primary: SourceSpan,
    name: &str,
    first_declared_at: SourceSpan,
) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::DuplicateAttribute,
        primary,
        format!("attribute \"{name}\" is repeated on the same element"),
        format!("remove one \"{name}\" attribute"),
    )
    .with_related(first_declared_at, "the attribute is first declared here")
    .expect("the static related message is non-blank")
}

fn invalid_character_reference(primary: SourceSpan, reference: &str) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidCharacterReference,
        primary,
        format!("character reference \"{reference}\" is malformed or unsupported"),
        "use a valid numeric reference or amp, lt, gt, quot, or apos",
    )
}

fn unclosed_element(primary: SourceSpan, name: &str, ended_at: SourceSpan) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::UnclosedElement,
        primary,
        format!("element <{name}> is not closed before the screenplay ends"),
        format!("add a closing </{name}> tag"),
    )
    .with_related(
        ended_at,
        "the screenplay ends before this element is closed",
    )
    .expect("a formatted related message is non-blank")
}

fn unexpected_closing_tag(primary: SourceSpan, found: &str) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::UnexpectedClosingTag,
        primary,
        format!("closing tag </{found}> has no open element"),
        format!("remove </{found}> or add its opening tag"),
    )
}

fn unsupported_directive(primary: SourceSpan, directive: UnsupportedDirective) -> Diagnostic {
    let name = match directive {
        UnsupportedDirective::ProcessingInstruction => "processing instruction",
        UnsupportedDirective::XmlDeclaration => "XML declaration",
        UnsupportedDirective::DocumentTypeDeclaration => "document type declaration",
    };

    author_diagnostic(
        DiagnosticCode::UnsupportedMarkupDirective,
        primary,
        format!("{name} is not supported in a screenplay"),
        format!("remove the {name}"),
    )
}

fn resource_limit(primary: SourceSpan, resource: SyntaxResource) -> Diagnostic {
    let (message, help) = match resource {
        SyntaxResource::SourceBytes => (
            "screenplay source exceeds the compiler byte limit",
            "split the screenplay or remove generated markup",
        ),
        SyntaxResource::Items => (
            "screenplay markup contains too many retained items",
            "split the screenplay or remove repeated elements, attributes, and text",
        ),
        SyntaxResource::NestingDepth => (
            "screenplay markup is nested beyond the compiler depth limit",
            "flatten the markup to the screenplay's film, scene, shot, and content structure",
        ),
    };

    author_diagnostic(
        DiagnosticCode::ScreenplayResourceLimit,
        primary,
        message,
        help,
    )
}

#[cfg(test)]
mod tests {
    use crate::diagnostics::{Diagnostic, DiagnosticCode};
    use crate::model::SourceId;

    use super::{ParseReport, parse};

    fn first_code(report: &ParseReport) -> Option<DiagnosticCode> {
        report.diagnostics().iter().next().map(Diagnostic::code)
    }

    #[test]
    fn translates_one_fatal_tokenizer_error_once() {
        let report = parse(SourceId::new(0), "<film id=\"");
        let diagnostics = report.diagnostics().iter().collect::<Vec<_>>();

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code(), DiagnosticCode::MalformedSyntax);
    }

    #[test]
    fn locates_a_fatal_error_after_multibyte_text() {
        let report = parse(SourceId::new(0), "<film>片<");
        let diagnostic = report
            .diagnostics()
            .iter()
            .next()
            .expect("the fixture contains malformed markup");

        assert_eq!(diagnostic.code(), DiagnosticCode::MalformedSyntax);
        assert_eq!(diagnostic.primary().start().get(), 9);
        assert_eq!(diagnostic.primary().end().get(), 9);
    }

    #[test]
    fn distinguishes_recoverable_structure_failures() {
        let unclosed = parse(SourceId::new(0), "<film>");
        let unexpected = parse(SourceId::new(0), "</film>");
        let directive = parse(SourceId::new(0), "<?render now?><film/>");

        assert_eq!(first_code(&unclosed), Some(DiagnosticCode::UnclosedElement),);
        assert_eq!(
            first_code(&unexpected),
            Some(DiagnosticCode::UnexpectedClosingTag),
        );
        assert_eq!(
            first_code(&directive),
            Some(DiagnosticCode::UnsupportedMarkupDirective),
        );
    }
}
