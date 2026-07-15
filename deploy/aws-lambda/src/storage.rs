use std::path::Path;
use std::time::Duration;

use aws_sdk_s3::Client;
use aws_sdk_s3::error::ProvideErrorMetadata;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use onmark_core::protocol::BundleManifest;
use onmark_render::{FrameArtifact, FrameArtifactLimits, WorkerCaptureRequest};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};
use tokio::time::timeout;

use crate::config::S3TransportLimits;
use crate::error::{DeploymentError, S3ObjectRole};
use crate::invocation::{ArtifactLocation, ObjectPrefix, Publication, artifact_key};

const REQUEST_BYTES: u64 = 16 * 1024 * 1024;
const COPY_BUFFER_BYTES: usize = 64 * 1024;
const MULTIPART_PART_BYTES: usize = 16 * 1024 * 1024;
const MAX_MULTIPART_PARTS: usize = 10_000;
const MAX_PUBLICATION_ATTEMPTS: usize = 3;

/// S3 operations owned by the Lambda deployment adapter.
///
/// It is deliberately not a generic object-store abstraction: the conditional
/// multipart semantics below are specific to S3 and are the reason this
/// deployment artifact carries the AWS SDK dependency budget.
#[derive(Clone)]
pub(crate) struct S3Storage {
    client: Client,
    body_idle_timeout: Duration,
}

impl S3Storage {
    pub(crate) const fn new(client: Client, limits: S3TransportLimits) -> Self {
        Self {
            client,
            body_idle_timeout: limits.body_idle_timeout(),
        }
    }

    pub(crate) async fn read_request(
        &self,
        input: &ObjectPrefix,
    ) -> Result<WorkerCaptureRequest, DeploymentError> {
        let key = input.key(WorkerCaptureRequest::FILE_NAME);
        let bytes = Box::pin(self.read_object(
            S3ObjectRole::WorkerRequest,
            input.bucket(),
            &key,
            REQUEST_BYTES,
        ))
        .await?;

        serde_json::from_slice(&bytes)
            .map_err(|source| DeploymentError::request(input.bucket(), &key, source))
    }

    pub(crate) async fn materialize_inputs(
        &self,
        input: &ObjectPrefix,
        request: &WorkerCaptureRequest,
        workspace: &Path,
        max_files: usize,
        max_bytes: u64,
    ) -> Result<(), DeploymentError> {
        let asset_ids = request.required_asset_ids();
        let files = request.bundle().files();
        let file_count = files
            .len()
            .checked_add(asset_ids.len())
            .ok_or_else(|| DeploymentError::input_files(usize::MAX, max_files))?;
        if file_count > max_files {
            return Err(DeploymentError::input_files(file_count, max_files));
        }

        let mut budget = DownloadBudget::new(S3ObjectRole::WorkerInput, max_bytes);
        for file in files {
            let key = input.key(&format!(
                "{}/{}",
                WorkerCaptureRequest::BUNDLE_DIRECTORY,
                file.path()
            ));
            let target = workspace
                .join(WorkerCaptureRequest::BUNDLE_DIRECTORY)
                .join(file.path());
            Box::pin(self.download_file(
                input.bucket(),
                &key,
                &target,
                Some(file.bytes()),
                &mut budget,
            ))
            .await?;
        }
        for id in asset_ids {
            let relative = BundleManifest::asset_path(id);
            let key = input.key(&relative);
            let target = workspace.join(relative);
            Box::pin(self.download_file(input.bucket(), &key, &target, None, &mut budget)).await?;
        }

        Ok(())
    }

