//! Bounded S3 input download and private worker-root materialization.
//!
//! Individual object lengths and the aggregate invocation budget are checked
//! while streaming. No remote length is trusted as an allocation request.

use std::path::Path;
use std::time::Duration;

use aws_sdk_s3::primitives::ByteStream;
use onmark_core::protocol::BundleManifest;
use onmark_render::WorkerCaptureRequest;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};
use tokio::time::timeout;

use crate::error::{DeploymentError, S3ObjectRole};
use crate::invocation::ObjectPrefix;
use crate::storage::S3Storage;

const COPY_BUFFER_BYTES: usize = 64 * 1024;

/// One worker-input tree and the bounds applied while materializing it.
pub(crate) struct InputMaterialization<'input> {
    pub(crate) source: &'input ObjectPrefix,
    pub(crate) request: &'input WorkerCaptureRequest,
    pub(crate) workspace: &'input Path,
    pub(crate) max_files: usize,
    pub(crate) max_bytes: u64,
}

/// Borrowed transport identity kept together across S3 and validation errors.
#[derive(Clone, Copy)]
pub(super) struct S3Object<'object> {
    bucket: &'object str,
    key: &'object str,
}

impl<'object> S3Object<'object> {
    pub(super) const fn new(bucket: &'object str, key: &'object str) -> Self {
        Self { bucket, key }
    }
}

impl S3Storage {
    pub(crate) async fn read_request(
        &self,
        input: &ObjectPrefix,
    ) -> Result<WorkerCaptureRequest, DeploymentError> {
        let key = input.key(WorkerCaptureRequest::FILE_NAME);
        let object = S3Object::new(input.bucket(), &key);
        let bytes = Box::pin(self.read_object(
            S3ObjectRole::WorkerRequest,
            object,
            WorkerCaptureRequest::MAX_JSON_BYTES,
        ))
        .await?;

        serde_json::from_slice(&bytes)
            .map_err(|source| DeploymentError::request(input.bucket(), &key, source))
    }

    pub(crate) async fn materialize_inputs(
        &self,
        input: InputMaterialization<'_>,
    ) -> Result<(), DeploymentError> {
        let assets = input.request.required_asset_ids();
        let files = input.request.bundle().files();
        let file_count = files
            .len()
            .checked_add(assets.len())
            .ok_or_else(|| DeploymentError::input_files(usize::MAX, input.max_files))?;
        if file_count > input.max_files {
            return Err(DeploymentError::input_files(file_count, input.max_files));
        }

        let mut budget = DownloadBudget::new(S3ObjectRole::WorkerInput, input.max_bytes);
        for file in files {
            let relative = format!("{}/{}", WorkerCaptureRequest::BUNDLE_DIRECTORY, file.path());
            let key = input.source.key(&relative);
            let object = S3Object::new(input.source.bucket(), &key);
            let target = input.workspace.join(&relative);
            Box::pin(self.download_file(object, &target, Some(file.bytes()), &mut budget)).await?;
        }
        for id in assets {
            let relative = BundleManifest::asset_path(id);
            let key = input.source.key(&relative);
            let object = S3Object::new(input.source.bucket(), &key);
            let target = input.workspace.join(relative);
            Box::pin(self.download_file(object, &target, None, &mut budget)).await?;
        }

        Ok(())
    }

    async fn read_object(
        &self,
        role: S3ObjectRole,
        object: S3Object<'_>,
        max_bytes: u64,
    ) -> Result<Vec<u8>, DeploymentError> {
        let mut reader = self.get_body(object).await?.into_async_read();
        let mut budget = DownloadBudget::new(role, max_bytes);
        let mut bytes = Vec::new();
        let mut buffer = vec![0; COPY_BUFFER_BYTES];

        loop {
            let read = read_body(&mut reader, &mut buffer, self.body_idle_timeout, object).await?;
            if read == 0 {
                return Ok(bytes);
            }
            budget.reserve(read, object)?;
            bytes.extend_from_slice(&buffer[..read]);
        }
    }

    async fn get_body(&self, object: S3Object<'_>) -> Result<ByteStream, DeploymentError> {
        self.client
            .get_object()
            .bucket(object.bucket)
            .key(object.key)
            .send()
            .await
            .map(|response| response.body)
            .map_err(|source| DeploymentError::s3("read", object.bucket, object.key, source))
    }

