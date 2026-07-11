use std::collections::VecDeque;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::error::{ProbeError, Stream};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Owns the direct child until an explicit wait has observed its exit.
///
/// `reaped` is set only after `try_wait` or `wait` returns an exit status, so
/// Drop never mistakes a termination request for completed cleanup.
pub(crate) struct RunningProbe {
    child: Child,
    path: PathBuf,
    reaped: bool,
}

impl RunningProbe {
    pub(crate) fn new(child: Child, path: PathBuf) -> Self {
        Self {
            child,
            path,
            reaped: false,
        }
    }

    fn child(&mut self) -> &mut Child {
        &mut self.child
    }

    fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn wait(&mut self, timeout: Duration) -> Result<ExitStatus, ProbeError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .expect("Ffprobe caps process lifetime at ten minutes");

        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.reaped = true;
                    return Ok(status);
                }
                Ok(None) => {}
                Err(source) => {
                    let error = ProbeError::process_control(&self.path, "poll", source);
                    self.terminate()?;
                    return Err(error);
                }
            }

            if Instant::now() >= deadline {
                self.terminate()?;
                return Err(ProbeError::timeout(&self.path, timeout));
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            thread::sleep(POLL_INTERVAL.min(remaining));
        }
    }

    fn terminate(&mut self) -> Result<(), ProbeError> {
        if self.reaped {
            return Ok(());
        }

        if let Err(source) = self.child.kill() {
            return self.observe_after_failed_kill(source);
        }

        self.reap()
    }

    fn observe_after_failed_kill(&mut self, source: io::Error) -> Result<(), ProbeError> {
        // The child may have exited between the last poll and kill. Observe
        // that race without performing an unbounded wait after a genuine
        // process-control failure.
        match self.child.try_wait() {
            Ok(Some(_)) => {
                self.reaped = true;
                Ok(())
            }
            Ok(None) | Err(_) => Err(ProbeError::process_control(&self.path, "terminate", source)),
        }
    }

    fn reap(&mut self) -> Result<(), ProbeError> {
        self.child
            .wait()
            .map(|_| self.reaped = true)
            .map_err(|source| ProbeError::process_control(&self.path, "reap", source))
    }
}

impl Drop for RunningProbe {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }

        // A successful forced termination makes the following wait finite.
        // If termination itself fails, Drop must not introduce an unbounded wait.
        if self.child.kill().is_ok() {
            let _ = self.child.wait();
        } else {
            let _ = self.child.try_wait();
        }
    }
}

/// Concurrently drains both child pipes while retaining bounded evidence.
pub(crate) struct OutputReaders {
    stdout: JoinHandle<io::Result<Captured>>,
    stderr: JoinHandle<io::Result<Captured>>,
}

impl OutputReaders {
    pub(crate) fn spawn(process: &mut RunningProbe, limit: usize) -> Result<Self, ProbeError> {
        let stdout = process
            .child()
            .stdout
            .take()
            .expect("stdout is piped immediately before readers are started");
        let stderr = process
            .child()
            .stderr
            .take()
            .expect("stderr is piped immediately before readers are started");
        let stdout = match thread::Builder::new()
            .name(String::from("onmark-ffprobe-stdout"))
            .spawn(move || capture(stdout, limit, RetainedEdge::Head))
        {
            Ok(stdout) => stdout,
            Err(source) => {
                let path = process.path().to_owned();
                process.terminate()?;
                return Err(ProbeError::output_reader(&path, Stream::Stdout, source));
            }
        };
        let stderr = match thread::Builder::new()
            .name(String::from("onmark-ffprobe-stderr"))
            .spawn(move || capture(stderr, limit, RetainedEdge::Tail))
        {
            Ok(stderr) => stderr,
            Err(source) => {
                let path = process.path().to_owned();
                process.terminate()?;
                let _ = stdout.join();
                return Err(ProbeError::output_reader(&path, Stream::Stderr, source));
            }
        };

        Ok(Self { stdout, stderr })
    }

