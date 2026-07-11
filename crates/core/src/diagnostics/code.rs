use std::fmt;

/// Stable identity of an authored problem.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DiagnosticCode {
    /// An authored node ID violates the language's ID rules.
    InvalidNodeId,
}

impl DiagnosticCode {
    /// Returns the stable external representation of this code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidNodeId => "ONM-ID-001",
        }
    }

    /// Returns the severity fixed by this diagnostic code.
    #[must_use]
    pub const fn severity(self) -> Severity {
        match self {
            Self::InvalidNodeId => Severity::Error,
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
    }
}
