//! Foundational domain values shared by the compiler phases.
//!
//! This module depends on no other `onmark-core` module.

mod asset;
mod duration;
mod element;
mod id;
mod reference;
mod source;
mod time;

pub use asset::{
    AssetMetadata, AudioMetadata, FrozenAsset, FrozenAssetId, InvalidVideoMetadata, VideoMetadata,
    VideoTiming,
};
pub use duration::{Duration, InvalidDuration};
pub use element::ElementKind;
pub use id::{InvalidNodeId, NodeId};
pub use reference::{AssetRef, CueId, EventRef, InvalidAssetRef};
pub use source::{ByteOffset, InvalidSourceSpan, SourceId, SourceSpan};
pub use time::{
    FrameConversionOverflow, FrameCount, FrameIndex, FrameInterval, FrameRate,
    InvalidFrameInterval, InvalidFrameRate, Rounding, Timebase,
};
