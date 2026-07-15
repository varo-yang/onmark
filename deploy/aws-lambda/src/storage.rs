//! Narrow S3 transport owner shared by download and publication policies.

use std::time::Duration;

use aws_sdk_s3::Client;

use crate::config::S3TransportLimits;

/// S3 boundary shared by input download and immutable artifact publication.
///
/// This is deliberately not a generic object-store abstraction. The sibling
/// modules own S3-specific download and multipart publication semantics.
#[derive(Clone)]
pub(crate) struct S3Storage {
    pub(super) client: Client,
    pub(super) body_idle_timeout: Duration,
}

impl S3Storage {
    pub(crate) const fn new(client: Client, limits: S3TransportLimits) -> Self {
        Self {
            client,
            body_idle_timeout: limits.body_idle_timeout(),
        }
    }
}
