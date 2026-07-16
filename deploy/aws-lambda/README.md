# `onmark-aws-lambda`

`onmark-aws-lambda` is the Gate-three AWS deployment adapter for one
immutable Onmark worker frame artifact. It owns Lambda event decoding, S3 input
materialization, conditional artifact publication, and the Lambda process
boundary. It calls the existing `onmark-render` worker executor; it never
accepts a screenplay or recompiles authored input.

This package is a distinct Rust release artifact because it runs in Lambda and
owns the AWS SDK dependency budget. `onmark-core` remains free of AWS types.

## Current boundary

The handler accepts an
[`aws-capture-invocation-v1` schema](../../schemas/aws-capture-invocation-v1.schema.json)
payload:

```json
{
  "version": 1,
  "input": {
    "bucket": "onmark-worker-inputs",
    "prefix": "captures/film-42/unit-0"
  }
}
```

The input prefix must contain the portable worker layout already used by
`onmark worker capture`:

```text
request.json
bundle/<every file named by request.bundle>
assets/sha256/<every frozen visual asset named by request.browserPlan>
```

`request.json` includes the locked capture-environment identity. The Lambda
deployment must supply the same identity; a mismatch fails before the worker
starts.
The structured result follows the checked-in
[`aws-capture-result-v1` schema](../../schemas/aws-capture-result-v1.schema.json).
These schemas are generated from Rust and checked by `cargo xtask schema
--check`. There is intentionally no TypeScript AWS SDK yet: no TypeScript
caller exists, and inventing one would prebuild a Gate-three coordinator.

The Lambda configuration is deployment-owned:

- `ONMARK_ARTIFACT_BUCKET` — required output bucket;
- `ONMARK_ARTIFACT_PREFIX` — optional canonical output prefix;
- `ONMARK_BROWSER_BINARY` — an already-expanded pinned
  `chrome-headless-shell` executable;
- `ONMARK_BROWSER_ARCHIVE` and `ONMARK_BROWSER_ARCHIVE_SHA256` — the preferred
  Lambda ZIP form: one zstd-compressed tar capture environment and its
  canonical `sha256:<lowercase-hex>` identity;
- `ONMARK_CAPTURE_ENVIRONMENT` — required `sha256:<lowercase-hex>` identity
  of the complete pixel-affecting deployment environment.

Exactly one browser form is required. An invocation cannot choose the output
bucket, browser representation, capture environment, or resource limits.

## Chromium isolation policy

Local and ordinary product launches retain Chromium's own sandbox and standard
multi-process topology. This adapter explicitly selects
`BrowserLaunchPolicy::isolated_worker()` because Lambda owns the outer process
isolation and cannot host Chromium's zygote or GPU subprocess sandboxes. The
policy uses `single-process`, `no-zygote`, and in-process SwiftShader; it does
not disable the graphics stack. It is never an automatic fallback, and an
invocation cannot choose it. The deployment's capture-environment identity
must include the complete launch policy.

A disposable arm64 Debian container has exercised the real `BrowserSession`
under a non-root user, read-only root filesystem, writable `/tmp`, no Linux
capabilities, no network, no privilege escalation, and bounded memory and PIDs.
Chromium could not create its internal namespace sandbox in that boundary even
with the distribution sandbox helper installed; the isolated-worker policy
completed launch, non-empty PNG capture, and shutdown.

## Browser packaging and cold start

Lambda starts polling the Runtime API before it touches a compressed browser
payload. The first valid invocation verifies and expands that payload in a
blocking worker, then retains the private installation for the life of the
execution environment. Warm invocations reuse it without another archive read.
This ordering is part of the deployment contract: browser preparation belongs
to the bounded invocation, not Lambda's ten-second initialization window.

The archive boundary accepts at most 128 MiB compressed, 64 entries, and 320
MiB expanded. It rejects absolute paths, parent traversal, duplicate entries,
links, special files, digest drift, and a non-executable shell. A failed
verification drops the private staging root. When the payload contains a
`fonts/` directory, the adapter creates a private font cache and an absolute
fontconfig file. `onmark-render` passes that file, the sidecar library path, and
the SwiftShader manifest only to the Chromium child; it does not mutate the
Lambda process environment.

The AWS package owns a separate deterministic operator tool. Build the Rust
bootstrap for Linux arm64 first, then package it with one expanded browser root:

