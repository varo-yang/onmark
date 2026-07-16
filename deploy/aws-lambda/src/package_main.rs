#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![deny(clippy::print_stderr, clippy::print_stdout, clippy::unwrap_used)]

//! Deterministic release packager for the AWS Lambda adapter.

mod packaging;

use std::env;
use std::io::{self, Write as _};
use std::process::ExitCode;

fn main() -> ExitCode {
    let result = packaging::run(env::args().skip(1));
    finish(result)
}

fn finish(result: Result<(), packaging::PackageError>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let mut stderr = io::stderr().lock();
            let _ = writeln!(stderr, "onmark-aws-lambda-package: {error}");
            ExitCode::FAILURE
        }
    }
}
