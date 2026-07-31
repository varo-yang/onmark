//! Bounded browser observations about objectively invalid semantic layout.
//!
//! These values describe captured DOM facts. They do not score aesthetics,
//! infer author intent, or authorize a second browser layout implementation.

use std::error::Error;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use super::BrowserNodeId;

/// Maximum distinct semantic-layout findings returned for one captured frame.
pub const MAX_BROWSER_VISUAL_FINDINGS: usize = 256;

/// Canonically ordered objective layout findings for one captured frame.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct BrowserVisualFindings(
    #[cfg_attr(
        feature = "schema",
        schemars(length(max = MAX_BROWSER_VISUAL_FINDINGS))
    )]
    Vec<BrowserVisualFinding>,
);

impl BrowserVisualFindings {
    /// Returns an empty finding collection.
    #[must_use]
    pub const fn empty() -> Self {
        Self(Vec::new())
    }

    /// Creates one checked collection in browser-node and issue order.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBrowserVisualFindings`] when the collection exceeds
    /// its wire budget or contains duplicate or out-of-order findings.
    pub fn new(findings: Vec<BrowserVisualFinding>) -> Result<Self, InvalidBrowserVisualFindings> {
        if findings.len() > MAX_BROWSER_VISUAL_FINDINGS {
            return Err(InvalidBrowserVisualFindings::TooManyFindings);
        }
        if findings.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(InvalidBrowserVisualFindings::NonCanonicalOrder);
        }
        Ok(Self(findings))
    }

    /// Returns findings in canonical browser-node and issue order.
    #[must_use]
    pub fn findings(&self) -> &[BrowserVisualFinding] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for BrowserVisualFindings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// One semantic node and the objective layout defect observed for it.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserVisualFinding {
    node_id: BrowserNodeId,
    issue: BrowserVisualIssue,
}

impl BrowserVisualFinding {
    /// Creates one finding for an already-projected browser node.
    #[must_use]
    pub const fn new(node_id: BrowserNodeId, issue: BrowserVisualIssue) -> Self {
        Self { node_id, issue }
    }

    /// Returns the unit-local semantic-node identity.
    #[must_use]
    pub const fn node_id(self) -> BrowserNodeId {
        self.node_id
    }

    /// Returns the observed layout defect.
    #[must_use]
    pub const fn issue(self) -> BrowserVisualIssue {
        self.issue
    }
}

/// Closed objective defects measured from an active semantic element.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(rename_all = "camelCase"))]
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BrowserVisualIssue {
    /// The active semantic element occupies no positive rendered area.
    EmptyBox,
    /// Authored horizontal clipping hides part of the overlay's contents.
    ClippedHorizontally,
    /// Authored vertical clipping hides part of the overlay's contents.
    ClippedVertically,
}

/// Reason a browser finding collection cannot cross the runtime boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidBrowserVisualFindings {
    /// The response exceeds the fixed per-frame finding budget.
    TooManyFindings,
    /// Findings are duplicated or not in canonical order.
    NonCanonicalOrder,
}

impl fmt::Display for InvalidBrowserVisualFindings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManyFindings => "browser visual findings exceed the per-frame limit",
            Self::NonCanonicalOrder => {
                "browser visual findings are not in canonical node and issue order"
            }
        })
    }
}

impl Error for InvalidBrowserVisualFindings {}

#[cfg(test)]
mod tests {
    use super::{
        BrowserVisualFinding, BrowserVisualFindings, BrowserVisualIssue,
        InvalidBrowserVisualFindings, MAX_BROWSER_VISUAL_FINDINGS,
    };
    use crate::protocol::BrowserNodeId;

    #[test]
    fn accepts_canonical_findings() {
        let findings = BrowserVisualFindings::new(vec![
            finding(1, BrowserVisualIssue::EmptyBox),
            finding(1, BrowserVisualIssue::ClippedHorizontally),
            finding(2, BrowserVisualIssue::ClippedVertically),
        ])
        .expect("the findings are ordered by node and issue");

        assert_eq!(findings.findings().len(), 3);
    }

    #[test]
    fn rejects_duplicate_or_out_of_order_findings() {
        let duplicated = vec![
            finding(1, BrowserVisualIssue::EmptyBox),
            finding(1, BrowserVisualIssue::EmptyBox),
        ];
        let reversed = vec![
            finding(2, BrowserVisualIssue::EmptyBox),
            finding(1, BrowserVisualIssue::EmptyBox),
        ];

        assert_eq!(
            BrowserVisualFindings::new(duplicated),
            Err(InvalidBrowserVisualFindings::NonCanonicalOrder),
        );
        assert_eq!(
            BrowserVisualFindings::new(reversed),
            Err(InvalidBrowserVisualFindings::NonCanonicalOrder),
        );
    }

    #[test]
    fn rejects_findings_above_the_wire_budget() {
        let findings =
            vec![finding(1, BrowserVisualIssue::EmptyBox); MAX_BROWSER_VISUAL_FINDINGS + 1];

        assert_eq!(
            BrowserVisualFindings::new(findings),
            Err(InvalidBrowserVisualFindings::TooManyFindings),
        );
    }

    fn finding(node_id: u32, issue: BrowserVisualIssue) -> BrowserVisualFinding {
        BrowserVisualFinding::new(BrowserNodeId::new(node_id), issue)
    }
}
