use std::cmp::Ordering;
use std::error::Error;
use std::fmt;

use crate::model::SourceSpan;

use super::{DiagnosticCode, Severity};

/// One structured report about authored input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
    code: DiagnosticCode,
    primary: SourceSpan,
    message: Box<str>,
    help: Option<Box<str>>,
    related: Vec<RelatedDiagnostic>,
}

impl Diagnostic {
    /// Creates a diagnostic with a non-blank message.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDiagnostic`] when `message` contains only whitespace.
    pub fn new(
        code: DiagnosticCode,
        primary: SourceSpan,
        message: impl Into<Box<str>>,
    ) -> Result<Self, InvalidDiagnostic> {
        let message = non_blank_text(message, InvalidDiagnostic::BlankMessage)?;

        Ok(Self {
            code,
            primary,
            message,
            help: None,
            related: Vec::new(),
        })
    }

    /// Adds a non-blank source-level repair suggestion.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDiagnostic`] when `help` contains only whitespace.
    pub fn with_help(mut self, help: impl Into<Box<str>>) -> Result<Self, InvalidDiagnostic> {
        self.help = Some(non_blank_text(help, InvalidDiagnostic::BlankHelp)?);
        Ok(self)
    }

    /// Adds a related source location with a non-blank explanation.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDiagnostic`] when `message` contains only whitespace.
    pub fn with_related(
        mut self,
        span: SourceSpan,
        message: impl Into<Box<str>>,
    ) -> Result<Self, InvalidDiagnostic> {
        let related = RelatedDiagnostic {
            span,
            message: non_blank_text(message, InvalidDiagnostic::BlankRelatedMessage)?,
        };
        let index = self.related.partition_point(|current| current <= &related);
        self.related.insert(index, related);
        Ok(self)
    }

    /// Returns the stable diagnostic code.
    #[must_use]
    pub const fn code(&self) -> DiagnosticCode {
        self.code
    }

    /// Returns the severity fixed by the diagnostic code.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.code.severity()
    }

    /// Returns the primary source location.
    #[must_use]
    pub const fn primary(&self) -> SourceSpan {
        self.primary
    }

    /// Returns the human-readable problem statement.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the optional source-level repair suggestion.
    #[must_use]
    pub fn help(&self) -> Option<&str> {
        self.help.as_deref()
    }

    /// Returns related source locations in deterministic source order.
    #[must_use]
    pub fn related(&self) -> &[RelatedDiagnostic] {
        &self.related
    }
}

/// Secondary source location attached to a diagnostic.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RelatedDiagnostic {
    span: SourceSpan,
    message: Box<str>,
}

impl RelatedDiagnostic {
    /// Returns the related source location.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }

    /// Returns the explanation of this relationship.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Deterministically ordered sequence of authored diagnostics.
///
/// Every submitted diagnostic is retained, including exact duplicates. The
/// compiler phase that owns a diagnostic is responsible for emitting it once;
/// silently deduplicating here would hide duplicate emission defects.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Diagnostics {
    entries: Vec<Diagnostic>,
}

impl Diagnostics {
    /// Creates an empty collection.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Adds one diagnostic.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        let index = self.entries.partition_point(|current| {
            compare_diagnostics(current, &diagnostic) != Ordering::Greater
        });
        self.entries.insert(index, diagnostic);
    }

    /// Returns the number of diagnostics.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the collection is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns whether the collection contains an error-severity diagnostic.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.entries
            .iter()
            .any(|diagnostic| diagnostic.severity() == Severity::Error)
    }

    /// Returns diagnostics in their current order.
    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Diagnostic> {
        self.entries.iter()
    }

    /// Returns the owned diagnostics in deterministic order.
    #[must_use]
    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.entries
    }
}

/// Reason a diagnostic cannot be constructed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidDiagnostic {
    /// The primary problem statement contains only whitespace.
    BlankMessage,
    /// The repair suggestion contains only whitespace.
    BlankHelp,
    /// A related-location explanation contains only whitespace.
    BlankRelatedMessage,
}

impl fmt::Display for InvalidDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::BlankMessage => "diagnostic message cannot be blank",
            Self::BlankHelp => "diagnostic help cannot be blank",
            Self::BlankRelatedMessage => "related diagnostic message cannot be blank",
        };

        formatter.write_str(message)
    }
}

impl Error for InvalidDiagnostic {}

fn non_blank_text(
    value: impl Into<Box<str>>,
    error: InvalidDiagnostic,
) -> Result<Box<str>, InvalidDiagnostic> {
    let value = value.into();

    if value.trim().is_empty() {
        return Err(error);
    }

    Ok(value)
}

fn compare_diagnostics(left: &Diagnostic, right: &Diagnostic) -> Ordering {
    left.primary
        .cmp(&right.primary)
        .then_with(|| left.code.as_str().cmp(right.code.as_str()))
        .then_with(|| left.message.cmp(&right.message))
        .then_with(|| left.help.cmp(&right.help))
        .then_with(|| left.related.cmp(&right.related))
}

