//! CDP request policy for one private browser execution root.
//!
//! The browser may read immutable bundle files and in-memory URLs only. Every
//! ambient network request and every file outside the verified unit root is
//! rejected before Chromium can resolve it.

use std::path::{Path, PathBuf};

use chromiumoxide::cdp::browser_protocol::fetch::{
    ContinueRequestParams, EnableParams, EventRequestPaused, FailRequestParams,
};
use chromiumoxide::cdp::browser_protocol::network::ErrorReason;
use chromiumoxide::error::CdpError;
use chromiumoxide::page::Page;
use futures::StreamExt as _;
use tokio::task::JoinHandle;
use url::Url;

use super::{BrowserError, BrowserErrorKind};

#[derive(Debug)]
pub(super) struct ResourceGuard {
    task: JoinHandle<Result<(), CdpError>>,
}

impl ResourceGuard {
    pub(super) async fn install(page: &Page, root: &Path) -> Result<Self, BrowserError> {
        let policy = ResourcePolicy::new(root).await?;
        let mut requests = page
            .event_listener::<EventRequestPaused>()
            .await
            .map_err(resource_cdp_error)?;
        page.execute(EnableParams::default())
            .await
            .map_err(resource_cdp_error)?;

        let page = page.clone();
        let task = tokio::spawn(async move {
            while let Some(request) = requests.next().await {
                if policy.allows(&request.request.url).await {
                    page.execute(ContinueRequestParams::new(request.request_id.clone()))
                        .await?;
                } else {
                    page.execute(FailRequestParams::new(
                        request.request_id.clone(),
                        ErrorReason::BlockedByClient,
                    ))
                    .await?;
                }
            }
            Ok(())
        });

        Ok(Self { task })
    }

    pub(super) async fn stop(self) -> Result<(), BrowserError> {
        if !self.task.is_finished() {
            self.task.abort();
            let _ = self.task.await;
            return Ok(());
        }

        self.task
            .await
            .map_err(|source| BrowserError::join(BrowserErrorKind::ResourcePolicy, source))?
            .map_err(resource_cdp_error)
    }
}

fn resource_cdp_error(source: CdpError) -> BrowserError {
    BrowserError::cdp_with_diagnostics(BrowserErrorKind::ResourcePolicy, source, None)
}

struct ResourcePolicy {
    root: PathBuf,
}

impl ResourcePolicy {
    async fn new(root: &Path) -> Result<Self, BrowserError> {
        let root = tokio::fs::canonicalize(root)
            .await
            .map_err(|source| BrowserError::io(BrowserErrorKind::ResourcePolicy, source))?;
        Ok(Self { root })
    }

    async fn allows(&self, request: &str) -> bool {
        let Ok(url) = Url::parse(request) else {
            return false;
        };
        match url.scheme() {
            "about" => request == "about:blank",
            "blob" | "data" => true,
            "file" => self.allows_file(url).await,
            _ => false,
        }
    }

    async fn allows_file(&self, url: Url) -> bool {
        let Ok(path) = url.to_file_path() else {
            return false;
        };
        let Ok(path) = tokio::fs::canonicalize(path).await else {
            return false;
        };
        path.starts_with(&self.root)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;
    use url::Url;

    use super::ResourcePolicy;

    #[tokio::test]
    async fn admits_only_files_beneath_the_private_root() {
        let root = tempdir().expect("the private root must be available");
        let outside = tempdir().expect("the outside root must be available");
        let nested = root.path().join("assets");
        fs::create_dir(&nested).expect("the nested fixture directory must be created");
        let admitted = nested.join("frame.svg");
        let rejected = outside.path().join("secret.txt");
        fs::write(&admitted, "<svg/>").expect("the admitted fixture must be written");
        fs::write(&rejected, "secret").expect("the rejected fixture must be written");
        let policy = ResourcePolicy::new(root.path())
            .await
            .expect("the private root must be canonical");

        assert!(policy.allows(file_url(&admitted).as_str()).await);
        assert!(!policy.allows(file_url(&rejected).as_str()).await);
        assert!(!policy.allows("file:///missing-resource.svg").await);
    }

    #[tokio::test]
    async fn rejects_ambient_network_schemes() {
        let root = tempdir().expect("the private root must be available");
        let policy = ResourcePolicy::new(root.path())
            .await
            .expect("the private root must be canonical");

        for request in [
            "https://example.com/image.png",
            "http://127.0.0.1/font.woff2",
            "wss://example.com/socket",
            "ftp://example.com/archive",
        ] {
            assert!(!policy.allows(request).await, "admitted {request}");
        }
        assert!(policy.allows("about:blank").await);
        assert!(policy.allows("data:image/svg+xml,<svg/>").await);
        assert!(policy.allows("blob:file:///presentation").await);
    }

    fn file_url(path: &std::path::Path) -> Url {
        Url::from_file_path(path).expect("the fixture path must be representable as a file URL")
    }
}
