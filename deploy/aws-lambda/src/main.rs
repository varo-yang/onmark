#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![deny(clippy::print_stderr, clippy::print_stdout, clippy::unwrap_used)]

//! AWS Lambda process entry point for the Onmark capture worker.

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    Box::pin(onmark_aws_lambda::run()).await
}
