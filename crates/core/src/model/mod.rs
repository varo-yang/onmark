//! Foundational domain values shared by the compiler phases.
//!
//! This module depends on no other `onmark-core` module.

mod asset;
mod audio;
mod caption;
mod duration;
mod element;
mod id;
mod media;
mod reference;
mod source;
mod temporal;
mod time;
mod variant;
mod visual;

pub use asset::{
    AssetMetadata, AudioMetadata, FrozenAsset, FrozenAssetId, InvalidFrozenAssetId,
    InvalidMediaTimebase, InvalidVideoDimensions, InvalidVideoFrameMap, InvalidVideoMetadata,
    MediaTimebase, MediaTimestampOverflow, VideoColorProfile, VideoDimensions, VideoFrameMap,
    VideoMetadata, VideoTiming,
};
pub use audio::{
    AudioChannelLayout, AudioEnvelope, AudioGain, AudioSampleConversionOverflow, AudioSampleCount,
    AudioSampleRate, InvalidAudioEnvelope, InvalidAudioGain, InvalidAudioSampleRate,
};
pub use caption::{
    CaptionCue, CaptionInterval, CaptionTrack, InvalidCaptionCue, InvalidCaptionInterval,
    InvalidCaptionTrack,
};
pub use duration::{Duration, InvalidDuration};
pub use element::{ElementKind, GeneralAudioKind};
pub use id::{InvalidNodeId, NodeId};
pub use media::{
    InvalidMediaSource, InvalidMediaSourceInterval, InvalidMediaTrim, InvalidPlayCount,
    InvalidPlaybackRate, MediaSource, MediaSourceInterval, MediaTrim, PlayCount, PlaybackRate,
};
pub use reference::{AssetRef, CueId, EventRef, InvalidAssetRef};
pub use source::{ByteOffset, InvalidSourceSpan, SourceId, SourceSpan};
pub use temporal::{InvalidPresentationTemporalCapability, PresentationTemporalCapability};
pub use time::{
    FrameConversionOverflow, FrameCount, FrameIndex, FrameInterval, FrameRate,
    InvalidFrameInterval, InvalidFrameRate, Rounding, Timebase,
};
pub use variant::{
    InvalidVariantFieldKind, InvalidVariantFieldName, InvalidVariantValue,
    MAX_EXACT_VARIANT_INTEGER, MAX_VARIANT_FIELD_NAME_BYTES, MAX_VARIANT_TEXT_BYTES,
    VariantFieldKind, VariantFieldName, VariantValue,
};
pub use visual::{
    InvalidPresentationDocumentScope, InvalidPresentationFrameBehavior,
    InvalidPresentationVisualCapability, PresentationDocumentScope, PresentationFrameBehavior,
    PresentationVisualCapability,
};
