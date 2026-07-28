//! Exact browser layout evidence for admitted native-media composition.
//!
//! Chromium owns CSS evaluation. These wire values retain only the closed,
//! integer geometry that native execution can validate without reconstructing
//! browser layout.

use std::error::Error;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use super::BrowserNodeId;

/// Maximum media placements carried by one browser layout response.
pub const MAX_BROWSER_MEDIA_LAYOUTS: usize = 16;
/// Fixed-point denominator used for browser object-position percentages.
pub const BROWSER_OBJECT_POSITION_SCALE: u32 = 1_000_000;

/// Canonically ordered layout evidence for browser video placements.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct BrowserMediaLayout(
    #[cfg_attr(
        feature = "schema",
        schemars(length(max = MAX_BROWSER_MEDIA_LAYOUTS))
    )]
    Vec<BrowserMediaPlacement>,
);

impl BrowserMediaLayout {
    /// Returns empty evidence when the selected media mode does not measure layout.
    #[must_use]
    pub const fn empty() -> Self {
        Self(Vec::new())
    }

    /// Creates one checked layout collection in browser-node order.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBrowserMediaLayout`] when the collection exceeds its
    /// wire budget or node identities are duplicated or out of order.
    pub fn new(placements: Vec<BrowserMediaPlacement>) -> Result<Self, InvalidBrowserMediaLayout> {
        if placements.len() > MAX_BROWSER_MEDIA_LAYOUTS {
            return Err(InvalidBrowserMediaLayout::TooManyPlacements);
        }
        if placements
            .windows(2)
            .any(|pair| pair[0].node_id() >= pair[1].node_id())
        {
            return Err(InvalidBrowserMediaLayout::NonCanonicalOrder);
        }
        Ok(Self(placements))
    }

    /// Returns placements in canonical browser-node order.
    #[must_use]
    pub fn placements(&self) -> &[BrowserMediaPlacement] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for BrowserMediaLayout {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// One video element's static, axis-aligned browser layout.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserMediaPlacement {
    node_id: BrowserNodeId,
    rectangle: BrowserPixelRectangle,
    object_fit: BrowserObjectFit,
    object_position: BrowserObjectPosition,
}

impl BrowserMediaPlacement {
    /// Creates one already-validated placement.
    #[must_use]
    pub const fn new(
        node_id: BrowserNodeId,
        rectangle: BrowserPixelRectangle,
        object_fit: BrowserObjectFit,
        object_position: BrowserObjectPosition,
    ) -> Self {
        Self {
            node_id,
            rectangle,
            object_fit,
            object_position,
        }
    }

    /// Returns the unit-local video identity.
    #[must_use]
    pub const fn node_id(self) -> BrowserNodeId {
        self.node_id
    }

    /// Returns the pixel-aligned element rectangle.
    #[must_use]
    pub const fn rectangle(self) -> BrowserPixelRectangle {
        self.rectangle
    }

    /// Returns the admitted CSS object-fit mode.
    #[must_use]
    pub const fn object_fit(self) -> BrowserObjectFit {
        self.object_fit
    }

    /// Returns the admitted CSS object-position.
    #[must_use]
    pub const fn object_position(self) -> BrowserObjectPosition {
        self.object_position
    }
}

/// Positive pixel rectangle relative to the output viewport.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserPixelRectangle {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl BrowserPixelRectangle {
    /// Creates one nonempty rectangle.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBrowserPixelRectangle`] when either extent is zero.
    pub const fn new(
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<Self, InvalidBrowserPixelRectangle> {
        if width == 0 || height == 0 {
            return Err(InvalidBrowserPixelRectangle);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    /// Returns the left viewport offset.
    #[must_use]
    pub const fn x(self) -> u32 {
        self.x
    }

    /// Returns the top viewport offset.
    #[must_use]
    pub const fn y(self) -> u32 {
        self.y
    }

    /// Returns the rectangle width.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the rectangle height.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
}

impl<'de> Deserialize<'de> for BrowserPixelRectangle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let rectangle = BrowserPixelRectangleWire::deserialize(deserializer)?;
        Self::new(rectangle.x, rectangle.y, rectangle.width, rectangle.height)
            .map_err(D::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BrowserPixelRectangleWire {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// Closed CSS object-fit subset admitted by native media.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(rename_all = "camelCase"))]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BrowserObjectFit {
    /// Distort source pixels to fill the element rectangle.
    Fill,
    /// Preserve aspect ratio while fitting completely inside the rectangle.
    Contain,
    /// Preserve aspect ratio while covering the complete rectangle.
    Cover,
}

/// Fixed-point CSS object-position, where one million represents 100%.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserObjectPosition {
    #[cfg_attr(
        feature = "schema",
        schemars(range(max = BROWSER_OBJECT_POSITION_SCALE))
    )]
    x: u32,
    #[cfg_attr(
        feature = "schema",
        schemars(range(max = BROWSER_OBJECT_POSITION_SCALE))
    )]
    y: u32,
}

