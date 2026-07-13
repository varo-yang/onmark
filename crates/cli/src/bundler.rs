use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use onmark_core::protocol::BundleManifest;
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncReadExt as _};
use tokio::process::Command;
use tokio::task::JoinError;
use tokio::time::timeout;

const DEADLINE: Duration = Duration::from_mins(2);
const MAX_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;
const READ_BUFFER_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PresentationBundler {
    executable: PathBuf,
}

impl PresentationBundler {
    pub(super) fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    pub(super) async fn bundle(&self, entry: &Path) -> Result<BundleArtifact, BundleError> {
        let root = tempfile::Builder::new()
            .prefix("onmark-bundle-")
            .tempdir()
            .map_err(BundleError::TemporaryDirectory)?;
        let directory = root.path().join("presentation");
        let mut child = self.spawn(entry, &directory)?;
        let stderr = child
            .stderr
            .take()
            .ok_or(BundleError::MissingDiagnosticPipe)?;
        // The reader starts before waiting so a verbose child cannot block on
        // a full pipe. Every exit path below joins this owned task.
        let stderr = DiagnosticReader::spawn(stderr);

        let status = wait_for_exit(&mut child).await;
        // `kill_on_drop` is the last-resort guard when explicit termination
        // failed. Drop it before joining the pipe reader so cleanup cannot
        // wait on a child that still owns stderr.
        drop(child);
        let stderr = stderr.finish().await?;
        let status = status?;
        if !status.success() {
            return Err(BundleError::Failed { status, stderr });
        }

        let manifest_path = directory.join(BundleManifest::FILE_NAME);
        let manifest = tokio::task::spawn_blocking(move || read_manifest(&manifest_path))
            .await
            .map_err(BundleError::ManifestTask)??;
        Ok(BundleArtifact {
            directory,
            manifest,
            root,
        })
    }

    fn spawn(&self, entry: &Path, output: &Path) -> Result<tokio::process::Child, BundleError> {
        let mut command = Command::new(&self.executable);
        command
            .arg("--entry")
            .arg(entry)
            .arg("--output")
            .arg(output)
            .arg("--max-output-bytes")
            .arg(MAX_OUTPUT_BYTES.to_string())
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        command.spawn().map_err(|source| BundleError::Spawn {
            executable: self.executable.clone(),
            source,
        })
    }
}

#[derive(Debug)]
pub(super) struct BundleArtifact {
    directory: PathBuf,
    manifest: BundleManifest,
    root: TempDir,
}

impl BundleArtifact {
    pub(super) fn into_parts(self) -> (PathBuf, BundleManifest, TempDir) {
        (self.directory, self.manifest, self.root)
    }
}

#[derive(Debug)]
pub(super) enum BundleError {
    TemporaryDirectory(io::Error),
    Spawn {
        executable: PathBuf,
        source: io::Error,
    },
    MissingDiagnosticPipe,
    Wait(io::Error),
    DiagnosticRead(io::Error),
    DiagnosticTask(JoinError),
    DiagnosticTimeout,
    ManifestTask(JoinError),
    Terminate(io::Error),
    Timeout,
    Failed {
        status: ExitStatus,
        stderr: CapturedStderr,
    },
    ManifestOpen {
        path: PathBuf,
        source: io::Error,
    },
    ManifestRead {
        path: PathBuf,
        source: io::Error,
    },
    ManifestLimit(PathBuf),
    ManifestDecode {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl fmt::Display for BundleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TemporaryDirectory(_) => {
                formatter.write_str("failed to create a private bundle directory")
            }
            Self::Spawn { executable, .. } => {
                write!(
                    formatter,
                    "failed to start bundler {}",
                    executable.display()
                )
            }
            Self::MissingDiagnosticPipe => {
                formatter.write_str("bundler started without its diagnostic pipe")
            }
            Self::Wait(_) => formatter.write_str("failed to wait for the bundler process"),
            Self::DiagnosticRead(_) => formatter.write_str("failed to read bundler diagnostics"),
            Self::DiagnosticTask(_) => {
                formatter.write_str("bundler diagnostic reader did not finish")
            }
            Self::DiagnosticTimeout => {
                formatter.write_str("bundler diagnostic reader missed its deadline")
            }
            Self::ManifestTask(_) => formatter.write_str("bundle manifest reader did not finish"),
            Self::Terminate(_) => formatter.write_str("failed to terminate the bundler process"),
            Self::Timeout => formatter.write_str("bundler exceeded its two-minute deadline"),
            Self::Failed { status, stderr } => {
                write!(formatter, "bundler exited with {status}")?;
                stderr.fmt_tail(formatter)
            }
            Self::ManifestOpen { path, .. } => {
                write!(
                    formatter,
                    "failed to open bundle manifest {}",
                    path.display()
                )
            }
            Self::ManifestRead { path, .. } => {
                write!(
                    formatter,
                    "failed to read bundle manifest {}",
                    path.display()
                )
            }
            Self::ManifestLimit(path) => {
                write!(
                    formatter,
                    "bundle manifest {} exceeds its byte limit",
                    path.display()
                )
            }
            Self::ManifestDecode { path, .. } => {
                write!(formatter, "bundle manifest {} is invalid", path.display())
            }
        }
    }
}

