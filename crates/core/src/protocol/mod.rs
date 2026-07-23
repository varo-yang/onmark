//! Versioned wire values shared across execution-process boundaries.
//!
//! Domain values remain owned by `model` and `timeline`. This module owns only
//! facts that cross process boundaries: native/browser messages and
//! the Node/native presentation-bundle manifest.

mod bundle;
mod frame;
mod message;
mod plan;

pub use bundle::{
    BundleFile, BundleIdentity, BundleManifest, BundleVersion, InvalidBundleFile,
    InvalidBundleManifest,
};
pub use frame::{InvalidWireFrame, WireFrame, WireFrameRate, WireInterval};

pub use message::{
    BrowserCommand, BrowserEvent, BrowserRequest, BrowserResponse, InvalidProtocolFailure,
    ProtocolFailure, ProtocolFailureCode, ProtocolVersion, RUNTIME_HOST_NAME, RequestId,
};
pub use plan::{
    BrowserNode, BrowserNodeId, BrowserOverlay, BrowserOverlayKind, BrowserPlan, BrowserScene,
    BrowserShot, BrowserVideo, InvalidBrowserPlan,
};
