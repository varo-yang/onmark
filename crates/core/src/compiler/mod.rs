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
//! let (film, binding_diagnostics) = bound.into_parts();
//! assert!(binding_diagnostics.is_empty());
//!
//! let resolved = compiler::resolve(film.expect("the source contains one film"));
//! assert!(resolved.diagnostics().is_empty());
//! assert!(resolved.film().is_some());
//! ```

mod bind;
mod linked_film;
mod parse;
mod resolve;
mod resolved_film;

pub use bind::{BindReport, bind};
pub use linked_film::{
    LinkedCue, LinkedCues, LinkedElement, LinkedFilm, LinkedNode, LinkedOverlay, LinkedScene,
    LinkedShot, LinkedShotContent, LinkedVideo, LinkedVoiceOver,
};
pub use parse::{ParseReport, parse};
pub use resolve::{ResolveReport, resolve};
pub use resolved_film::{
    Authored, ResolvedCue, ResolvedCues, ResolvedElement, ResolvedFilm, ResolvedNode,
    ResolvedOverlay, ResolvedScene, ResolvedShot, ResolvedShotContent, ResolvedStart, ResolvedText,
    ResolvedVideo, ResolvedVoiceOver,
};