    pub(crate) async fn publish(
        &self,
        destination: &ObjectPrefix,
        artifact: &FrameArtifact,
        artifact_limits: FrameArtifactLimits,
        workspace: &Path,
        max_download_bytes: u64,
    ) -> Result<(ArtifactLocation, Publication), DeploymentError> {
        let id = artifact.id();
        let key = artifact_key(destination, id);
        let location =
            ArtifactLocation::new(destination.bucket(), key.as_str(), id, artifact.frames());

        for _ in 0..MAX_PUBLICATION_ATTEMPTS {
            match self
                .publish_once(destination.bucket(), &key, artifact.path())
                .await?
            {
                PublicationAttempt::Published => return Ok((location, Publication::Published)),
                PublicationAttempt::Existing => {
                    let existing = Box::pin(self.verify_existing(
                        destination.bucket(),
                        &key,
                        id,
                        artifact_limits,
                        workspace,
                        max_download_bytes,
                    ))
                    .await?;
                    FrameArtifact::verify_raw_rgba_equivalence(
                        std::slice::from_ref(artifact),
                        std::slice::from_ref(&existing),
                    )
                    .await?;
                    let location = ArtifactLocation::new(
                        destination.bucket(),
                        key.as_str(),
                        id,
                        existing.frames(),
                    );
                    return Ok((location, Publication::Reused));
                }
                PublicationAttempt::Retry => {}
            }
        }

        Err(DeploymentError::PublicationConflicts {
            bucket: destination.bucket().into(),
            key: key.into(),
        })
    }

    async fn read_object(
        &self,
        role: S3ObjectRole,
        bucket: &str,
        key: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, DeploymentError> {
        let mut reader = self.get_body(bucket, key).await?.into_async_read();
        let mut budget = DownloadBudget::new(role, max_bytes);
        let mut bytes = Vec::new();
        let mut buffer = vec![0; COPY_BUFFER_BYTES];

        loop {
            let read = read_s3_body(
                &mut reader,
                &mut buffer,
                self.body_idle_timeout,
                bucket,
                key,
            )
            .await?;
            if read == 0 {
                return Ok(bytes);
            }
            budget.reserve(read, bucket, key)?;
            bytes.extend_from_slice(&buffer[..read]);
        }
    }

    async fn get_body(&self, bucket: &str, key: &str) -> Result<ByteStream, DeploymentError> {
        self.client
            .get_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map(|response| response.body)
            .map_err(|source| DeploymentError::s3("read", bucket, key, source))
    }

    async fn download_file(
        &self,
        bucket: &str,
        key: &str,
        target: &Path,
        expected_bytes: Option<u64>,
        budget: &mut DownloadBudget,
    ) -> Result<(), DeploymentError> {
        let body = self.get_body(bucket, key).await?;
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
            let read = read_s3_body(
                &mut reader,
                &mut buffer,
                self.body_idle_timeout,
                bucket,
                key,
            )
            .await?;
            if read == 0 {
                break;
            }
            let amount = u64::try_from(read).expect("the fixed copy buffer fits a u64");
            let actual = bytes
                .checked_add(amount)
                .ok_or_else(|| budget.exhausted(bucket, key))?;
            if let Some(expected) = expected_bytes
                && actual > expected
            {
                return Err(DeploymentError::input_length(bucket, key, expected, actual));
            }
            budget.reserve(read, bucket, key)?;
            bytes = actual;
            output.write_all(&buffer[..read]).await.map_err(|source| {
                DeploymentError::filesystem("write input file", target, source)
            })?;
        }
        output
            .flush()
            .await
            .map_err(|source| DeploymentError::filesystem("flush input file", target, source))?;

        if let Some(expected) = expected_bytes
            && bytes != expected
        {
            return Err(DeploymentError::input_length(bucket, key, expected, bytes));
        }
        Ok(())
    }

    async fn publish_once(
        &self,
        bucket: &str,
        key: &str,
        artifact: &Path,
    ) -> Result<PublicationAttempt, DeploymentError> {
        let upload = MultipartUpload::start(&self.client, bucket, key).await?;
        let parts = match upload.write(artifact).await {
            Ok(parts) => parts,
            Err(error) => return upload.abort_after_failure(error).await,
        };
        match upload.complete(parts).await {
            Ok(PublicationAttempt::Published) => Ok(PublicationAttempt::Published),
            Ok(attempt) => {
                upload.abort().await?;
                Ok(attempt)
            }
            Err(error) => upload.abort_after_failure(error).await,
        }
    }

