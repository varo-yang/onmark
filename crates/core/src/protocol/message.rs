//! Versioned request/response envelope for native-to-browser execution.
//!
//! The protocol transports solved facts and readiness only; browser vendor
//! types and runtime-owned timing decisions are intentionally absent.

use std::error::Error;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use super::{BrowserPlan, WireFrame};

/// Browser-global capability installed by every compatible runtime bundle.
pub const RUNTIME_HOST_NAME: &str = "__ONMARK_RUNTIME__";

const MAX_FAILURE_MESSAGE_CHARACTERS: usize = 4_096;
const MAX_PENDING_RESOURCES: usize = 256;
const MAX_PENDING_RESOURCE_CHARACTERS: usize = 1_024;

/// Version of the native-to-browser message contract.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("const" = 1)))]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ProtocolVersion(u16);

impl ProtocolVersion {
    /// First browser protocol implemented by Gate one.
    pub const V1: Self = Self(1);

    /// Returns the stable integer representation.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl<'de> Deserialize<'de> for ProtocolVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let version = u16::deserialize(deserializer)?;
        if version == Self::V1.get() {
            return Ok(Self::V1);
        }

        Err(D::Error::custom("unsupported browser protocol version"))
    }
}

/// Correlation identity shared by one request and its response events.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RequestId(#[cfg_attr(feature = "schema", schemars(range(max = u32::MAX)))] u32);

impl RequestId {
    /// Creates a request identity selected by the native executor.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the stable integer representation.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// One versioned command sent from the native executor to the browser.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(extend("x-onmark-runtime-host" = RUNTIME_HOST_NAME))
)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserRequest {
    version: ProtocolVersion,
    request_id: RequestId,
    command: BrowserCommand,
}

impl BrowserRequest {
    /// Wraps one Gate-one command in the current protocol envelope.
    #[must_use]
    pub const fn new(request_id: RequestId, command: BrowserCommand) -> Self {
        Self {
            version: ProtocolVersion::V1,
            request_id,
            command,
        }
    }

    /// Returns the browser protocol version.
    #[must_use]
    pub const fn version(&self) -> ProtocolVersion {
        self.version
    }

    /// Returns the request correlation identity.
    #[must_use]
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Returns the enclosed command.
    #[must_use]
    pub const fn command(&self) -> &BrowserCommand {
        &self.command
    }
}

/// Closed Gate-one commands understood by the browser runtime.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(
    deny_unknown_fields,
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BrowserCommand {
    /// Install one immutable browser plan.
    Load {
        /// Solved frame facts consumed by the runtime clock.
        plan: BrowserPlan,
    },
    /// Stabilize resources at the evaluation start frame.
    Prepare {
        /// First frame that may be evaluated by this unit.
        evaluation_start: WireFrame,
    },
    /// Evaluate one exact absolute frame.
    Seek {
        /// Frame selected by the native executor.
        frame: WireFrame,
    },
    /// Confirm staged media reached the compositor before accepting capture.
    Confirm {
        /// Frame whose staged media must be compositor-confirmed.
        frame: WireFrame,
    },
    /// Release page-owned resources for this session.
    Dispose,
}

/// One versioned event returned by the browser runtime.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(
    feature = "schema",
    schemars(
        extend("x-onmark-max-failure-message-characters" = MAX_FAILURE_MESSAGE_CHARACTERS),
        extend("x-onmark-max-pending-resources" = MAX_PENDING_RESOURCES),
        extend(
            "x-onmark-max-pending-resource-characters" = MAX_PENDING_RESOURCE_CHARACTERS
        )
    )
)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserResponse {
    version: ProtocolVersion,
    request_id: RequestId,
    event: BrowserEvent,
}

impl BrowserResponse {
    /// Wraps one Gate-one event in the current protocol envelope.
    #[must_use]
    pub const fn new(request_id: RequestId, event: BrowserEvent) -> Self {
        Self {
            version: ProtocolVersion::V1,
            request_id,
            event,
        }
    }

    /// Returns the browser protocol version.
    #[must_use]
    pub const fn version(&self) -> ProtocolVersion {
        self.version
    }

    /// Returns the request correlation identity.
    #[must_use]
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    /// Returns the enclosed browser event.
    #[must_use]
    pub const fn event(&self) -> &BrowserEvent {
        &self.event
    }
}