```sh
cargo run --locked \
  --package onmark-aws-lambda \
  --bin onmark-aws-lambda-package \
  --no-default-features \
  --features package \
  -- \
  --bootstrap target/lambda/onmark-aws-lambda/bootstrap \
  --browser-root /path/to/locked-browser \
  --output dist/onmark-aws-lambda
```

The output directory must not already exist. It contains:

```text
onmark-aws-lambda.zip
manifest.json
```

The builder writes through a private sibling directory, so a normally
completed run never exposes a partial package. Portable directory rename does
not make the preceding absence check a cross-process no-clobber transaction;
operators must assign one output directory to one packaging process.

The ZIP carries the executable `bootstrap` and `browser.tar.zst`. The builder
normalizes browser traversal order, tar metadata, zstd settings, and ZIP entry
metadata; it rejects links and special files and reuses the adapter's runtime
archive limits. Both executables must be Linux arm64 ELF files, and their
combined unzipped package payload may not exceed 240 MiB, leaving headroom
below Lambda's 250 MiB ceiling. The manifest records canonical SHA-256
identities for both inputs and the final ZIP. Its `browserArchive.sha256` value
configures `ONMARK_BROWSER_ARCHIVE_SHA256`, while `captureEnvironment`
configures `ONMARK_CAPTURE_ENVIRONMENT`. `ONMARK_BROWSER_ARCHIVE` points to
`/var/task/browser.tar.zst` after deployment.

Two runs over byte-identical locked inputs produce byte-identical ZIP and
manifest files. This guarantee begins at the prebuilt bootstrap and expanded
browser root; use a pinned Linux arm64 builder such as Cargo Lambda to produce
the bootstrap. The packager does not claim that arbitrary host toolchains
cross-compile identical ELF binaries.

This command remains intentionally AWS-specific. Future GCP, container, or
multi-machine adapters consume the same portable worker request and frame
artifact, but own their own SDK, transport, and release package. They do not
inherit Lambda ZIP or S3 semantics.

A real arm64 Lambda experiment used a 92.4 MB (88.1 MiB) ZIP containing a
23 MiB Rust bootstrap and a 79 MiB compressed Chromium v149, SwiftShader, and
Open Sans environment. At 4,096 MiB, three independent cold environments
captured, verified, and conditionally published or reused the same 30 lossless
320×180 frames:

| sample | init | prepare browser | capture artifact | complete invocation | peak memory |
| --- | ---: | ---: | ---: | ---: | ---: |
| cold 1 | 175 ms | 1,673 ms | 852 ms | 3,005 ms | 455 MB |
| cold 2 | 246 ms | 1,097 ms | 732 ms | 2,277 ms | 457 MB |
| cold 3 | 216 ms | 1,683 ms | 906 ms | 3,069 ms | 457 MB |
| warm reuse | — | 0 ms | 774 ms | 1,325 ms | 457 MB |

The same artifact also located the memory/CPU knee with one independent cold
environment at each lower setting:

| configured memory | prepare browser | capture artifact | complete invocation | peak memory |
| ---: | ---: | ---: | ---: | ---: |
| 4,096 MiB | 1,673 ms | 852 ms | 3,005 ms | 455 MB |
| 2,048 MiB | 1,688 ms | 923 ms | 3,069 ms | 454 MB |
| 1,024 MiB | 2,906 ms | 1,705 ms | 5,080 ms | 451 MB |

Two GiB is the measured latency/cost knee for this fixture: four GiB adds no
material speed, while one GiB remains viable when lower GB-seconds matter more
than a roughly three-second cold response. A production default still needs
media-bearing and higher-resolution measurements.

A second experiment measured the capture path rather than browser delivery. It
used one 1,920×1,080 H.264 fixture, 60 output frames at 30 fps, the same compact
browser payload, and one immutable artifact identity across memory tiers. Each
current capture produced the same 60 canonical raw-RGBA fingerprints across
independent cold environments. The values below are individual samples, not a
statistical production benchmark:

| configured memory | warm capture | cold invocation | warm GB-seconds | peak memory |
| ---: | ---: | ---: | ---: | ---: |
| 2,048 MiB | 22.07 s | 20.69–25.55 s | 47.11 | 600–603 MB |
| 4,096 MiB | 13.00 s | 16.37 s | 58.72 | 610–616 MB |
| 8,192 MiB | 7.91 s | 10.82 s | 73.46 | 609–612 MB |

