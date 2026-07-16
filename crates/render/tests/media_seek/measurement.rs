//! Process-tree wall-time and resident-memory measurement for the media spike.

use std::collections::BTreeSet;
use std::future::Future;
use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::sync::oneshot;
use tokio::time::{MissedTickBehavior, interval, timeout};

const PROCESS_DEADLINE: Duration = Duration::from_secs(20);
const MAX_PROCESS_TABLE_BYTES: usize = 1024 * 1024;
const RSS_SAMPLE_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Debug)]
pub(super) struct Measurement<T> {
    pub(super) output: T,
    pub(super) elapsed: Duration,
    baseline_rss_kib: u64,
    peak_rss_kib: u64,
}

impl<T> Measurement<T> {
    pub(super) fn incremental_peak_rss_kib(&self) -> u64 {
        self.peak_rss_kib.saturating_sub(self.baseline_rss_kib)
    }
}

pub(super) async fn measure<T>(future: impl Future<Output = T>) -> Measurement<T> {
    let baseline_rss_kib = process_tree_rss_kib().await;
    let (stop, stopped) = oneshot::channel();
    let sampler = tokio::spawn(sample_peak_rss(stopped, baseline_rss_kib));
    let started = Instant::now();
    let output = future.await;
    let elapsed = started.elapsed();
    let _ = stop.send(());
    let peak_rss_kib = sampler
        .await
        .expect("the resident-memory sampler must complete");

    Measurement {
        output,
        elapsed,
        baseline_rss_kib,
        peak_rss_kib,
    }
}

async fn sample_peak_rss(mut stopped: oneshot::Receiver<()>, baseline_rss_kib: u64) -> u64 {
    let mut peak_rss_kib = baseline_rss_kib;
    let mut samples = interval(RSS_SAMPLE_INTERVAL);
    samples.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = samples.tick() => {
                peak_rss_kib = peak_rss_kib.max(process_tree_rss_kib().await);
            }
            _ = &mut stopped => {
                return peak_rss_kib.max(process_tree_rss_kib().await);
            }
        }
    }
}

async fn process_tree_rss_kib() -> u64 {
    let table = Command::new("ps")
        .args(["-e", "-o", "pid=,ppid=,rss="])
        .output();
    let table = timeout(PROCESS_DEADLINE, table)
        .await
        .expect("the process table must finish before its deadline")
        .expect("ps must report the experiment process tree");
    assert!(
        table.status.success(),
        "process-tree sampling failed: {}",
        String::from_utf8_lossy(&table.stderr),
    );
    assert!(
        table.stdout.len() <= MAX_PROCESS_TABLE_BYTES,
        "the process table exceeds its retained-byte ceiling",
    );

    let processes = parse_process_table(&table.stdout);
    let mut descendants = BTreeSet::from([std::process::id()]);
    loop {
        let previous = descendants.len();
        for &(pid, parent, _) in &processes {
            if descendants.contains(&parent) {
                descendants.insert(pid);
            }
        }
        if descendants.len() == previous {
            break;
        }
    }

    processes
        .iter()
        .filter(|(pid, _, _)| descendants.contains(pid))
        .map(|(_, _, rss_kib)| *rss_kib)
        .sum()
}

fn parse_process_table(table: &[u8]) -> Vec<(u32, u32, u64)> {
    String::from_utf8_lossy(table)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_ascii_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?.parse().ok()?;
            let rss_kib = fields.next()?.parse().ok()?;
            Some((pid, parent, rss_kib))
        })
        .collect()
}
