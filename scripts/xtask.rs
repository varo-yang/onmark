//! Repository-owned generation, evaluation, and release entry point.

use std::env;
use std::error::Error;
use std::fmt;
use std::path::Path;
use std::process::ExitCode;

mod evaluation;
mod release;
mod schema;

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
        Command::Schema(mode) => schema::generate(repository, mode),
        Command::AudioEvaluation => evaluation::grade_audio(repository),
        Command::AudioEnvelopeEvaluation => evaluation::grade_audio_envelope(repository),
        Command::CaptionEvaluation => evaluation::grade_captions(repository),
        Command::HtmlEvaluation => evaluation::grade_html(repository),
        Command::TransitionEvaluation => evaluation::grade_transition(repository),
        Command::VariantEvaluation => evaluation::grade_variant(repository),
        Command::VideoEvaluation => evaluation::grade_video(repository),
        Command::ReleaseSidecar(arguments) => {
            release::run_sidecar(repository, arguments.into_iter()).map_err(Into::into)
        }
        Command::ReleasePrepare(version) => {
            release::prepare_version(repository, &version).map_err(Into::into)
        }
        Command::ReleaseVerify(expected) => {
            release::verify_version(repository, expected.as_deref()).map_err(Into::into)
        }
    }
}

enum Command {
    Schema(schema::GenerationMode),
    AudioEvaluation,
    AudioEnvelopeEvaluation,
    CaptionEvaluation,
    HtmlEvaluation,
    TransitionEvaluation,
    VariantEvaluation,
    VideoEvaluation,
    ReleaseSidecar(Vec<String>),
    ReleasePrepare(String),
    ReleaseVerify(Option<String>),
}

impl Command {
    fn parse(arguments: impl Iterator<Item = String>) -> Result<Self, InvalidCommand> {
        let arguments = arguments.collect::<Vec<_>>();
        match arguments.as_slice() {
            [command] if command == "schema" => Ok(Self::Schema(schema::GenerationMode::Write)),
            [command, flag] if command == "schema" && flag == "--check" => {
                Ok(Self::Schema(schema::GenerationMode::Check))
            }
            [command, subject] if command == "eval" && subject == "audio" => {
                Ok(Self::AudioEvaluation)
            }
            [command, subject] if command == "eval" && subject == "audio-envelope" => {
                Ok(Self::AudioEnvelopeEvaluation)
            }
            [command, subject] if command == "eval" && subject == "captions" => {
                Ok(Self::CaptionEvaluation)
            }
            [command, subject] if command == "eval" && subject == "html" => {
                Ok(Self::HtmlEvaluation)
            }
            [command, subject] if command == "eval" && subject == "transition" => {
                Ok(Self::TransitionEvaluation)
            }
            [command, subject] if command == "eval" && subject == "variant" => {
                Ok(Self::VariantEvaluation)
            }
            [command, subject] if command == "eval" && subject == "video" => {
                Ok(Self::VideoEvaluation)
            }
            [command, artifact, arguments @ ..]
                if command == "release" && artifact == "sidecar" =>
            {
                Ok(Self::ReleaseSidecar(arguments.to_vec()))
            }
            [command, action, version] if command == "release" && action == "prepare" => {
                Ok(Self::ReleasePrepare(version.clone()))
            }
            [command, action] if command == "release" && action == "verify" => {
                Ok(Self::ReleaseVerify(None))
            }
            [command, action, expected] if command == "release" && action == "verify" => {
                Ok(Self::ReleaseVerify(Some(expected.clone())))
            }
            _ => Err(InvalidCommand),
        }
    }
}

#[derive(Debug)]
struct InvalidCommand;

impl fmt::Display for InvalidCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "expected `cargo xtask schema [--check]`, `cargo xtask eval audio`, \
             `cargo xtask eval audio-envelope`, \
             `cargo xtask eval captions`, \
             `cargo xtask eval html`, `cargo xtask eval transition`, \
             `cargo xtask eval variant`, `cargo xtask eval video`, \
             `cargo xtask release prepare <version>`, \
             `cargo xtask release verify [version]`, or \
             `cargo xtask release sidecar <options>`",
        )
    }
}

impl Error for InvalidCommand {}