Actual memory stayed near 600 MiB; the configured tier mainly bought Lambda CPU.
Two GiB minimized measured GB-seconds, eight GiB minimized latency, and four GiB
sat between them. At eight GiB, aggregate time across the 60 frames was 2.96
seconds in runtime staging and media seek, 3.83 seconds in BeginFrame screenshot
readback, 0.79 seconds in PNG decoding and raw-RGBA hashing, and under 0.2 seconds
in confirmation plus artifact writes. The next performance work therefore
targets decoded-media seeking and PNG screenshot transport, not handler control
flow or S3 publication.

An earlier 66-second observation is not a cold-start baseline. The pre-fix frame
handshake stalled until its deadline, and the AWS CLI's default 60-second read
timeout retried while the first invocation was still running. Direct synchronous
experiments must disable client retries and use a read timeout longer than the
worker deadline.

The packaging choice came from a controlled failure, not an image-size guess.
An identical 249 MiB expanded browser copied into a fresh container-image layer
made `capture_artifact` take 30.9 seconds. Nesting a 79 MiB compressed payload
inside a fresh image layer reduced bytes but still exhausted Lambda's init
window when extraction happened before Runtime API polling. ZIP delivery plus
invocation-owned preparation removed both failure modes. A future Lambda Layer
may version the stable browser payload separately, but it is not required for
this performance path and is not introduced without a release consumer.

These measurements prove one locked experimental environment. They do not
generalize across resolutions, media-heavy presentations, or AWS regions. The
reviewed packager now replaces the experiment's hand-built ZIP procedure; a
published release workflow is still intentionally absent.

## Publication and limits

The handler follows one linear path:

```text
decode invocation
→ download the bounded worker input
→ materialize the verified Render Unit
→ prepare or reuse the verified browser installation
→ capture and verify one frame artifact
→ conditionally publish it to S3
→ return its immutable location
```

S3 object keys are derived from the worker request's frame-artifact identity.
The adapter completes a multipart upload with `If-None-Match: *`. A `412`
fetches, fully verifies, and compares the existing raw-RGBA sequence with this
capture before returning `reused`; a transient `409` retries the conditional
publication at most three times. It never treats object existence alone as
proof of equivalence.

The first Lambda policy permits at most 10,000 bundle/asset input files, one
GiB of downloaded bundle/asset payload plus a 16 MiB request, one million
frames, and two GiB of artifact payload. The deployment must configure Lambda
ephemeral storage to 10,240 MB. A collision can temporarily retain the
downloaded input, its copied unit root, the new artifact, and the artifact being
verified for reuse; the limits leave headroom for Chromium and bounded transfer
buffers.

The deployment-owned S3 client uses a five-second connect timeout, a
45-second attempt timeout, a 90-second operation timeout, and at most three
SDK attempts. Once `GetObject` has yielded a response body, every pending body
read must make progress within 30 seconds. The body boundary is explicit
because an SDK request timeout ends at a response stream, not at the end of a
download. One absolute 13-minute work deadline covers download,
materialization, capture, verification, and publication. The remaining two
minutes of Lambda's maximum duration are reserved for multipart abort and
runtime response delivery. If publication and abort both fail, the typed error
retains both causes rather than replacing the original failure with cleanup.

Each expensive handler phase emits one structured completion event with its
elapsed milliseconds and success state. The Lambda runtime attaches the
request identity, so CloudWatch can separate input download, unit
materialization, browser capture, artifact verification, and publication.
Nested infrastructure failures retain their source chain in the invocation
error instead of collapsing to the outer deployment category.

For a direct synchronous conformance invocation, disable AWS CLI retries and
raise its default 60-second read timeout:

```sh
AWS_MAX_ATTEMPTS=1 aws lambda invoke \
  --cli-read-timeout 900 \
  --function-name "$FUNCTION" \
  --payload fileb://invocation.json \
  result.json
```

Without those controls, a client-side read timeout may retry while the first
Lambda invocation is still capturing. Conditional publication prevents an
incorrect overwrite, but it cannot recover the duplicated browser cost.

The execution role should be restricted to `s3:GetObject` over the approved
worker-input and artifact prefixes, plus `s3:PutObject` and
`s3:AbortMultipartUpload` over the artifact prefix. It does not require a broad
bucket listing permission.

## Deliberate non-goals

This package does not yet include infrastructure-as-code, a published release
workflow, a queue, leases, scheduler capability matching, or a coordinator.
The deterministic packager replaces the hand-built ZIP procedure, but does not
deploy cloud resources. Any later deployment workflow wraps this same handler
and renderer; it must not fork timing, browser handshakes, artifact identity,
or FFmpeg logic.
