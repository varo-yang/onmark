#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![deny(clippy::print_stderr, clippy::print_stdout, clippy::unwrap_used)]

//! Onmark command-line composition root.

mod arguments;
mod artifact_cache;
mod assets;
mod benchmark;
mod browser_install;
mod bundler;
mod check;
mod compilation;
mod diagnostic;
mod doctor;
mod environment;
mod execution;
mod failure;
mod info;
mod input;
mod inspect;
mod output;
mod progress;
mod render;
mod review;
mod snapshot;
mod subtitle;
mod worker;

use std::io;
use std::process::ExitCode;

use clap::Parser as _;

use arguments::{Cli, Command};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    let result = match cli.command {
        Command::Benchmark(args) => benchmark::run(args, json)
            .await
            .map(benchmark::BenchmarkOutcome::write),
        Command::Check(args) => check::run(args, json).await.map(check::CheckOutcome::write),
        Command::Doctor(args) => doctor::run(args, json)
            .await
            .map(doctor::DoctorOutcome::write),
        Command::Info => Ok(info::run(json).write()),
        Command::Inspect(args) => inspect::run(args, json)
            .await
            .map(inspect::InspectOutcome::write),
        Command::Review(args) => review::run(args, json)
            .await
            .map(review::ReviewOutcome::write),
        Command::Render(args) => render::run(args, json)
            .await
            .map(render::RenderOutcome::write),
        Command::Snapshot(args) => snapshot::run(args, json)
            .await
            .map(snapshot::SnapshotOutcome::write),
        Command::Worker(args) => worker::run(args, json)
            .await
            .map(worker::WorkerOutcome::write),
    };
    finish(result, json)
}

fn finish(result: Result<ExitCode, failure::CliError>, json: bool) -> ExitCode {
    match result {
        Ok(code) => code,
        Err(error) if json => {
            let mut stdout = io::stdout().lock();
            failure::write_json(&mut stdout, &error).unwrap_or(ExitCode::FAILURE)
        }
        Err(error) => {
            let mut stderr = io::stderr().lock();
            failure::write(&mut stderr, &error).unwrap_or(ExitCode::FAILURE)
        }
    }
}
