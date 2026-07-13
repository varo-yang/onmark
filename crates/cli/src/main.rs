#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![deny(clippy::print_stderr, clippy::print_stdout, clippy::unwrap_used)]

mod arguments;
mod assets;
mod bundler;
mod compilation;
mod diagnostic;
mod environment;
mod failure;
mod render;

use std::io;
use std::process::ExitCode;

use clap::Parser as _;

use arguments::{Cli, Command};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Render(args) => finish(render::run(args).await),
    }
}

fn finish(result: Result<render::RenderOutcome, failure::CliError>) -> ExitCode {
    match result {
        Ok(outcome) => outcome.write(),
        Err(error) => {
            let mut stderr = io::stderr().lock();
            failure::write(&mut stderr, &error).unwrap_or(ExitCode::FAILURE)
        }
    }
}
