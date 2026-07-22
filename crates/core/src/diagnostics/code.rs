//! Stable diagnostic identities and their centrally owned severity policy.

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
    /// An element name is outside the current screenplay vocabulary.
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
    /// A shot has no rule that determines its duration.
    MissingDurationSource,
    /// Explicit and content-derived shot durations compete.
    ConflictingDurationSources,
    /// Resolved content starts outside its owning shot.
    TimingOutsideShot,
    /// Exact time cannot fit in the selected frame domain.
    FrameConversionOverflow,
    /// A film has no shot with a positive solved duration.
    EmptyFilm,
    /// Renderable media omits its frozen artifact reference.
    MissingMediaSource,
    /// A media element references an artifact without its required track.
    IncompatibleMediaSource,
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
    /// Screenplay syntax exceeds a bounded compiler resource.
    ScreenplayResourceLimit,
    /// A standalone subtitle file violates its selected format grammar.
    InvalidSubtitleFile,
    /// A standalone subtitle file uses presentation semantics not represented by caption facts.
    UnsupportedSubtitleFeature,
    /// A standalone subtitle file exceeds a bounded ingestion limit.
    SubtitleResourceLimit,
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
            Self::MissingDurationSource => "ONM-TIME-002",
            Self::ConflictingDurationSources => "ONM-TIME-003",
            Self::TimingOutsideShot => "ONM-TIME-004",
            Self::FrameConversionOverflow => "ONM-TIME-005",
            Self::EmptyFilm => "ONM-TIME-006",
            Self::MissingMediaSource => "ONM-ASSET-001",
            Self::IncompatibleMediaSource => "ONM-ASSET-002",
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
            Self::ScreenplayResourceLimit => "ONM-SYNTAX-008",
            Self::InvalidSubtitleFile => "ONM-CAPTION-001",
            Self::UnsupportedSubtitleFeature => "ONM-CAPTION-002",
            Self::SubtitleResourceLimit => "ONM-CAPTION-003",
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
            | Self::MissingDurationSource
            | Self::ConflictingDurationSources
            | Self::TimingOutsideShot
            | Self::FrameConversionOverflow
            | Self::EmptyFilm
            | Self::MissingMediaSource
            | Self::IncompatibleMediaSource
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
            | Self::UnsupportedMarkupDirective
            | Self::ScreenplayResourceLimit
            | Self::InvalidSubtitleFile
            | Self::UnsupportedSubtitleFeature
            | Self::SubtitleResourceLimit => Severity::Error,
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
        let errors = [
            (DiagnosticCode::InvalidNodeId, "ONM-ID-001"),
            (DiagnosticCode::DuplicateNodeId, "ONM-ID-002"),
            (DiagnosticCode::UnknownElement, "ONM-STRUCT-001"),
            (DiagnosticCode::MissingFilmRoot, "ONM-STRUCT-002"),
            (DiagnosticCode::MultipleFilmRoots, "ONM-STRUCT-003"),
            (DiagnosticCode::MisplacedElement, "ONM-STRUCT-004"),
            (DiagnosticCode::DuplicateCues, "ONM-STRUCT-005"),
            (DiagnosticCode::UnexpectedText, "ONM-STRUCT-006"),
            (DiagnosticCode::InvalidDuration, "ONM-TIME-001"),
            (DiagnosticCode::MissingDurationSource, "ONM-TIME-002"),
            (DiagnosticCode::ConflictingDurationSources, "ONM-TIME-003"),
            (DiagnosticCode::TimingOutsideShot, "ONM-TIME-004"),
            (DiagnosticCode::FrameConversionOverflow, "ONM-TIME-005"),
            (DiagnosticCode::EmptyFilm, "ONM-TIME-006"),
            (DiagnosticCode::MissingMediaSource, "ONM-ASSET-001"),
            (DiagnosticCode::IncompatibleMediaSource, "ONM-ASSET-002"),
            (DiagnosticCode::UnknownCueReference, "ONM-REF-001"),
            (DiagnosticCode::UnknownAttribute, "ONM-ATTR-001"),
            (DiagnosticCode::MissingRequiredAttribute, "ONM-ATTR-002"),
            (DiagnosticCode::InvalidAttributeValue, "ONM-ATTR-003"),
            (DiagnosticCode::ConflictingAttributes, "ONM-ATTR-004"),
            (DiagnosticCode::MalformedSyntax, "ONM-SYNTAX-001"),
            (DiagnosticCode::MismatchedClosingTag, "ONM-SYNTAX-002"),
            (DiagnosticCode::DuplicateAttribute, "ONM-SYNTAX-003"),
            (DiagnosticCode::InvalidCharacterReference, "ONM-SYNTAX-004"),
            (DiagnosticCode::UnclosedElement, "ONM-SYNTAX-005"),
            (DiagnosticCode::UnexpectedClosingTag, "ONM-SYNTAX-006"),
            (DiagnosticCode::UnsupportedMarkupDirective, "ONM-SYNTAX-007"),
            (DiagnosticCode::ScreenplayResourceLimit, "ONM-SYNTAX-008"),
            (DiagnosticCode::InvalidSubtitleFile, "ONM-CAPTION-001"),
            (
                DiagnosticCode::UnsupportedSubtitleFeature,
                "ONM-CAPTION-002",
            ),
            (DiagnosticCode::SubtitleResourceLimit, "ONM-CAPTION-003"),
        ];

        for (code, stable) in errors {
            assert_eq!(code.as_str(), stable);
            assert_eq!(code.severity(), Severity::Error);
        }

        assert_eq!(DiagnosticCode::UnusedCue.as_str(), "ONM-REF-002");
        assert_eq!(DiagnosticCode::UnusedCue.severity(), Severity::Warning);
    }
}