impl BrowserObjectPosition {
    /// Creates one bounded fixed-point position.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBrowserObjectPosition`] when either component lies
    /// outside the inclusive zero-to-100% domain.
    pub const fn new(x: u32, y: u32) -> Result<Self, InvalidBrowserObjectPosition> {
        if x > BROWSER_OBJECT_POSITION_SCALE || y > BROWSER_OBJECT_POSITION_SCALE {
            return Err(InvalidBrowserObjectPosition);
        }
        Ok(Self { x, y })
    }

    /// Returns the horizontal millionth fraction.
    #[must_use]
    pub const fn x(self) -> u32 {
        self.x
    }

    /// Returns the vertical millionth fraction.
    #[must_use]
    pub const fn y(self) -> u32 {
        self.y
    }
}

impl<'de> Deserialize<'de> for BrowserObjectPosition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let position = BrowserObjectPositionWire::deserialize(deserializer)?;
        Self::new(position.x, position.y).map_err(D::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BrowserObjectPositionWire {
    x: u32,
    y: u32,
}

/// Reason a browser media-layout collection was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidBrowserMediaLayout {
    /// The browser returned more placements than the protocol permits.
    TooManyPlacements,
    /// Placement identities are duplicated or not in ascending order.
    NonCanonicalOrder,
}

impl fmt::Display for InvalidBrowserMediaLayout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManyPlacements => "browser media layout exceeds its placement limit",
            Self::NonCanonicalOrder => "browser media layout is not in canonical node order",
        })
    }
}

impl Error for InvalidBrowserMediaLayout {}

/// Reason a browser pixel rectangle was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidBrowserPixelRectangle;

impl fmt::Display for InvalidBrowserPixelRectangle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("browser media rectangle must have positive width and height")
    }
}

impl Error for InvalidBrowserPixelRectangle {}

/// Reason a browser object-position was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidBrowserObjectPosition;

impl fmt::Display for InvalidBrowserObjectPosition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("browser object position must lie between 0% and 100%")
    }
}

impl Error for InvalidBrowserObjectPosition {}

#[cfg(test)]
mod tests {
    use super::{
        BROWSER_OBJECT_POSITION_SCALE, BrowserMediaLayout, BrowserMediaPlacement, BrowserObjectFit,
        BrowserObjectPosition, BrowserPixelRectangle, InvalidBrowserMediaLayout,
        MAX_BROWSER_MEDIA_LAYOUTS,
    };
    use crate::protocol::BrowserNodeId;

    #[test]
    fn accepts_strictly_ordered_layout_evidence() {
        let layout = BrowserMediaLayout::new(vec![placement(1), placement(3)])
            .expect("strictly ordered evidence is canonical");

        assert_eq!(layout.placements().len(), 2);
    }

    #[test]
    fn rejects_duplicate_or_descending_node_identity() {
        assert_eq!(
            BrowserMediaLayout::new(vec![placement(2), placement(2)]),
            Err(InvalidBrowserMediaLayout::NonCanonicalOrder),
        );
        assert_eq!(
            BrowserMediaLayout::new(vec![placement(2), placement(1)]),
            Err(InvalidBrowserMediaLayout::NonCanonicalOrder),
        );
    }

    #[test]
    fn rejects_layout_evidence_beyond_its_wire_budget() {
        let placements = (0..=MAX_BROWSER_MEDIA_LAYOUTS)
            .map(|index| placement(u32::try_from(index).expect("the fixture index fits")))
            .collect();

        assert_eq!(
            BrowserMediaLayout::new(placements),
            Err(InvalidBrowserMediaLayout::TooManyPlacements),
        );
    }

    #[test]
    fn validates_rectangle_and_fixed_point_domains() {
        assert!(BrowserPixelRectangle::new(0, 0, 0, 1).is_err());
        assert!(BrowserPixelRectangle::new(0, 0, 1, 0).is_err());
        assert!(
            BrowserObjectPosition::new(
                BROWSER_OBJECT_POSITION_SCALE,
                BROWSER_OBJECT_POSITION_SCALE,
            )
            .is_ok(),
        );
        assert!(BrowserObjectPosition::new(BROWSER_OBJECT_POSITION_SCALE + 1, 0).is_err(),);
    }

    #[test]
    fn rejects_unknown_nested_layout_fields() {
        let extra_rectangle = serde_json::json!({
            "nodeId": 1,
            "rectangle": {"x": 0, "y": 0, "width": 1, "height": 1, "right": 1},
            "objectFit": "fill",
            "objectPosition": {"x": 0, "y": 0},
        });
        let extra_position = serde_json::json!({
            "nodeId": 1,
            "rectangle": {"x": 0, "y": 0, "width": 1, "height": 1},
            "objectFit": "fill",
            "objectPosition": {"x": 0, "y": 0, "unit": "millionth"},
        });

        assert!(serde_json::from_value::<BrowserMediaPlacement>(extra_rectangle).is_err());
        assert!(serde_json::from_value::<BrowserMediaPlacement>(extra_position).is_err());
    }

    fn placement(node_id: u32) -> BrowserMediaPlacement {
        BrowserMediaPlacement::new(
            BrowserNodeId::new(node_id),
            BrowserPixelRectangle::new(0, 0, 1, 1).expect("the fixture rectangle is valid"),
            BrowserObjectFit::Contain,
            BrowserObjectPosition::new(0, 0).expect("the fixture position is valid"),
        )
    }
}
