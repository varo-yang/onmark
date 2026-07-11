//! Foundational domain values shared by the compiler phases.
//!
//! This module depends on no other `onmark-core` module.

mod id;
mod time;

pub use id::{InvalidNodeId, NodeId};
pub use time::{
    FrameCount, FrameIndex, FrameInterval, FrameRate, InvalidFrameInterval, InvalidFrameRate,
};