impl Error for BundleError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::TemporaryDirectory(source)
            | Self::Wait(source)
            | Self::Terminate(source)
            | Self::DiagnosticRead(source)
            | Self::Spawn { source, .. }
            | Self::ManifestOpen { source, .. }
            | Self::ManifestRead { source, .. } => Some(source),
            Self::DiagnosticTask(source) | Self::ManifestTask(source) => Some(source),
            Self::ManifestDecode { source, .. } => Some(source),
            Self::MissingDiagnosticPipe
            | Self::DiagnosticTimeout
            | Self::Timeout
            | Self::Failed { .. }
            | Self::ManifestLimit(_) => None,
        }
    }
}

#[derive(Debug)]
pub(super) struct CapturedStderr {
    bytes: Vec<u8>,
    truncated: bool,
}

struct DiagnosticReader {
    task: Option<tokio::task::JoinHandle<Result<CapturedStderr, io::Error>>>,
}

impl DiagnosticReader {
    fn spawn(stream: impl AsyncRead + Send + Unpin + 'static) -> Self {
        Self {
            task: Some(tokio::spawn(retain_tail(stream, MAX_STDERR_BYTES))),
        }
    }

    async fn finish(mut self) -> Result<CapturedStderr, BundleError> {
        let task = self
            .task
            .take()
            .expect("the reader task is owned until its one terminal finish");
        finish_stderr_before(task, DEADLINE).await
    }
}

impl Drop for DiagnosticReader {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl CapturedStderr {
    fn fmt_tail(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.bytes.is_empty() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&self.bytes);
        write!(formatter, ": {}", text.trim())?;
        if self.truncated {
            formatter.write_str(" [truncated]")?;
        }
        Ok(())
    }
}

async fn retain_tail(
    mut stream: impl AsyncRead + Unpin,
    limit: usize,
) -> Result<CapturedStderr, io::Error> {
    let mut retained = VecDeque::with_capacity(limit);
    let mut buffer = [0; READ_BUFFER_BYTES];
    let mut truncated = false;

    loop {
        let bytes = stream.read(&mut buffer).await?;
        if bytes == 0 {
            break;
        }
        append_tail(&mut retained, &buffer[..bytes], limit, &mut truncated);
    }

    Ok(CapturedStderr {
        bytes: retained.into_iter().collect(),
        truncated,
    })
}

fn append_tail(retained: &mut VecDeque<u8>, bytes: &[u8], limit: usize, truncated: &mut bool) {
    if bytes.len() >= limit {
        retained.clear();
        retained.extend(bytes[bytes.len() - limit..].iter().copied());
        *truncated = true;
        return;
    }
    let overflow = retained
        .len()
        .saturating_add(bytes.len())
        .saturating_sub(limit);
    if overflow > 0 {
        retained.drain(..overflow);
        *truncated = true;
    }
    retained.extend(bytes.iter().copied());
}

async fn finish_stderr_before(
    mut task: tokio::task::JoinHandle<Result<CapturedStderr, io::Error>>,
    deadline: Duration,
) -> Result<CapturedStderr, BundleError> {
    let Ok(result) = timeout(deadline, &mut task).await else {
        task.abort();
        let _ = task.await;
        return Err(BundleError::DiagnosticTimeout);
    };
    result
        .map_err(BundleError::DiagnosticTask)?
        .map_err(BundleError::DiagnosticRead)
}

async fn wait_for_exit(child: &mut tokio::process::Child) -> Result<ExitStatus, BundleError> {
    match timeout(DEADLINE, child.wait()).await {
        Ok(Ok(status)) => Ok(status),
        Ok(Err(source)) => {
            terminate(child).await?;
            Err(BundleError::Wait(source))
        }
        Err(_) => {
            terminate(child).await?;
            Err(BundleError::Timeout)
        }
    }
}

async fn terminate(child: &mut tokio::process::Child) -> Result<(), BundleError> {
    if let Err(source) = child.start_kill() {
        return match child.try_wait() {
            Ok(Some(_)) => Ok(()),
            Ok(None) | Err(_) => Err(BundleError::Terminate(source)),
        };
    }
    child
        .wait()
        .await
        .map(|_| ())
        .map_err(BundleError::Terminate)
}

fn read_manifest(path: &Path) -> Result<BundleManifest, BundleError> {
    let file = File::open(path).map_err(|source| BundleError::ManifestOpen {
        path: path.to_owned(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| BundleError::ManifestRead {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(BundleError::ManifestLimit(path.to_owned()));
    }
    serde_json::from_slice(&bytes).map_err(|source| BundleError::ManifestDecode {
        path: path.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::future;
    use std::time::Duration;

    use super::{BundleError, CapturedStderr, append_tail, finish_stderr_before};

    #[test]
    fn retains_only_the_bounded_diagnostic_tail() {
        let mut retained = VecDeque::new();
        let mut truncated = false;
        append_tail(&mut retained, b"first", 8, &mut truncated);
        append_tail(&mut retained, b"-second", 8, &mut truncated);

        let captured = CapturedStderr {
            bytes: retained.into_iter().collect(),
            truncated,
        };
        assert_eq!(captured.bytes, b"t-second");
        assert!(captured.truncated);
    }

    #[tokio::test]
    async fn aborts_a_diagnostic_reader_that_misses_its_deadline() {
        let task = tokio::spawn(async {
            future::pending::<()>().await;
            Ok(CapturedStderr {
                bytes: Vec::new(),
                truncated: false,
            })
        });

        let error = finish_stderr_before(task, Duration::ZERO)
            .await
            .expect_err("the pending reader must time out");

        assert!(matches!(error, BundleError::DiagnosticTimeout));
    }
}
