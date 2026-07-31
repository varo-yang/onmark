# Gate-eight closure evidence

> Admitted on 2026-07-31 from implementation commit `8472bf8`.

## Contract under test

Gate eight closes only if the desktop artifacts installed by a user retain the
same authoring, planning, rendering, and feedback contracts exercised inside
the repository. Admission therefore packs the public npm package and its native
platform sidecar, installs both into an empty consumer project, and invokes
only the installed `onmark` entry point.

The fixture covers:

- typed variant application and selected film-level captions;
- exact duration, trim, playback rate, voice-over, and semantic music ducking;
- `check` and versioned `inspect` reports without diagnostics;
- H.264/AAC MP4 and ProRes 4444/PCM MOV delivery;
- exact `snapshot` and `review` artifacts;
- a two-item variant `batch` whose video changes while audio remains equal;
- persistent frame-artifact reuse across separate CLI invocations; and
- no-clobber refusal without changing an existing output.

The release boundary does not use a workspace CLI, private renderer, alternate
planner, template, or hidden pixel fallback.

## Locked local admission

- host: macOS 26.5.2, arm64
- Node.js: 26.4.0
- npm: 11.17.0
- browser contract: Chrome for Testing 149.0.7827.55
- FFmpeg and ffprobe: 8.1.2
- profile: 320 × 180, opaque, 24 fps
- duration: 45 frames
- capture: portable screenshot on the exact SwiftShader contract

The installed candidate reported:

| Surface | Observed result |
| --- | --- |
| `check` | 45 frames, 2 assets, 1 render region |
| `inspect` | variant `headline = "Exact release"`, caption track `en`, music duck target `1/10`, 1 voice-over interval |
| MP4 | 45 decoded H.264 frames and AAC audio |
| MOV | ProRes 4444 video and PCM audio |
| `snapshot` | absolute frame 12 |
| `review` | 7 exact checkpoints plus manifest and static index |
| `batch` | 2 outputs; unequal decoded video hashes and equal decoded audio hashes |
| no-clobber | nonzero status; existing output identity unchanged |

The final installed-product run reused the already verified persistent artifact
and measured 262.2 ms and 257.1 ms for the MP4 invocations and 282.3 ms for the
MOV invocation. These are observations, not portable performance thresholds.
Independent browser-process raw-RGBA repeatability remains owned by the
lower-level exact-raster conformance; a cache hit is not presented as a second
browser capture.

## Cross-platform authority

The desktop release workflow applies the same package assembly and admission
script to `darwin-arm64`, `linux-x64`, and `win32-x64` before publication.
This local record establishes the exact implementation candidate and macOS
result. It does not claim pixel equality across operating systems, browser
products, graphics backends, or capture modes.