    async fn verify_existing(
        &self,
        bucket: &str,
        key: &str,
        expected_id: onmark_render::FrameArtifactId,
        limits: FrameArtifactLimits,
        workspace: &Path,
        max_bytes: u64,
    ) -> Result<FrameArtifact, DeploymentError> {
        let path = workspace.join("existing.onmark-frames");
        let mut budget = DownloadBudget::new(S3ObjectRole::ExistingArtifact, max_bytes);
        Box::pin(self.download_file(bucket, key, &path, None, &mut budget)).await?;
        let artifact = FrameArtifact::open(&path, limits).await?;
        if artifact.id() != expected_id {
            return Err(DeploymentError::ArtifactIdentity {
                expected: expected_id,
                actual: artifact.id(),
            });
        }
        artifact.verify().await?;
        Ok(artifact)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationAttempt {
    Published,
    Existing,
    Retry,
}

/// One S3 multipart upload that must either complete or be explicitly aborted.
///
/// S3 does not give this resource an async destructor. Keeping its lifecycle
/// in one type makes every incomplete branch visibly release the upload.
struct MultipartUpload<'client> {
    client: &'client Client,
    bucket: &'client str,
    key: &'client str,
    id: Box<str>,
}

impl<'client> MultipartUpload<'client> {
    async fn start(
        client: &'client Client,
        bucket: &'client str,
        key: &'client str,
    ) -> Result<Self, DeploymentError> {
        let response = client
            .create_multipart_upload()
            .bucket(bucket)
            .key(key)
            .content_type("application/vnd.onmark.frame-artifact")
            .send()
            .await
            .map_err(|source| {
                DeploymentError::s3("start multipart upload for", bucket, key, source)
            })?;
        let id = response.upload_id().ok_or_else(|| {
            DeploymentError::multipart_response(
                "start multipart upload for",
                bucket,
                key,
                "upload id",
            )
        })?;

        Ok(Self {
            client,
            bucket,
            key,
            id: id.into(),
        })
    }

    async fn write(&self, artifact: &Path) -> Result<Vec<CompletedPart>, DeploymentError> {
        let mut input = tokio::fs::File::open(artifact).await.map_err(|source| {
            DeploymentError::filesystem("open frame artifact", artifact, source)
        })?;
        let mut parts = Vec::new();
        let mut buffer = vec![0; MULTIPART_PART_BYTES];

        loop {
            let read = input.read(&mut buffer).await.map_err(|source| {
                DeploymentError::filesystem("read frame artifact", artifact, source)
            })?;
            if read == 0 {
                return Ok(parts);
            }

            let number = self.next_part_number(parts.len())?;
            let part = self.upload_part(number, &buffer[..read]).await?;
            parts.push(part);
        }
    }

    fn next_part_number(&self, parts: usize) -> Result<i32, DeploymentError> {
        if parts == MAX_MULTIPART_PARTS {
            return Err(DeploymentError::multipart_response(
                "upload",
                self.bucket,
                self.key,
                "supported multipart part count",
            ));
        }

        i32::try_from(parts + 1).map_err(|_| {
            DeploymentError::multipart_response(
                "upload",
                self.bucket,
                self.key,
                "valid multipart part number",
            )
        })
    }

    async fn upload_part(
        &self,
        number: i32,
        bytes: &[u8],
    ) -> Result<CompletedPart, DeploymentError> {
        // `ByteStream` owns this part while the caller reuses its fixed read
        // buffer for the next S3 request, bounding retained upload memory.
        let body = ByteStream::from(bytes.to_vec());
        let response = self
            .client
            .upload_part()
            .bucket(self.bucket)
            .key(self.key)
            .upload_id(self.id.as_ref())
            .part_number(number)
            .body(body)
            .send()
            .await
            .map_err(|source| {
                DeploymentError::s3("upload part for", self.bucket, self.key, source)
            })?;
        let e_tag = response.e_tag().ok_or_else(|| {
            DeploymentError::multipart_response(
                "upload part for",
                self.bucket,
                self.key,
                "part ETag",
            )
        })?;

        Ok(CompletedPart::builder()
            .part_number(number)
            .e_tag(e_tag)
            .build())
    }