    pub(crate) fn finish(self, path: &Path) -> Result<ProcessOutput, ProbeError> {
        // Join both readers before propagating either failure so no reader is
        // detached from the process boundary.
        let stdout = join_output(self.stdout, path, Stream::Stdout);
        let stderr = join_output(self.stderr, path, Stream::Stderr);

        Ok(ProcessOutput {
            stdout: stdout?,
            stderr: stderr?,
        })
    }
}

pub(crate) struct ProcessOutput {
    pub(crate) stdout: Captured,
    pub(crate) stderr: Captured,
}

pub(crate) struct Captured {
    pub(crate) bytes: Vec<u8>,
    pub(crate) truncated: bool,
}

fn capture(mut reader: impl Read, limit: usize, edge: RetainedEdge) -> io::Result<Captured> {
    let mut retained = RetainedBytes::new(limit, edge);
    let mut buffer = [0_u8; 8_192];
    let mut truncated = false;

    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }

        // Reading continues after the retention limit so ffprobe cannot block
        // on a full pipe while retained memory remains fixed.
        truncated |= retained.push(&buffer[..count]);
    }

    Ok(Captured {
        bytes: retained.finish(),
        truncated,
    })
}

fn join_output(
    reader: JoinHandle<io::Result<Captured>>,
    path: &Path,
    stream: Stream,
) -> Result<Captured, ProbeError> {
    reader
        .join()
        .map_err(|_| ProbeError::output_join(path, stream))?
        .map_err(|source| ProbeError::output_io(path, stream, source))
}

#[derive(Clone, Copy)]
enum RetainedEdge {
    /// Preserve the response prefix needed for JSON parsing.
    Head,
    /// Preserve the diagnostic suffix where ffprobe reports its final cause.
    Tail,
}

struct RetainedBytes {
    bytes: VecDeque<u8>,
    limit: usize,
    edge: RetainedEdge,
}

impl RetainedBytes {
    fn new(limit: usize, edge: RetainedEdge) -> Self {
        Self {
            bytes: VecDeque::with_capacity(limit.min(8_192)),
            limit,
            edge,
        }
    }

    fn push(&mut self, chunk: &[u8]) -> bool {
        let truncated = self.bytes.len().saturating_add(chunk.len()) > self.limit;

        match self.edge {
            RetainedEdge::Head => self.push_head(chunk),
            RetainedEdge::Tail => self.push_tail(chunk),
        }

        truncated
    }

    fn push_head(&mut self, chunk: &[u8]) {
        let count = self.limit.saturating_sub(self.bytes.len()).min(chunk.len());
        self.bytes.extend(&chunk[..count]);
    }

    fn push_tail(&mut self, chunk: &[u8]) {
        if chunk.len() >= self.limit {
            self.bytes.clear();
            self.bytes.extend(&chunk[chunk.len() - self.limit..]);
            return;
        }

        let overflow = self
            .bytes
            .len()
            .saturating_add(chunk.len())
            .saturating_sub(self.limit);
        self.bytes.drain(..overflow);
        self.bytes.extend(chunk);
    }

    fn finish(self) -> Vec<u8> {
        self.bytes.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{RetainedBytes, RetainedEdge};

    #[test]
    fn retains_the_bounded_output_head() {
        let mut retained = RetainedBytes::new(5, RetainedEdge::Head);

        assert!(!retained.push(b"abc"));
        assert!(retained.push(b"def"));
        assert_eq!(retained.finish(), b"abcde");
    }

    #[test]
    fn retains_the_bounded_output_tail() {
        let mut retained = RetainedBytes::new(5, RetainedEdge::Tail);

        assert!(!retained.push(b"abc"));
        assert!(retained.push(b"def"));
        assert_eq!(retained.finish(), b"bcdef");
    }
}
