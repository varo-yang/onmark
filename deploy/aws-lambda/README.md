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
image must supply the same identity; a mismatch fails before the worker starts.
The structured result follows the checked-in
[`aws-capture-result-v1` schema](../../schemas/aws-capture-result-v1.schema.json).
These schemas are generated from Rust and checked by `cargo xtask schema
--check`. There is intentionally no TypeScript AWS SDK yet: no TypeScript
caller exists, and inventing one would prebuild a Gate-three coordinator.

The Lambda configuration is deployment-owned:

- `ONMARK_ARTIFACT_BUCKET` — required output bucket;
- `ONMARK_ARTIFACT_PREFIX` — optional canonical output prefix;
- `ONMARK_BROWSER_BINARY` — required Chromium executable inside the image;
- `ONMARK_CAPTURE_ENVIRONMENT` — required `sha256:<lowercase-hex>` identity
  of the complete pixel-affecting image environment.

The invocation cannot choose an output bucket, browser binary, environment, or
resource limit.

## Chromium isolation policy

Local and ordinary product launches retain Chromium's own sandbox. This adapter
explicitly selects `ChromiumSandbox::Disabled` because a Lambda worker is
intended to run inside a separately isolated execution environment. It is not a
fallback after a failed browser launch, and an invocation cannot choose it. The
deployment's capture-environment identity must include that launch policy.

This is an experimental deployment contract, not a claim that every Lambda
image is already safe to run. A real Lambda conformance run must establish the
outer process-isolation and Chromium launch behavior before a container image
or infrastructure template is treated as production-ready.

A disposable arm64 Debian container has exercised the real `BrowserSession`
under a non-root user, read-only root filesystem, writable `/tmp`, no Linux
capabilities, no network, no privilege escalation, and bounded memory and PIDs.
Chromium could not create its internal namespace sandbox in that boundary even
with the distribution sandbox helper installed; the explicit disabled policy
completed launch, non-empty PNG capture, and shutdown. This proves the renderer
path inside a representative outer sandbox, not the still-required Lambda
conformance.

## Publication and limits

The handler follows one linear path:

```text
decode invocation
→ download the bounded worker input
→ materialize the verified Render Unit
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
download.

The execution role should be restricted to `s3:GetObject` over the approved
worker-input and artifact prefixes, plus `s3:PutObject` and
`s3:AbortMultipartUpload` over the artifact prefix. It does not require a broad
bucket listing permission.

## Deliberate non-goals

This package does not yet include infrastructure-as-code, a production
container image, a queue, leases, scheduler capability matching, or a
coordinator. In particular, a Chromium-in-Lambda sandbox experiment must prove
the image launch contract before a Dockerfile or deployment template is treated
as production-ready. Those additions wrap this same handler and renderer; they
must not fork timing, browser handshakes, artifact identity, or FFmpeg logic.
