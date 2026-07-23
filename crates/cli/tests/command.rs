//! Fast command-boundary checks that must run without browser or media tools.

use std::path::Path;
use std::process::Command;

use tempfile::tempdir;

#[test]
fn exposes_render_and_the_gate_three_worker_adapter_without_a_coordinator() {
    let output = Command::new(env!("CARGO_BIN_EXE_onmark"))
        .arg("--help")
        .output()
        .expect("the CLI can be started");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("help output is UTF-8");
    assert!(stdout.contains("render"));
    assert!(stdout.contains("worker"));
    assert!(!stdout.contains("coordinator"));
}

#[test]
fn reports_authored_errors_before_environment_preflight() {
    let directory = tempdir().expect("the fixture directory is available");
    let screenplay = directory.path().join("invalid.onmark");
    std::fs::write(&screenplay, "<film><scene><unknown /></scene></film>")
        .expect("the fixture screenplay is writable");
    let output = Command::new(env!("CARGO_BIN_EXE_onmark"))
        .arg("render")
        .arg(screenplay)
        .env(
            "ONMARK_BROWSER_PROVISIONER",
            "author-errors-must-not-start-this",
        )
        .env(
            "ONMARK_BROWSER_PROVISIONER_ENTRY",
            "author-errors-must-not-read-this",
        )
        .env("PATH", "")
        .output()
        .expect("the CLI can be started");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("diagnostics are UTF-8");
    assert!(stderr.contains("ONM-STRUCT-001"));
    assert!(!stderr.contains("executable"));
    assert!(!stderr.contains("browser provisioner"));
}

#[test]
fn reports_authored_errors_before_inspecting_the_default_stylesheet() {
    let directory = tempdir().expect("the fixture directory is available");
    let screenplay = directory.path().join("film.onmark");
    let stylesheet = directory.path().join("film.css");
    std::fs::write(&screenplay, "<film><unknown /></film>")
        .expect("the fixture screenplay is writable");
    std::fs::create_dir(&stylesheet).expect("the invalid stylesheet directory is writable");

    let output = Command::new(env!("CARGO_BIN_EXE_onmark"))
        .arg("render")
        .arg(screenplay)
        .env("PATH", "")
        .output()
        .expect("the CLI can be started");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("diagnostics are UTF-8");
    assert!(stderr.contains("ONM-STRUCT-001"));
    assert!(!stderr.contains("presentation source"));
}

#[test]
fn reports_subtitle_errors_against_their_own_source_before_preflight() {
    let directory = tempdir().expect("the fixture directory is available");
    let screenplay = directory.path().join("film.onmark");
    let subtitles = directory.path().join("captions.vtt");
    std::fs::write(
        &screenplay,
        "<film><scene><shot duration=\"1s\" /></scene></film>",
    )
    .expect("the fixture screenplay is writable");
    std::fs::write(&subtitles, "WEBVTT\n\n00:01.000 --> 00:00.000\nBad\n")
        .expect("the fixture subtitle is writable");

    let output = Command::new(env!("CARGO_BIN_EXE_onmark"))
        .arg("render")
        .arg(screenplay)
        .arg("--subtitle")
        .arg(&subtitles)
        .env("PATH", "")
        .output()
        .expect("the CLI can be started");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("diagnostics are UTF-8");
    assert!(stderr.contains("ONM-CAPTION-001"));
    assert!(stderr.contains(&subtitles.display().to_string()));
    assert!(!stderr.contains("executable"));
}

#[test]
fn valid_screenplay_does_not_require_an_authored_presentation_entry() {
    let directory = tempdir().expect("the fixture directory is available");
    let screenplay = directory.path().join("film.onmark");
    let destination = directory.path().join("film.mp4");
    std::fs::write(
        &screenplay,
        "<film><scene><shot duration=\"1s\" /></scene></film>",
    )
    .expect("the fixture screenplay is writable");

    let output = Command::new(env!("CARGO_BIN_EXE_onmark"))
        .arg("render")
        .arg(screenplay)
        .arg("--output")
        .arg(destination)
        .env("PATH", "")
        .output()
        .expect("the CLI can be started");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("the failure is UTF-8");
    assert!(stderr.contains("executable"));
    assert!(!stderr.contains("presentation source"));
}

#[test]
fn rejects_a_voice_over_source_without_an_audio_track_before_rendering() {
    let directory = tempdir().expect("the fixture directory is available");
    let screenplay = directory.path().join("invalid.onmark");
    let source = directory.path().join("silent.mp4");
    let destination = directory.path().join("output.mp4");
    std::fs::write(
        &screenplay,
        "<film><scene><shot><vo src=\"silent.mp4\">Narration</vo></shot></scene></film>",
    )
    .expect("the fixture screenplay is writable");
    std::fs::write(&source, "video-only fixture bytes").expect("the fixture asset is writable");

    let executable = Path::new(env!("CARGO_BIN_EXE_onmark"));
    let ffprobe = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ffprobe-video-only");
    let output = Command::new(executable)
        .arg("render")
        .arg(&screenplay)
        .arg("--output")
        .arg(&destination)
        .arg("--browser")
        .arg(executable)
        .arg("--bundler")
        .arg(executable)
        .arg("--ffmpeg")
        .arg(executable)
        .arg("--ffprobe")
        .arg(ffprobe)
        .output()
        .expect("the CLI can be started");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!destination.exists());
    let stderr = String::from_utf8(output.stderr).expect("diagnostics are UTF-8");
    assert!(stderr.contains("ONM-ASSET-002"));
    assert!(stderr.contains("<vo> source \"silent.mp4\" has no audio stream"));
}

#[test]
fn rejects_an_existing_output_before_environment_preflight() {
    let directory = tempdir().expect("the fixture directory is available");
    let screenplay = directory.path().join("film.onmark");
    let destination = directory.path().join("film.mp4");
    std::fs::write(&screenplay, "<film />").expect("the fixture screenplay is writable");
    std::fs::write(&destination, "existing").expect("the fixture output is writable");

    let output = Command::new(env!("CARGO_BIN_EXE_onmark"))
        .arg("render")
        .arg(screenplay)
        .arg("--output")
        .arg(&destination)
        .env("PATH", "")
        .output()
        .expect("the CLI can be started");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("the failure is UTF-8");
    assert!(stderr.contains("already exists"));
    assert!(!stderr.contains("executable"));
}
