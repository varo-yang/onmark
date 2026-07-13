//! Versioned wire values shared across execution-process boundaries.
//!
//! Domain values remain owned by `model` and `timeline`. This module owns only
//! facts that cross Gate-one process boundaries: native/browser messages and
//! the Node/native presentation-bundle manifest.

mod bundle;
mod message;
mod plan;

pub use bundle::{
    BundleFile, BundleManifest, BundleVersion, InvalidBundleFile, InvalidBundleManifest,
};

pub use message::{
    BrowserCommand, BrowserEvent, BrowserRequest, BrowserResponse, InvalidProtocolFailure,
    ProtocolFailure, ProtocolFailureCode, ProtocolVersion, RUNTIME_HOST_NAME, RequestId,
};
pub use plan::{
    BrowserOverlay, BrowserOverlayKind, BrowserPlan, BrowserVideo, InvalidBrowserPlan,
    InvalidWireFrame, WireFrame, WireFrameRate, WireInterval,
};
