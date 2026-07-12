//! Versioned wire values shared by native execution and the browser runtime.
//!
//! Domain values remain owned by `model` and `timeline`. This module lowers
//! only the facts consumed across the Gate-one browser boundary.
//! Native execution constructs and serializes requests; it deserializes and
//! validates responses returned by the browser.

mod message;
mod plan;

pub use message::{
    BrowserCommand, BrowserEvent, BrowserRequest, BrowserResponse, InvalidProtocolFailure,
    ProtocolFailure, ProtocolFailureCode, ProtocolVersion, RequestId,
};
pub use plan::{
    BrowserPlan, BrowserVideo, InvalidBrowserPlan, InvalidWireFrame, WireFrame, WireFrameRate,
    WireInterval,
};
