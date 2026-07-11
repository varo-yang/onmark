//! Pure orchestration across screenplay compilation phases.
//!
//! ```
//! use onmark_core::compiler;
//! use onmark_core::model::SourceId;
//!
//! let report = compiler::parse(SourceId::new(0), "<film />");
//! assert!(report.diagnostics().is_empty());
//! assert_eq!(report.document().nodes().len(), 1);
//! ```

mod parse;

pub use parse::{ParseReport, parse};
