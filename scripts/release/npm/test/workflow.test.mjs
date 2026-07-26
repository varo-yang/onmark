// Release workflow tests pin the protected revision that may reach npm.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import test from "node:test";

const WORKFLOW = fileURLToPath(
  new URL("../../../../.github/workflows/desktop-release.yml", import.meta.url),
);

test("releases the merged main revision from the protected base context", async () => {
  const source = await readFile(WORKFLOW, "utf8");
  const mergedRevision = "github.event.pull_request.merge_commit_sha";

  assert.match(source, /^  pull_request_target:$/mu);
  assert.equal(source.split(mergedRevision).length - 1, 1);
  assert.match(source, /^  source:$/mu);
  assert.match(source, /needs\.source\.outputs\.revision/u);
  assert.doesNotMatch(source, /ref: \$\{\{ github\.sha \}\}/u);
  assert.match(source, /EVENT_NAME" == pull_request_target/u);
});

test("reuses content-addressed release build artifacts", async () => {
  const source = await readFile(WORKFLOW, "utf8");

  assert.match(source, /id: media-cache/u);
  assert.match(source, /desktop-media-\$\{\{ matrix\.target \}\}/u);
  assert.match(source, /steps\.media-cache\.outputs\.cache-hit != 'true'/u);
  assert.match(source, /desktop-cargo-\$\{\{ matrix\.target \}\}/u);
  assert.match(source, /CARGO_TARGET_DIR: \.release\/cargo-target/u);
});

test("builds the CLI and release driver in one native profile", async () => {
  const source = await readFile(WORKFLOW, "utf8");

  assert.match(
    source,
    /cargo build --locked --release -p onmark-cli -p onmark-xtask/u,
  );
  assert.match(source, /if: matrix\.target == 'linux-x64'/u);
  assert.doesNotMatch(source, /cargo xtask release sidecar/u);
});
