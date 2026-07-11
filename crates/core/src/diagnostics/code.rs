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
    /// A valid node ID is declared more than once in one film.
    DuplicateNodeId,
    /// An element name is outside the Gate-one screenplay vocabulary.
    UnknownElement,
    /// No top-level film element exists.
    MissingFilmRoot,
    /// More than one top-level film element exists.
    MultipleFilmRoots,
    /// A known element appears under a parent that cannot own it.
    MisplacedElement,
    /// A film contains more than one cues container.
    DuplicateCues,
    /// Authored text appears in a structural or empty element.
    UnexpectedText,
    /// An authored duration violates the exact duration grammar or range.
    InvalidDuration,
    /// A cue reference does not name a resolved cue.
    UnknownCueReference,
    /// A resolved cue is never referenced.
    UnusedCue,
    /// An element contains an attribute outside its language contract.
    UnknownAttribute,
    /// An element omits an attribute required for resolution.
    MissingRequiredAttribute,
    /// An authored attribute value violates its domain rules.
    InvalidAttributeValue,
    /// Two attributes define mutually exclusive rules.
    ConflictingAttributes,
    /// Markup tokenization failed before another trustworthy token was produced.
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
            Self::DuplicateNodeId => "ONM-ID-002",
            Self::UnknownElement => "ONM-STRUCT-001",
            Self::MissingFilmRoot => "ONM-STRUCT-002",
            Self::MultipleFilmRoots => "ONM-STRUCT-003",
            Self::MisplacedElement => "ONM-STRUCT-004",
            Self::DuplicateCues => "ONM-STRUCT-005",
            Self::UnexpectedText => "ONM-STRUCT-006",
            Self::InvalidDuration => "ONM-TIME-001",
            Self::UnknownCueReference => "ONM-REF-001",
            Self::UnusedCue => "ONM-REF-002",
            Self::UnknownAttribute => "ONM-ATTR-001",
            Self::MissingRequiredAttribute => "ONM-ATTR-002",
            Self::InvalidAttributeValue => "ONM-ATTR-003",
            Self::ConflictingAttributes => "ONM-ATTR-004",
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
            | Self::DuplicateNodeId
            | Self::UnknownElement
            | Self::MissingFilmRoot
            | Self::MultipleFilmRoots
            | Self::MisplacedElement
            | Self::DuplicateCues
            | Self::UnexpectedText
            | Self::InvalidDuration
            | Self::UnknownCueReference
            | Self::UnknownAttribute
            | Self::MissingRequiredAttribute
            | Self::InvalidAttributeValue
            | Self::ConflictingAttributes
            | Self::MalformedSyntax
            | Self::MismatchedClosingTag
            | Self::DuplicateAttribute
            | Self::InvalidCharacterReference
            | Self::UnclosedElement
            | Self::UnexpectedClosingTag
            | Self::UnsupportedMarkupDirective => Severity::Error,
            Self::UnusedCue => Severity::Warning,
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
        assert_eq!(DiagnosticCode::DuplicateNodeId.as_str(), "ONM-ID-002");
        assert_eq!(DiagnosticCode::UnknownElement.as_str(), "ONM-STRUCT-001");
        assert_eq!(DiagnosticCode::MissingFilmRoot.as_str(), "ONM-STRUCT-002");
        assert_eq!(DiagnosticCode::MultipleFilmRoots.as_str(), "ONM-STRUCT-003");
        assert_eq!(DiagnosticCode::MisplacedElement.as_str(), "ONM-STRUCT-004");
        assert_eq!(DiagnosticCode::DuplicateCues.as_str(), "ONM-STRUCT-005");
        assert_eq!(DiagnosticCode::UnexpectedText.as_str(), "ONM-STRUCT-006");
        assert_eq!(DiagnosticCode::InvalidDuration.as_str(), "ONM-TIME-001");
        assert_eq!(DiagnosticCode::UnknownCueReference.as_str(), "ONM-REF-001");
        assert_eq!(DiagnosticCode::UnusedCue.as_str(), "ONM-REF-002");
        assert_eq!(DiagnosticCode::UnusedCue.severity(), Severity::Warning);
        assert_eq!(DiagnosticCode::UnknownAttribute.as_str(), "ONM-ATTR-001");
        assert_eq!(
            DiagnosticCode::MissingRequiredAttribute.as_str(),
            "ONM-ATTR-002"
        );
        assert_eq!(
            DiagnosticCode::InvalidAttributeValue.as_str(),
            "ONM-ATTR-003"
        );
        assert_eq!(
            DiagnosticCode::ConflictingAttributes.as_str(),
            "ONM-ATTR-004"
        );
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
