//! Versioned wire values shared across execution-process boundaries.
//!
//! Domain values remain owned by `model` and `timeline`. This module owns only
//! facts that cross process boundaries: native/browser messages and
//! the Node/native presentation-bundle manifest.

mod bundle;
mod bundle_projection;
mod frame;
mod layout;
mod message;
mod plan;
mod projection;

pub use bundle::{
    BundleFile, BundleIdentity, BundleManifest, BundleVersion, InvalidBundleFile,
    InvalidBundleManifest,
};
pub use bundle_projection::{
    BundleProjection, BundleProjectionRegion, BundleProjectionVersion, InvalidBundleProjection,
};
pub use frame::{
    InvalidWireFrame, WireFrame, WireFrameRate, WireInterval, WireMediaTimebase, WirePlaybackRate,
};
pub use layout::{
    BROWSER_OBJECT_POSITION_SCALE, BrowserMediaLayout, BrowserMediaPlacement, BrowserObjectFit,
    BrowserObjectPosition, BrowserPixelRectangle, InvalidBrowserMediaLayout,
    InvalidBrowserObjectPosition, InvalidBrowserPixelRectangle, MAX_BROWSER_MEDIA_LAYOUTS,
};

pub use message::{
    BrowserCommand, BrowserEvent, BrowserMediaMode, BrowserRequest, BrowserResponse,
    InvalidProtocolFailure, ProtocolFailure, ProtocolFailureCode, ProtocolVersion,
    RUNTIME_HOST_NAME, RequestId,
};
pub use plan::{
    BrowserCaptionTrack, BrowserNode, BrowserNodeId, BrowserOverlay, BrowserOverlayKind,
    BrowserPlan, BrowserScene, BrowserShot, BrowserTransition, BrowserVariantField,
    BrowserVariantValue, BrowserVideo, BrowserVideoSource, BrowserVideoTiming, InvalidBrowserPlan,
};