    pub(super) async fn download_file(
        &self,
        object: S3Object<'_>,
        target: &Path,
        expected_bytes: Option<u64>,
        budget: &mut DownloadBudget,
    ) -> Result<(), DeploymentError> {
        let body = self.get_body(object).await?;
        let parent = target
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        tokio::fs::create_dir_all(parent).await.map_err(|source| {
            DeploymentError::filesystem("create input directory", parent, source)
        })?;
        let mut output = tokio::fs::File::create(target)
            .await
            .map_err(|source| DeploymentError::filesystem("create input file", target, source))?;
        let mut reader = body.into_async_read();
        let mut bytes = 0_u64;
        let mut buffer = vec![0; COPY_BUFFER_BYTES];

        loop {
            let read = read_body(&mut reader, &mut buffer, self.body_idle_timeout, object).await?;
            if read == 0 {
                break;
            }
            let amount = u64::try_from(read).expect("the fixed copy buffer fits a u64");
            let actual = bytes
                .checked_add(amount)
                .ok_or_else(|| budget.exhausted(object))?;
            require_expected_length(expected_bytes, actual, object)?;
            budget.reserve(read, object)?;
            output.write_all(&buffer[..read]).await.map_err(|source| {
                DeploymentError::filesystem("write input file", target, source)
            })?;
            bytes = actual;
        }
        output
            .flush()
            .await
            .map_err(|source| DeploymentError::filesystem("flush input file", target, source))?;
        require_final_length(expected_bytes, bytes, object)
    }
}

/// Aggregate retained-byte budget shared by every object in one input tree.
pub(super) struct DownloadBudget {
    role: S3ObjectRole,
    remaining: u64,
    limit: u64,
}

impl DownloadBudget {
    pub(super) const fn new(role: S3ObjectRole, limit: u64) -> Self {
        Self {
            role,
            remaining: limit,
            limit,
        }
    }

    fn reserve(&mut self, bytes: usize, object: S3Object<'_>) -> Result<(), DeploymentError> {
        let bytes = u64::try_from(bytes).expect("the fixed copy buffer fits a u64");
        let Some(remaining) = self.remaining.checked_sub(bytes) else {
            return Err(self.exhausted(object));
        };
        self.remaining = remaining;
        Ok(())
    }

    fn exhausted(&self, object: S3Object<'_>) -> DeploymentError {
        DeploymentError::download_limit(self.role, object.bucket, object.key, self.limit)
    }
}

fn require_expected_length(
    expected: Option<u64>,
    actual: u64,
    object: S3Object<'_>,
) -> Result<(), DeploymentError> {
    if let Some(expected) = expected
        && actual > expected
    {
        return Err(DeploymentError::input_length(
            object.bucket,
            object.key,
            expected,
            actual,
        ));
    }
    Ok(())
}

fn require_final_length(
    expected: Option<u64>,
    actual: u64,
    object: S3Object<'_>,
) -> Result<(), DeploymentError> {
    if let Some(expected) = expected
        && actual != expected
    {
        return Err(DeploymentError::input_length(
            object.bucket,
            object.key,
            expected,
            actual,
        ));
    }
    Ok(())
}

async fn read_body(
    reader: &mut (impl AsyncRead + Unpin),
    buffer: &mut [u8],
    idle_timeout: Duration,
    object: S3Object<'_>,
) -> Result<usize, DeploymentError> {
    match timeout(idle_timeout, reader.read(buffer)).await {
        Ok(result) => result
            .map_err(|source| DeploymentError::s3_body("read", object.bucket, object.key, source)),
        Err(_) => Err(DeploymentError::s3_idle_timeout(
            object.bucket,
            object.key,
            idle_timeout,
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{S3Object, read_body};
    use crate::error::DeploymentError;

    #[tokio::test]
    async fn rejects_a_stalled_s3_body_read() {
        let (_writer, mut reader) = tokio::io::duplex(1);
        let mut buffer = [0_u8; 1];
        let error = read_body(
            &mut reader,
            &mut buffer,
            Duration::ZERO,
            S3Object::new("onmark-artifacts", "frame-artifacts/example"),
        )
        .await
        .expect_err("an unreadable S3 body must not wait forever");

        assert!(matches!(error, DeploymentError::S3IdleTimeout { .. }));
    }
}