/// Closed Gate-one events emitted by the browser runtime.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    deny_unknown_fields,
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BrowserEvent {
    /// The immutable browser plan was accepted.
    Loaded,
    /// Resources required at the evaluation start are stable.
    Prepared {
        /// Frame at which preparation completed.
        evaluation_start: WireFrame,
    },
    /// One requested frame has been staged for the compositor.
    FrameStaged {
        /// Exact frame represented by staged browser state.
        frame: WireFrame,
    },
    /// The captured payload's staged media passed compositor confirmation.
    FrameReady {
        /// Exact frame confirmed by browser media state.
        frame: WireFrame,
    },
    /// The browser rejected a command or could not reach readiness.
    Failed(ProtocolFailure),
    /// The browser session released its resources.
    Disposed,
}

/// Actionable details for one failed browser command.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProtocolFailure {
    /// Stable machine-readable failure category.
    code: ProtocolFailureCode,
    /// Direct explanation in browser/runtime terms.
    #[cfg_attr(
        feature = "schema",
        schemars(
            length(max = MAX_FAILURE_MESSAGE_CHARACTERS),
            regex(pattern = r"\S")
        )
    )]
    message: Box<str>,
    /// Resources that prevented readiness, in caller-owned deterministic order.
    #[cfg_attr(
        feature = "schema",
        schemars(
            length(max = MAX_PENDING_RESOURCES),
            inner(
                length(max = MAX_PENDING_RESOURCE_CHARACTERS),
                regex(pattern = r"\S")
            )
        )
    )]
    pending_resources: Vec<Box<str>>,
}

impl ProtocolFailure {
    /// Creates one validated failure report.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidProtocolFailure`] when the report exceeds its V1 wire
    /// budget or contains a blank message or resource description.
    pub fn new(
        code: ProtocolFailureCode,
        message: impl Into<Box<str>>,
        pending_resources: Vec<Box<str>>,
    ) -> Result<Self, InvalidProtocolFailure> {
        let message = message.into();
        if message.trim().is_empty() {
            return Err(InvalidProtocolFailure::BlankMessage);
        }
        if exceeds_character_limit(&message, MAX_FAILURE_MESSAGE_CHARACTERS) {
            return Err(InvalidProtocolFailure::MessageTooLong);
        }
        if pending_resources.len() > MAX_PENDING_RESOURCES {
            return Err(InvalidProtocolFailure::TooManyPendingResources);
        }
        for (index, resource) in pending_resources.iter().enumerate() {
            if resource.trim().is_empty() {
                return Err(InvalidProtocolFailure::BlankPendingResource(index));
            }
            if exceeds_character_limit(resource, MAX_PENDING_RESOURCE_CHARACTERS) {
                return Err(InvalidProtocolFailure::PendingResourceTooLong(index));
            }
        }

        Ok(Self {
            code,
            message,
            pending_resources,
        })
    }

    /// Returns the stable failure category.
    #[must_use]
    pub const fn code(&self) -> ProtocolFailureCode {
        self.code
    }

    /// Returns the direct failure explanation.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns resources that prevented readiness.
    #[must_use]
    pub fn pending_resources(&self) -> impl ExactSizeIterator<Item = &str> {
        self.pending_resources.iter().map(Box::as_ref)
    }
}

impl<'de> Deserialize<'de> for ProtocolFailure {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let failure = ProtocolFailureWire::deserialize(deserializer)?;
        Self::new(failure.code, failure.message, failure.pending_resources)
            .map_err(D::Error::custom)
    }
}

// The helper preserves the flat wire shape while routing construction through
// the public invariant-checking constructor.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ProtocolFailureWire {
    code: ProtocolFailureCode,
    message: Box<str>,
    pending_resources: Vec<Box<str>>,
}

/// Reason browser failure details are not actionable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidProtocolFailure {
    /// The direct failure explanation contains no visible content.
    BlankMessage,
    /// The direct failure explanation exceeds the V1 character limit.
    MessageTooLong,
    /// The report contains more pending resources than V1 can carry.
    TooManyPendingResources,
    /// One pending-resource description contains no visible content.
    BlankPendingResource(usize),
    /// One pending-resource description exceeds the V1 character limit.
    PendingResourceTooLong(usize),
}

