#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![deny(clippy::print_stderr, clippy::print_stdout, clippy::unwrap_used)]

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    onmark_aws_lambda::run().await
}
