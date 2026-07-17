//! Real-environment exit conformance for the Gate-three AWS adapter.

#[path = "remote_partition/aws.rs"]
mod aws;
#[path = "remote_partition/fixture.rs"]
mod fixture;
#[path = "remote_partition/media.rs"]
mod media;

use onmark_render::FrameArtifact;
use tempfile::tempdir;

use self::aws::{AwsConformance, RemoteEnvironment};
use self::fixture::ConformanceFilm;

#[tokio::test]
#[ignore = "requires explicit AWS conformance and FFmpeg environment"]
async fn assembles_two_concurrent_remote_partitions_equivalently_to_one_remote_film() {
    let environment = RemoteEnvironment::read();
    let workspace = tempdir().expect("the remote conformance workspace is available");
    let film = ConformanceFilm::build(workspace.path(), &environment).await;
    let remote = AwsConformance::new(&environment);
    let (whole_case, [first_case, second_case]) = film
        .capture_cases(environment.capture_environment())
        .into_parts();

    let whole = remote.capture(workspace.path(), &whole_case).await;
    // Lambda handles one request per execution environment. Overlapping these
    // calls therefore proves that both partitions can leave the composition
    // process and execute on independent workers.
    let (first, second) = tokio::join!(
        remote.capture(workspace.path(), &first_case),
        remote.capture(workspace.path(), &second_case),
    );
    let partitions = [first, second];
    FrameArtifact::verify_raw_rgba_equivalence(std::slice::from_ref(&whole), &partitions)
        .await
        .expect("remote partitions reproduce the remote whole-film pixel sequence");

    let output = workspace.path().join("assembled.mp4");
    film.assemble(&partitions, &environment, &output).await;
    media::verify_output(&output, &environment).await;
}