#[cfg(test)]
mod tests {
    use crate::model::{ByteOffset, SourceId, SourceSpan};

    use super::{
        Diagnostic, DiagnosticCode, Diagnostics, InvalidDiagnostic, RelatedDiagnostic, Severity,
    };

    fn span(source: u32, start: u64, end: u64) -> SourceSpan {
        SourceSpan::new(
            SourceId::new(source),
            ByteOffset::new(start),
            ByteOffset::new(end),
        )
        .expect("the fixture has ordered source bounds")
    }

    #[test]
    fn builds_an_actionable_diagnostic() {
        let diagnostic = Diagnostic::new(
            DiagnosticCode::InvalidNodeId,
            span(0, 18, 31),
            "shot ID contains ASCII whitespace",
        )
        .expect("the fixture has a message")
        .with_help("replace the space with a hyphen")
        .expect("the fixture has help")
        .with_related(span(0, 0, 6), "inside this film")
        .expect("the fixture has a related message")
        .with_related(span(0, 40, 45), "later related location")
        .expect("the fixture has a related message")
        .with_related(span(0, 32, 36), "middle related location")
        .expect("the fixture has a related message");

        assert_eq!(diagnostic.code(), DiagnosticCode::InvalidNodeId);
        assert_eq!(diagnostic.severity(), Severity::Error);
        assert_eq!(diagnostic.primary(), span(0, 18, 31));
        assert_eq!(diagnostic.help(), Some("replace the space with a hyphen"));
        assert_eq!(
            diagnostic.related(),
            &[
                RelatedDiagnostic {
                    span: span(0, 0, 6),
                    message: Box::from("inside this film"),
                },
                RelatedDiagnostic {
                    span: span(0, 32, 36),
                    message: Box::from("middle related location"),
                },
                RelatedDiagnostic {
                    span: span(0, 40, 45),
                    message: Box::from("later related location"),
                },
            ],
        );
    }

    #[test]
    fn rejects_blank_authored_text() {
        let primary = span(0, 0, 0);

        assert_eq!(
            Diagnostic::new(DiagnosticCode::InvalidNodeId, primary, ""),
            Err(InvalidDiagnostic::BlankMessage),
        );
        assert_eq!(
            Diagnostic::new(DiagnosticCode::InvalidNodeId, primary, " \n\t"),
            Err(InvalidDiagnostic::BlankMessage),
        );

        let diagnostic = Diagnostic::new(DiagnosticCode::InvalidNodeId, primary, "invalid ID")
            .expect("the fixture has a message");
        assert_eq!(
            diagnostic.clone().with_help(""),
            Err(InvalidDiagnostic::BlankHelp),
        );
        assert_eq!(
            diagnostic.clone().with_help("\n"),
            Err(InvalidDiagnostic::BlankHelp),
        );
        assert_eq!(
            diagnostic.with_related(primary, "\t"),
            Err(InvalidDiagnostic::BlankRelatedMessage),
        );
    }

    #[test]
    fn sorts_diagnostics_deterministically() {
        let later = Diagnostic::new(DiagnosticCode::InvalidNodeId, span(1, 20, 25), "later")
            .expect("the fixture has a message");
        let earlier = Diagnostic::new(DiagnosticCode::InvalidNodeId, span(0, 10, 15), "earlier")
            .expect("the fixture has a message");
        let mut diagnostics = Diagnostics::new();

        diagnostics.push(later);
        diagnostics.push(earlier.clone());

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics.iter().next(), Some(&earlier));
        assert_eq!(diagnostics.into_vec()[0], earlier);
    }

    #[test]
    fn retains_duplicate_diagnostics() {
        let diagnostic =
            Diagnostic::new(DiagnosticCode::InvalidNodeId, span(0, 10, 15), "invalid ID")
                .expect("the fixture has a message");
        let mut diagnostics = Diagnostics::new();

        diagnostics.push(diagnostic.clone());
        diagnostics.push(diagnostic.clone());

        assert_eq!(diagnostics.into_vec(), vec![diagnostic.clone(), diagnostic]);
    }

    #[test]
    fn distinguishes_errors_from_warnings() {
        let warning = Diagnostic::new(
            DiagnosticCode::UnusedCue,
            span(0, 4, 8),
            "cue is never referenced",
        )
        .expect("the fixture has a message");
        let error = Diagnostic::new(
            DiagnosticCode::InvalidDuration,
            span(0, 12, 15),
            "duration is invalid",
        )
        .expect("the fixture has a message");
        let mut diagnostics = Diagnostics::new();

        diagnostics.push(warning);
        assert!(!diagnostics.has_errors());

        diagnostics.push(error);
        assert!(diagnostics.has_errors());
    }
}
