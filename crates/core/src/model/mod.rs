//! Foundational domain values shared by the compiler phases.
//!
//! This module depends on no other `onmark-core` module.

mod duration;
mod element;
mod id;
mod reference;
mod source;
mod time;

pub use duration::{Duration, InvalidDuration};
pub use element::ElementKind;
pub use id::{InvalidNodeId, NodeId};
pub use reference::{AssetRef, CueId, EventRef, InvalidAssetRef};
pub use source::{ByteOffset, InvalidSourceSpan, SourceId, SourceSpan};
pub use time::{
    FrameCount, FrameIndex, FrameInterval, FrameRate, InvalidFrameInterval, InvalidFrameRate,
};
