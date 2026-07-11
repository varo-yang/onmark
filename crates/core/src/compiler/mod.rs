//! Pure orchestration across screenplay compilation phases.
//!
//! ```
//! use onmark_core::compiler;
//! use onmark_core::model::SourceId;
//!
//! let parsed = compiler::parse(SourceId::new(0), "<film />");
//! let (document, syntax_diagnostics) = parsed.into_parts();
//! assert!(syntax_diagnostics.is_empty());
//!
//! let bound = compiler::bind(document);
//! assert!(bound.diagnostics().is_empty());
//! assert!(bound.film().is_some());
//! ```

mod bind;
mod linked;
mod parse;

pub use bind::{BindReport, bind};
pub use linked::{
    LinkedCue, LinkedCues, LinkedElement, LinkedFilm, LinkedNode, LinkedOverlay, LinkedScene,
    LinkedShot, LinkedShotContent, LinkedVideo, LinkedVoiceOver,
};
pub use parse::{ParseReport, parse};
