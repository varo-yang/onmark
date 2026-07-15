#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![deny(clippy::print_stderr, clippy::print_stdout, clippy::unwrap_used)]

//! Onmark command-line composition root.

mod arguments;
mod assets;
mod bundler;
mod compilation;
mod diagnostic;
mod environment;
mod execution;
mod failure;
mod render;
mod worker;

use std::io;
use std::process::ExitCode;

use clap::Parser as _;

use arguments::{Cli, Command};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Render(args) => render::run(args).await.map(render::RenderOutcome::write),
        Command::Worker(args) => worker::run(args).await.map(worker::WorkerOutcome::write),
    };
    finish(result)
}

fn finish(result: Result<ExitCode, failure::CliError>) -> ExitCode {
    match result {
        Ok(code) => code,
        Err(error) => {
            let mut stderr = io::stderr().lock();
            failure::write(&mut stderr, &error).unwrap_or(ExitCode::FAILURE)
        }
    }
}
