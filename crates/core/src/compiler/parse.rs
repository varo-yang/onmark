use crate::diagnostics::{Diagnostic, DiagnosticCode, Diagnostics};
use crate::model::{SourceId, SourceSpan};
use crate::syntax::{self, SourceDocument, SyntaxError, SyntaxErrorKind, UnsupportedDirective};

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
    }
}

fn malformed_markup(primary: SourceSpan) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::MalformedSyntax,
        primary,
        "screenplay markup is malformed",
    )
    .expect("the static malformed-markup message is non-blank")
    .with_help("check tag delimiters, quotes, and other markup punctuation")
    .expect("the static malformed-markup help is non-blank")
}

fn mismatched_closing_tag(
    primary: SourceSpan,
    expected: &str,
    found: &str,
    opened_at: SourceSpan,
) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::MismatchedClosingTag,
        primary,
        format!("closing tag </{found}> does not match open element <{expected}>"),
    )
    .expect("a formatted closing-tag message is non-blank")
    .with_help(format!("replace </{found}> with </{expected}>"))
    .expect("a formatted closing-tag help is non-blank")
    .with_related(opened_at, format!("<{expected}> is opened here"))
    .expect("a formatted related message is non-blank")
}

fn duplicate_attribute(
    primary: SourceSpan,
    name: &str,
    first_declared_at: SourceSpan,
) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::DuplicateAttribute,
        primary,
        format!("attribute \"{name}\" is repeated on the same element"),
    )
    .expect("a formatted duplicate-attribute message is non-blank")
    .with_help(format!("remove one \"{name}\" attribute"))
    .expect("a formatted duplicate-attribute help is non-blank")
    .with_related(first_declared_at, "the attribute is first declared here")
    .expect("the static related message is non-blank")
}

fn invalid_character_reference(primary: SourceSpan, reference: &str) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::InvalidCharacterReference,
        primary,
        format!("character reference \"{reference}\" is malformed or unsupported"),
    )
    .expect("a formatted character-reference message is non-blank")
    .with_help("use a valid numeric reference or amp, lt, gt, quot, or apos")
    .expect("the static character-reference help is non-blank")
}

fn unclosed_element(primary: SourceSpan, name: &str, ended_at: SourceSpan) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::UnclosedElement,
        primary,
        format!("element <{name}> is not closed before the screenplay ends"),
    )
    .expect("a formatted unclosed-element message is non-blank")
    .with_help(format!("add a closing </{name}> tag"))
    .expect("a formatted unclosed-element help is non-blank")
    .with_related(
        ended_at,
        "the screenplay ends before this element is closed",
    )
    .expect("a formatted related message is non-blank")
}

fn unexpected_closing_tag(primary: SourceSpan, found: &str) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::UnexpectedClosingTag,
        primary,
        format!("closing tag </{found}> has no open element"),
    )
    .expect("a formatted unexpected-closing-tag message is non-blank")
    .with_help(format!("remove </{found}> or add its opening tag"))
    .expect("a formatted unexpected-closing-tag help is non-blank")
}

fn unsupported_directive(primary: SourceSpan, directive: UnsupportedDirective) -> Diagnostic {
    let name = match directive {
        UnsupportedDirective::ProcessingInstruction => "processing instruction",
        UnsupportedDirective::XmlDeclaration => "XML declaration",
        UnsupportedDirective::DocumentTypeDeclaration => "document type declaration",
    };

    Diagnostic::new(
        DiagnosticCode::UnsupportedMarkupDirective,
        primary,
        format!("{name} is not supported in a screenplay"),
    )
    .expect("a formatted unsupported-directive message is non-blank")
    .with_help(format!("remove the {name}"))
    .expect("a formatted unsupported-directive help is non-blank")
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
