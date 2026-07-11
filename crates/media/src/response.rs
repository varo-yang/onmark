use std::path::Path;

use onmark_core::model::{AssetMetadata, Duration};
use serde::Deserialize;

use crate::error::ProbeError;

#[derive(Deserialize)]
struct ProbeResponse {
    format: Option<ProbeFormat>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    duration: Option<Box<str>>,
}

pub(crate) fn parse_metadata(path: &Path, bytes: &[u8]) -> Result<AssetMetadata, ProbeError> {
    let response = serde_json::from_slice::<ProbeResponse>(bytes)
        .map_err(|source| ProbeError::invalid_response(path, source))?;
    let Some(duration) = response.format.and_then(|format| format.duration) else {
        return Err(ProbeError::missing_duration(path));
    };
    let duration = Duration::parse(&format!("{duration}s"))
        .map_err(|source| ProbeError::invalid_duration(path, &duration, source))?;

    Ok(AssetMetadata::new(duration))
}
