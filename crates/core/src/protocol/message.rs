use std::error::Error;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use super::{BrowserPlan, WireFrame};

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
    /// Release page-owned resources for this session.
    Dispose,
}

/// One versioned event returned by the browser runtime.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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
    /// One requested frame is stable and may be captured.
    FrameReady {
        /// Exact frame now represented by browser state.
        frame: WireFrame,
        /// Canonical hash of runtime-owned state for this frame.
        state_hash: StateHash,
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
    #[cfg_attr(feature = "schema", schemars(regex(pattern = r"\S")))]
    message: Box<str>,
    /// Resources that prevented readiness, in deterministic order.
    #[cfg_attr(feature = "schema", schemars(inner(regex(pattern = r"\S"))))]
    pending_resources: Vec<Box<str>>,
}

impl ProtocolFailure {
    /// Creates one validated failure report.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidProtocolFailure`] when the message or any resource
    /// description contains only whitespace.
    pub fn new(
        code: ProtocolFailureCode,
        message: impl Into<Box<str>>,
        pending_resources: Vec<Box<str>>,
    ) -> Result<Self, InvalidProtocolFailure> {
        let message = message.into();
        if message.trim().is_empty() {
            return Err(InvalidProtocolFailure::BlankMessage);
        }
        if let Some(index) = pending_resources
            .iter()
            .position(|resource| resource.trim().is_empty())
        {
            return Err(InvalidProtocolFailure::BlankPendingResource(index));
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
    /// One pending-resource description contains no visible content.
    BlankPendingResource(usize),
}

impl fmt::Display for InvalidProtocolFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BlankMessage => formatter.write_str("protocol failure message cannot be blank"),
            Self::BlankPendingResource(index) => {
                write!(
                    formatter,
                    "pending resource at index {index} cannot be blank"
                )
            }
        }
    }
}

impl Error for InvalidProtocolFailure {}

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
    /// One or more resources missed the readiness deadline.
    ReadinessTimeout,
    /// The runtime violated an internal invariant.
    Internal,
}

/// Canonical lowercase SHA-256 spelling of runtime-owned frame state.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct StateHash(
    #[cfg_attr(feature = "schema", schemars(regex(pattern = "^[0-9a-f]{64}$")))] Box<str>,
);

impl StateHash {
    /// Parses one canonical lowercase SHA-256 spelling.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidStateHash`] unless the value contains exactly 64
    /// lowercase hexadecimal ASCII digits.
    pub fn parse(value: &str) -> Result<Self, InvalidStateHash> {
        if value.len() != 64 {
            return Err(InvalidStateHash::InvalidLength);
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(InvalidStateHash::NonCanonicalDigit);
        }
        Ok(Self(Box::from(value)))
    }

    /// Returns the canonical hexadecimal spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for StateHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Box::<str>::deserialize(deserializer)?;
        Self::parse(&value).map_err(D::Error::custom)
    }
}

/// Reason a browser state hash spelling is not canonical.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidStateHash {
    /// SHA-256 hexadecimal output must contain exactly 64 digits.
    InvalidLength,
    /// Only lowercase hexadecimal ASCII digits are accepted.
    NonCanonicalDigit,
}

impl fmt::Display for InvalidStateHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidLength => "state hash must contain exactly 64 hexadecimal digits",
            Self::NonCanonicalDigit => "state hash must use lowercase hexadecimal ASCII digits",
        };
        formatter.write_str(message)
    }
}

impl Error for InvalidStateHash {}

#[cfg(test)]
mod tests {
    use super::{
        BrowserResponse, InvalidProtocolFailure, InvalidStateHash, ProtocolFailure,
        ProtocolFailureCode, StateHash,
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
    fn rejects_noncanonical_state_hashes() {
        assert_eq!(
            StateHash::parse("abc"),
            Err(InvalidStateHash::InvalidLength)
        );
        assert_eq!(
            StateHash::parse(&"A".repeat(64)),
            Err(InvalidStateHash::NonCanonicalDigit),
        );
    }
}
