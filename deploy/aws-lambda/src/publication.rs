//! Immutable S3 publication with collision verification and explicit cleanup.
//!
//! A conditional multipart completion elects one artifact. Losing writers
//! verify raw-frame equivalence before reusing the winner rather than trusting
//! object identity alone.

use std::path::Path;

use aws_sdk_s3::Client;
use aws_sdk_s3::error::ProvideErrorMetadata;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use onmark_render::{FrameArtifact, FrameArtifactLimits};
use tokio::io::AsyncReadExt as _;

use crate::deadline::InvocationDeadline;
use crate::download::{DownloadBudget, S3Object};
use crate::error::{DeploymentError, S3ObjectRole};
use crate::invocation::{ArtifactLocation, ObjectPrefix, Publication, artifact_key};
use crate::storage::S3Storage;

const MULTIPART_PART_BYTES: usize = 16 * 1024 * 1024;
const MAX_MULTIPART_PARTS: usize = 10_000;
const MAX_PUBLICATION_ATTEMPTS: usize = 3;

/// Inputs and verification bounds for one immutable artifact publication.
pub(crate) struct ArtifactPublication<'artifact> {
    pub(crate) destination: &'artifact ObjectPrefix,
    pub(crate) artifact: &'artifact FrameArtifact,
    pub(crate) artifact_limits: FrameArtifactLimits,
    pub(crate) workspace: &'artifact Path,
    pub(crate) max_download_bytes: u64,
    pub(crate) deadline: InvocationDeadline,
}

impl ArtifactPublication<'_> {
    fn result(
        &self,
        key: &str,
        artifact: &FrameArtifact,
        status: Publication,
    ) -> (ArtifactLocation, Publication) {
        let location = ArtifactLocation::new(
            self.destination.bucket(),
            key,
            artifact.id(),
            artifact.frames(),
        );
        (location, status)
    }

    fn conflicts(&self, key: &str) -> DeploymentError {
        DeploymentError::PublicationConflicts {
            bucket: self.destination.bucket().into(),
            key: key.into(),
        }
    }
}

impl S3Storage {
    pub(crate) async fn publish(
        &self,
        input: ArtifactPublication<'_>,
    ) -> Result<(ArtifactLocation, Publication), DeploymentError> {
        let key = artifact_key(input.destination, input.artifact.id());

        for _ in 0..MAX_PUBLICATION_ATTEMPTS {
            match self.publish_once(&input, &key).await? {
                PublicationAttempt::Published => {
                    return Ok(input.result(&key, input.artifact, Publication::Published));
                }
                PublicationAttempt::Existing => {
                    return self.reuse_existing(&input, &key).await;
                }
                PublicationAttempt::Retry => {}
            }
        }

        Err(input.conflicts(&key))
    }

    async fn publish_once(
        &self,
        publication: &ArtifactPublication<'_>,
        key: &str,
    ) -> Result<PublicationAttempt, DeploymentError> {
        let deadline = publication.deadline;
        let upload = deadline
            .run(MultipartUpload::start(
                &self.client,
                publication.destination.bucket(),
                key,
            ))
            .await?;
        upload.commit(publication.artifact.path(), deadline).await
    }

    async fn reuse_existing(
        &self,
        publication: &ArtifactPublication<'_>,
        key: &str,
    ) -> Result<(ArtifactLocation, Publication), DeploymentError> {
        let existing = publication
            .deadline
            .run(self.verify_existing(publication, key))
            .await?;
        publication
            .deadline
            .run(FrameArtifact::verify_raw_rgba_equivalence(
                std::slice::from_ref(publication.artifact),
                std::slice::from_ref(&existing),
            ))
            .await?;

        Ok(publication.result(key, &existing, Publication::Reused))
    }

    async fn verify_existing(
        &self,
        publication: &ArtifactPublication<'_>,
        key: &str,
    ) -> Result<FrameArtifact, DeploymentError> {
        let path = publication.workspace.join("existing.onmark-frames");
        let mut budget = DownloadBudget::new(
            S3ObjectRole::ExistingArtifact,
            publication.max_download_bytes,
        );
        let object = S3Object::new(publication.destination.bucket(), key);
        Box::pin(self.download_file(object, &path, None, &mut budget)).await?;
        let artifact = FrameArtifact::open(&path, publication.artifact_limits).await?;
        if artifact.id() != publication.artifact.id() {
            return Err(DeploymentError::ArtifactIdentity {
                expected: publication.artifact.id(),
                actual: artifact.id(),
            });
        }
        artifact.verify().await?;
        Ok(artifact)
    }
}

/// Result of one conditional completion, before bounded conflict retry policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationAttempt {
    Published,
    Existing,
    Retry,
}

/// One S3 multipart upload that must complete or be explicitly aborted.
///
/// S3 has no asynchronous destructor for this resource. This owner therefore
/// makes every incomplete branch release the upload before returning.
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

    async fn commit(
        &self,
        artifact: &Path,
        deadline: InvocationDeadline,
    ) -> Result<PublicationAttempt, DeploymentError> {
        // Abort is not deadline-bound: once S3 has allocated an upload, cleanup
        // remains mandatory even when the invocation's useful work has expired.
        let parts = match deadline.run(self.write(artifact)).await {
            Ok(parts) => parts,
            Err(error) => return self.abort_after(error).await,
        };

        match deadline.run(self.complete(parts)).await {
            Ok(PublicationAttempt::Published) => Ok(PublicationAttempt::Published),
            Ok(attempt) => {
                self.abort().await?;
                Ok(attempt)
            }
            Err(error) => self.abort_after(error).await,
        }
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
            parts.push(self.upload_part(number, &buffer[..read]).await?);
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
        // The SDK owns one copied part while this method reuses the fixed read
        // buffer for the next request, bounding retained upload memory.
        let response = self
            .client
            .upload_part()
            .bucket(self.bucket)
            .key(self.key)
            .upload_id(self.id.as_ref())
            .part_number(number)
            .body(ByteStream::from(bytes.to_vec()))
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

    async fn abort_after(
        &self,
        failure: DeploymentError,
    ) -> Result<PublicationAttempt, DeploymentError> {
        match self.abort().await {
            Ok(()) => Err(failure),
            Err(abort) => Err(DeploymentError::multipart_abort(failure, abort)),
        }
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

fn service_code<E>(error: &aws_sdk_s3::error::SdkError<E>) -> Option<&str>
where
    E: ProvideErrorMetadata,
{
    error.as_service_error().and_then(|error| error.code())
}