    async fn complete(
        &self,
        parts: Vec<CompletedPart>,
    ) -> Result<PublicationAttempt, DeploymentError> {
        let upload = CompletedMultipartUpload::builder()
            .set_parts(Some(parts))
            .build();
        let result = self
            .client
            .complete_multipart_upload()
            .bucket(self.bucket)
            .key(self.key)
            .upload_id(self.id.as_ref())
            .multipart_upload(upload)
            .if_none_match("*")
            .send()
            .await;

        match result {
            Ok(_) => Ok(PublicationAttempt::Published),
            Err(source) if service_code(&source) == Some("PreconditionFailed") => {
                Ok(PublicationAttempt::Existing)
            }
            Err(source) if service_code(&source) == Some("ConditionalRequestConflict") => {
                Ok(PublicationAttempt::Retry)
            }
            Err(source) => Err(DeploymentError::s3(
                "complete multipart upload for",
                self.bucket,
                self.key,
                source,
            )),
        }
    }

    async fn abort_after_failure(
        &self,
        failure: DeploymentError,
    ) -> Result<PublicationAttempt, DeploymentError> {
        self.abort().await?;
        Err(failure)
    }

    async fn abort(&self) -> Result<(), DeploymentError> {
        self.client
            .abort_multipart_upload()
            .bucket(self.bucket)
            .key(self.key)
            .upload_id(self.id.as_ref())
            .send()
            .await
            .map_err(|source| {
                DeploymentError::s3("abort multipart upload for", self.bucket, self.key, source)
            })?;
        Ok(())
    }
}

struct DownloadBudget {
    role: S3ObjectRole,
    remaining: u64,
    limit: u64,
}

impl DownloadBudget {
    const fn new(role: S3ObjectRole, limit: u64) -> Self {
        Self {
            role,
            remaining: limit,
            limit,
        }
    }

    fn reserve(&mut self, bytes: usize, bucket: &str, key: &str) -> Result<(), DeploymentError> {
        let bytes = u64::try_from(bytes).expect("the fixed copy buffer fits a u64");
        let Some(remaining) = self.remaining.checked_sub(bytes) else {
            return Err(self.exhausted(bucket, key));
        };
        self.remaining = remaining;
        Ok(())
    }

    fn exhausted(&self, bucket: &str, key: &str) -> DeploymentError {
        DeploymentError::download_limit(self.role, bucket, key, self.limit)
    }
}

fn service_code<E>(error: &aws_sdk_s3::error::SdkError<E>) -> Option<&str>
where
    E: ProvideErrorMetadata,
{
    error.as_service_error().and_then(|error| error.code())
}

async fn read_s3_body(
    reader: &mut (impl AsyncRead + Unpin),
    buffer: &mut [u8],
    idle_timeout: Duration,
    bucket: &str,
    key: &str,
) -> Result<usize, DeploymentError> {
    match timeout(idle_timeout, reader.read(buffer)).await {
        Ok(result) => {
            result.map_err(|source| DeploymentError::s3_body("read", bucket, key, source))
        }
        Err(_) => Err(DeploymentError::s3_idle_timeout(bucket, key, idle_timeout)),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::read_s3_body;
    use crate::error::DeploymentError;

    #[tokio::test]
    async fn rejects_a_stalled_s3_body_read() {
        let (_writer, mut reader) = tokio::io::duplex(1);
        let mut buffer = [0_u8; 1];
        let error = read_s3_body(
            &mut reader,
            &mut buffer,
            Duration::ZERO,
            "onmark-artifacts",
            "frame-artifacts/example",
        )
        .await
        .expect_err("an unreadable S3 body must not wait forever");

        assert!(matches!(error, DeploymentError::S3IdleTimeout { .. }));
    }
}
