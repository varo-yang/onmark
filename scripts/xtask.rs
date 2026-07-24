//! Repository-owned schema generation and frozen language-evaluation grading.
//!
//! Rust wire types are the source of truth. This binary writes deterministic
//! schemas, delegates TypeScript codec generation, and makes stale output fail.

use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode};

use onmark_aws_lambda::{CaptureInvocation, CaptureResult};
use onmark_core::protocol::{BrowserRequest, BrowserResponse, BundleManifest};
use schemars::{JsonSchema, schema_for};
use serde_json::Value;

mod audio_eval;
mod html_eval;
mod release;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let command = Command::parse(env::args().skip(1))?;
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("scripts is nested directly below the repository root");

    match command {
        Command::Schema(mode) => generate_schemas(repository, mode),
        Command::AudioEvaluation => audio_eval::grade(repository),
        Command::HtmlEvaluation => html_eval::grade(repository),
        Command::ReleaseSidecar(arguments) => {
            release::run_sidecar(repository, arguments.into_iter()).map_err(Into::into)
        }
    }
}

fn generate_schemas(repository: &Path, mode: GenerationMode) -> Result<(), Box<dyn Error>> {
    let schemas = [
        SchemaArtifact::new::<BrowserRequest>(
            "https://onmark.dev/schemas/browser-request-v1.schema.json",
            "browser-request-v1.schema.json",
        )?,
        SchemaArtifact::new::<BrowserResponse>(
            "https://onmark.dev/schemas/browser-response-v1.schema.json",
            "browser-response-v1.schema.json",
        )?,
        SchemaArtifact::new::<BundleManifest>(
            "https://onmark.dev/schemas/bundle-manifest-v1.schema.json",
            "bundle-manifest-v1.schema.json",
        )?,
        SchemaArtifact::new::<CaptureInvocation>(
            "https://onmark.dev/schemas/aws-capture-invocation-v1.schema.json",
            "aws-capture-invocation-v1.schema.json",
        )?,
        SchemaArtifact::new::<CaptureResult>(
            "https://onmark.dev/schemas/aws-capture-result-v1.schema.json",
            "aws-capture-result-v1.schema.json",
        )?,
    ];
    let directory = repository.join("schemas");
    if mode == GenerationMode::Write {
        fs::create_dir_all(&directory)?;
    }

    for schema in schemas {
        schema.publish(&directory, mode)?;
    }
    generate_typescript(repository, mode)
}

fn generate_typescript(repository: &Path, mode: GenerationMode) -> Result<(), Box<dyn Error>> {
    let mut command = ProcessCommand::new("node");
    command
        .arg(repository.join("scripts/protocol-codegen.mjs"))
        .current_dir(repository);
    if mode == GenerationMode::Check {
        command.arg("--check");
    }

    let status = command.status()?;
    if !status.success() {
        return Err(Box::new(CodegenFailed(status)));
    }
    Ok(())
}

enum Command {
    Schema(GenerationMode),
    AudioEvaluation,
    HtmlEvaluation,
    ReleaseSidecar(Vec<String>),
}

impl Command {
    fn parse(arguments: impl Iterator<Item = String>) -> Result<Self, InvalidCommand> {
        let arguments = arguments.collect::<Vec<_>>();
        match arguments.as_slice() {
            [command] if command == "schema" => Ok(Self::Schema(GenerationMode::Write)),
            [command, flag] if command == "schema" && flag == "--check" => {
                Ok(Self::Schema(GenerationMode::Check))
            }
            [command, subject] if command == "eval" && subject == "audio" => {
                Ok(Self::AudioEvaluation)
            }
            [command, subject] if command == "eval" && subject == "html" => {
                Ok(Self::HtmlEvaluation)
            }
            [command, artifact, arguments @ ..]
                if command == "release" && artifact == "sidecar" =>
            {
                Ok(Self::ReleaseSidecar(arguments.to_vec()))
            }
            _ => Err(InvalidCommand),
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum GenerationMode {
    Write,
    Check,
}

#[derive(Debug)]
struct InvalidCommand;

impl fmt::Display for InvalidCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "expected `cargo xtask schema [--check]`, `cargo xtask eval audio`, \
             `cargo xtask eval html`, or `cargo xtask release sidecar <options>`",
        )
    }
}

impl Error for InvalidCommand {}

/// Fully rendered schema kept in memory for byte-exact drift comparison.
struct SchemaArtifact {
    filename: &'static str,
    contents: String,
}

impl SchemaArtifact {
    fn new<T: JsonSchema>(
        id: &'static str,
        filename: &'static str,
    ) -> Result<Self, serde_json::Error> {
        let mut schema = serde_json::to_value(schema_for!(T))?;
        let object = schema
            .as_object_mut()
            .expect("schemars root schemas are JSON objects");
        object.insert(String::from("$id"), Value::String(String::from(id)));
        let mut contents = serde_json::to_string_pretty(&schema)?;
        contents.push('\n');
        Ok(Self { filename, contents })
    }

    fn publish(self, directory: &Path, mode: GenerationMode) -> Result<(), Box<dyn Error>> {
        let path = directory.join(self.filename);
        if mode == GenerationMode::Check {
            let current = fs::read_to_string(&path).map_err(|source| MissingArtifact {
                path: path.clone(),
                source,
            })?;
            if current != self.contents {
                return Err(Box::new(StaleArtifact(path)));
            }
            return Ok(());
        }

        fs::write(path, self.contents)?;
        Ok(())
    }
}

#[derive(Debug)]
struct MissingArtifact {
    path: PathBuf,
    source: std::io::Error,
}

impl fmt::Display for MissingArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "cannot read generated artifact {}",
            self.path.display()
        )
    }
}

impl Error for MissingArtifact {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug)]
struct StaleArtifact(PathBuf);

impl fmt::Display for StaleArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "generated artifact {} is stale; run `cargo xtask schema`",
            self.0.display(),
        )
    }
}

impl Error for StaleArtifact {}

#[derive(Debug)]
struct CodegenFailed(std::process::ExitStatus);

impl fmt::Display for CodegenFailed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "TypeScript protocol generation exited with {}",
            self.0
        )
    }
}

impl Error for CodegenFailed {}
