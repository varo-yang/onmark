//! Bounded browser provisioning for the installable desktop release.
//!
//! The provisioner owns network and cache effects. This module accepts only one
//! executable path back into the native render pipeline.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tempfile::TempDir;
use tokio::process::{Child, Command};
use tokio::time;

use crate::input::{self, BoundedReadError};

const BROWSER_PATH_LIMIT: u64 = 8 * 1_024;
const PROVISION_TIMEOUT: Duration = Duration::from_mins(5);

pub(super) async fn provision(
    executable: &Path,
    entry: &Path,
) -> Result<PathBuf, BrowserInstallError> {
    let exchange = BrowserPathExchange::new()?;
    let mut child = spawn(executable, entry, exchange.path())?;
    let status = wait(&mut child).await?;
    if !status.success() {
        return Err(BrowserInstallError::Exit(status));
    }

    exchange.read()
}

fn spawn(executable: &Path, entry: &Path, output: &Path) -> Result<Child, BrowserInstallError> {
    Command::new(executable)
        .arg(entry)
        .arg("--output")
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .map_err(BrowserInstallError::Spawn)
}

async fn wait(child: &mut Child) -> Result<ExitStatus, BrowserInstallError> {
    if let Ok(status) = time::timeout(PROVISION_TIMEOUT, child.wait()).await {
        return status.map_err(BrowserInstallError::Wait);
    }

    let termination = child.kill().await.err();
    Err(BrowserInstallError::Timeout { termination })
}

#[derive(Debug)]
struct BrowserPathExchange {
    _directory: TempDir,
    path: PathBuf,
}

impl BrowserPathExchange {
    fn new() -> Result<Self, BrowserInstallError> {
        let directory = tempfile::tempdir().map_err(BrowserInstallError::Exchange)?;
        let path = directory.path().join("browser-path");
        Ok(Self {
            _directory: directory,
            path,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn read(&self) -> Result<PathBuf, BrowserInstallError> {
        let value = input::read_utf8(&self.path, BROWSER_PATH_LIMIT)
            .map_err(BrowserInstallError::ReadPath)?;
        if value.is_empty() {
            return Err(BrowserInstallError::BlankPath);
        }
        Ok(PathBuf::from(value))
    }
}

#[derive(Debug)]
pub(super) enum BrowserInstallError {
    Exchange(io::Error),
    Spawn(io::Error),
    Wait(io::Error),
    Timeout { termination: Option<io::Error> },
    Exit(ExitStatus),
    ReadPath(BoundedReadError),
    BlankPath,
}

impl fmt::Display for BrowserInstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exchange(_) => {
                formatter.write_str("failed to create the browser provision exchange")
            }
            Self::Spawn(_) => formatter.write_str("failed to start the browser provisioner"),
            Self::Wait(_) => formatter.write_str("failed to wait for the browser provisioner"),
            Self::Timeout { termination: None } => {
                formatter.write_str("browser provisioning exceeded its five-minute deadline")
            }
            Self::Timeout {
                termination: Some(_),
            } => formatter.write_str(
                "browser provisioning exceeded its five-minute deadline and could not be terminated",
            ),
            Self::Exit(status) => write!(formatter, "browser provisioner exited with {status}"),
            Self::ReadPath(_) => {
                formatter.write_str("failed to read the provisioned browser path")
            }
            Self::BlankPath => formatter.write_str("browser provisioner returned a blank path"),
        }
    }
}

impl Error for BrowserInstallError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Exchange(source)
            | Self::Spawn(source)
            | Self::Wait(source)
            | Self::Timeout {
                termination: Some(source),
            } => Some(source),
            Self::ReadPath(source) => Some(source),
            Self::Timeout { termination: None } | Self::Exit(_) | Self::BlankPath => None,
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::Path;

    use super::provision;

    #[tokio::test]
    async fn accepts_one_bounded_path_from_the_provisioner() {
        let directory = tempfile::tempdir().expect("the fixture has a private directory");
        let provisioner = directory.path().join("onmark-browser");
        std::fs::write(
            &provisioner,
            "#!/bin/sh\nprintf '%s' '/browser/chrome-headless-shell' > \"$2\"\n",
        )
        .expect("the provisioner fixture can be written");
        let mut permissions = provisioner
            .metadata()
            .expect("the provisioner fixture has metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&provisioner, permissions)
            .expect("the provisioner fixture can be made executable");

        let browser = provision(Path::new("/bin/sh"), &provisioner)
            .await
            .expect("the provisioner returns one path");

        assert_eq!(
            browser,
            std::path::Path::new("/browser/chrome-headless-shell")
        );
    }
}
