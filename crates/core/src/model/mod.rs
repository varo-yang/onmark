//! Foundational domain values shared by the compiler phases.
//!
//! This module depends on no other `onmark-core` module.

mod id;
mod source;
mod time;

pub use id::{InvalidNodeId, NodeId};
pub use source::{ByteOffset, InvalidSourceSpan, SourceId, SourceSpan};
pub use time::{
    FrameCount, FrameIndex, FrameInterval, FrameRate, InvalidFrameInterval, InvalidFrameRate,
};
