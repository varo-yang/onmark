//! Structured reports for authored problems.
//!
//! Diagnostics are expected compiler output. They are distinct from
//! infrastructure and internal execution errors.

mod code;
mod report;

pub use code::{DiagnosticCode, Severity};
pub use report::{Diagnostic, Diagnostics, InvalidDiagnostic, RelatedDiagnostic};
