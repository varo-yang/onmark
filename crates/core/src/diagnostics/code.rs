use std::fmt;

/// Stable identity of an authored problem.
///
/// Diagnostic codes are an expanding external protocol, so downstream code
/// must tolerate codes added by later Onmark versions. Local validation-reason
/// enums remain exhaustive because their variants belong to one constructor's
/// closed failure contract.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DiagnosticCode {
    /// An authored node ID violates the language's ID rules.
    InvalidNodeId,
    /// Markup tokenization failed or input ended unexpectedly.
    MalformedSyntax,
    /// A closing tag does not match the currently open element.
    MismatchedClosingTag,
    /// An element repeats the same qualified attribute name.
    DuplicateAttribute,
    /// Text or an attribute contains an invalid character reference.
    InvalidCharacterReference,
    /// The source ends before an element is closed.
    UnclosedElement,
    /// A closing tag appears without an open element.
    UnexpectedClosingTag,
    /// XML machinery outside the screenplay surface is authored.
    UnsupportedMarkupDirective,
}

impl DiagnosticCode {
    /// Returns the stable external representation of this code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidNodeId => "ONM-ID-001",
            Self::MalformedSyntax => "ONM-SYNTAX-001",
            Self::MismatchedClosingTag => "ONM-SYNTAX-002",
            Self::DuplicateAttribute => "ONM-SYNTAX-003",
            Self::InvalidCharacterReference => "ONM-SYNTAX-004",
            Self::UnclosedElement => "ONM-SYNTAX-005",
            Self::UnexpectedClosingTag => "ONM-SYNTAX-006",
            Self::UnsupportedMarkupDirective => "ONM-SYNTAX-007",
        }
    }

    /// Returns the severity fixed by this diagnostic code.
    #[must_use]
    pub const fn severity(self) -> Severity {
        match self {
            Self::InvalidNodeId
            | Self::MalformedSyntax
            | Self::MismatchedClosingTag
            | Self::DuplicateAttribute
            | Self::InvalidCharacterReference
            | Self::UnclosedElement
            | Self::UnexpectedClosingTag
            | Self::UnsupportedMarkupDirective => Severity::Error,
        }
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Effect a diagnostic has on compilation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Severity {
    /// Compilation cannot produce a valid result while this problem remains.
    Error,
    /// Compilation may continue, but the author should review the result.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Error => "error",
            Self::Warning => "warning",
        };

        formatter.write_str(text)
    }
}

#[cfg(test)]
mod tests {
    use super::{DiagnosticCode, Severity};

    #[test]
    fn exposes_stable_code_and_severity() {
        assert_eq!(DiagnosticCode::InvalidNodeId.as_str(), "ONM-ID-001");
        assert_eq!(DiagnosticCode::InvalidNodeId.severity(), Severity::Error);
        assert_eq!(DiagnosticCode::MalformedSyntax.as_str(), "ONM-SYNTAX-001",);
        assert_eq!(
            DiagnosticCode::MismatchedClosingTag.as_str(),
            "ONM-SYNTAX-002",
        );
        assert_eq!(
            DiagnosticCode::DuplicateAttribute.as_str(),
            "ONM-SYNTAX-003",
        );
        assert_eq!(
            DiagnosticCode::InvalidCharacterReference.as_str(),
            "ONM-SYNTAX-004",
        );
        assert_eq!(DiagnosticCode::UnclosedElement.as_str(), "ONM-SYNTAX-005");
        assert_eq!(
            DiagnosticCode::UnexpectedClosingTag.as_str(),
            "ONM-SYNTAX-006",
        );
        assert_eq!(
            DiagnosticCode::UnsupportedMarkupDirective.as_str(),
            "ONM-SYNTAX-007",
        );
    }
}
