//! Foundational domain values shared by the compiler phases.
//!
//! This module depends on no other `onmark-core` module.

mod asset;
mod audio;
mod caption;
mod duration;
mod element;
mod id;
mod reference;
mod source;
mod temporal;
mod time;

pub use asset::{
    AssetMetadata, AudioMetadata, FrozenAsset, FrozenAssetId, InvalidFrozenAssetId,
    InvalidVideoMetadata, VideoColorProfile, VideoMetadata, VideoTiming,
};
pub use audio::{
    AudioChannelLayout, AudioGain, AudioSampleConversionOverflow, AudioSampleCount,
    AudioSampleRate, InvalidAudioGain, InvalidAudioSampleRate,
};
pub use caption::{
    CaptionCue, CaptionInterval, CaptionTrack, InvalidCaptionCue, InvalidCaptionInterval,
    InvalidCaptionTrack,
};
pub use duration::{Duration, InvalidDuration};
pub use element::ElementKind;
pub use id::{InvalidNodeId, NodeId};
pub use reference::{AssetRef, CueId, EventRef, InvalidAssetRef};
pub use source::{ByteOffset, InvalidSourceSpan, SourceId, SourceSpan};
pub use temporal::{InvalidPresentationTemporalCapability, PresentationTemporalCapability};
pub use time::{
    FrameConversionOverflow, FrameCount, FrameIndex, FrameInterval, FrameRate,
    InvalidFrameInterval, InvalidFrameRate, Rounding, Timebase,
};