impl fmt::Display for InvalidProtocolFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BlankMessage => formatter.write_str("protocol failure message cannot be blank"),
            Self::MessageTooLong => {
                formatter.write_str("protocol failure message exceeds the V1 character limit")
            }
            Self::TooManyPendingResources => {
                formatter.write_str("protocol failure has too many pending resources")
            }
            Self::BlankPendingResource(index) => {
                write!(
                    formatter,
                    "pending resource at index {index} cannot be blank"
                )
            }
            Self::PendingResourceTooLong(index) => {
                write!(
                    formatter,
                    "pending resource at index {index} exceeds the V1 character limit"
                )
            }
        }
    }
}

impl Error for InvalidProtocolFailure {}

fn exceeds_character_limit(value: &str, limit: usize) -> bool {
    value.chars().nth(limit).is_some()
}

/// Stable browser protocol failure category.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "camelCase")]
pub enum ProtocolFailureCode {
    /// The envelope version is unsupported.
    ProtocolMismatch,
    /// The command violates its wire contract.
    InvalidRequest,
    /// The immutable plan could not be installed.
    LoadFailed,
    /// Preparation could not stabilize required resources.
    PrepareFailed,
    /// The requested frame could not be evaluated.
    SeekFailed,
    /// The staged frame could not be confirmed after compositor capture.
    ConfirmFailed,
    /// One or more resources missed the readiness deadline.
    ReadinessTimeout,
    /// The runtime violated an internal invariant.
    Internal,
}

#[cfg(test)]
mod tests {
    use super::{
        BrowserResponse, InvalidProtocolFailure, MAX_FAILURE_MESSAGE_CHARACTERS,
        MAX_PENDING_RESOURCE_CHARACTERS, MAX_PENDING_RESOURCES, ProtocolFailure,
        ProtocolFailureCode,
    };

    #[test]
    fn rejects_an_unsupported_deserialized_protocol_version() {
        let encoded = serde_json::json!({
            "version": 2,
            "requestId": 1,
            "event": { "type": "loaded" },
        });
        assert!(serde_json::from_value::<BrowserResponse>(encoded).is_err());
    }

    #[test]
    fn rejects_an_unsafe_response_frame() {
        let encoded = serde_json::json!({
            "version": 1,
            "requestId": 1,
            "event": {
                "type": "frameReady",
                "frame": 9_007_199_254_740_992_u64,
            },
        });
        assert!(serde_json::from_value::<BrowserResponse>(encoded).is_err());
    }

    #[test]
    fn rejects_the_removed_runtime_state_hash() {
        let encoded = serde_json::json!({
            "version": 1,
            "requestId": 1,
            "event": {
                "type": "frameReady",
                "frame": 15,
                "stateHash": "0".repeat(64),
            },
        });
        assert!(serde_json::from_value::<BrowserResponse>(encoded).is_err());
    }

    #[test]
    fn rejects_a_blank_deserialized_failure_message() {
        let encoded = serde_json::json!({
            "version": 1,
            "requestId": 1,
            "event": {
                "type": "failed",
                "code": "internal",
                "message": " ",
                "pendingResources": [],
            },
        });
        assert!(serde_json::from_value::<BrowserResponse>(encoded).is_err());
    }

    #[test]
    fn rejects_blank_failure_details_at_construction() {
        assert_eq!(
            ProtocolFailure::new(ProtocolFailureCode::Internal, " ", Vec::new(),),
            Err(InvalidProtocolFailure::BlankMessage),
        );
        assert_eq!(
            ProtocolFailure::new(
                ProtocolFailureCode::Internal,
                "rendering failed",
                vec![Box::from("\t")],
            ),
            Err(InvalidProtocolFailure::BlankPendingResource(0)),
        );
    }

    #[test]
    fn rejects_failure_details_outside_the_wire_budget() {
        assert_eq!(
            ProtocolFailure::new(
                ProtocolFailureCode::Internal,
                "x".repeat(MAX_FAILURE_MESSAGE_CHARACTERS + 1),
                Vec::new(),
            ),
            Err(InvalidProtocolFailure::MessageTooLong),
        );
        assert_eq!(
            ProtocolFailure::new(
                ProtocolFailureCode::Internal,
                "rendering failed",
                vec![Box::from("resource"); MAX_PENDING_RESOURCES + 1],
            ),
            Err(InvalidProtocolFailure::TooManyPendingResources),
        );
        assert_eq!(
            ProtocolFailure::new(
                ProtocolFailureCode::Internal,
                "rendering failed",
                vec![
                    "x".repeat(MAX_PENDING_RESOURCE_CHARACTERS + 1)
                        .into_boxed_str()
                ],
            ),
            Err(InvalidProtocolFailure::PendingResourceTooLong(0)),
        );
    }
}
