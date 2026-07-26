// Release workflow tests pin the protected revision that may reach npm.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import test from "node:test";

const WORKFLOW = fileURLToPath(
  new URL("../../../../.github/workflows/desktop-release.yml", import.meta.url),
);
const PUBLISHER = fileURLToPath(new URL("../publish.mjs", import.meta.url));

test("releases a merged release PR from its protected main push", async () => {
  const source = await readFile(WORKFLOW, "utf8");

  assert.doesNotMatch(source, /^  pull_request_target:$/mu);
  assert.match(source, /^  push:$/mu);
  assert.match(source, /^  source:$/mu);
  assert.doesNotMatch(source, /commits\/\$GITHUB_SHA\/pulls/u);
  assert.match(source, /commits\/\$GITHUB_SHA"/u);
  assert.match(source, /pulls\/\$pull_number/u);
  assert.match(source, /"\$merge_revision" == "\$GITHUB_SHA"/u);
  assert.match(source, /"\$base_branch" == main/u);
  assert.match(source, /"\$candidate_branch" == release\/v\*/u);
  assert.match(source, /echo "revision=\$GITHUB_SHA"/u);
  assert.match(source, /needs\.source\.outputs\.version != ''/u);
  assert.match(source, /needs\.source\.outputs\.revision/u);
});

test("restores and refreshes release build caches from trusted main runs", async () => {
  const source = await readFile(WORKFLOW, "utf8");

  assert.match(source, /id: media-cache/u);
  assert.match(source, /id: cargo-cache/u);
  assert.equal(source.split("uses: actions/cache/restore@").length - 1, 2);
  assert.equal(source.split("uses: actions/cache/save@").length - 1, 2);
  assert.match(source, /desktop-media-\$\{\{ matrix\.target \}\}/u);
  assert.match(source, /steps\.media-cache\.outputs\.cache-hit != 'true'/u);
  assert.match(source, /desktop-cargo-\$\{\{ matrix\.target \}\}/u);
  assert.match(
    source,
    /restore-keys:[\s\S]*desktop-cargo-\$\{\{ matrix\.target \}\}-\$\{\{ hashFiles\('rust-toolchain\.toml'\) \}\}-/u,
  );
  const mediaSave = source.slice(
    source.indexOf("- name: Save release media tools"),
    source.indexOf("- name: Save release Cargo artifacts"),
  );
  const cargoSave = source.slice(
    source.indexOf("- name: Save release Cargo artifacts"),
    source.indexOf("\n  publish:"),
  );

  assert.match(
    mediaSave,
    /if: steps\.media-cache\.outputs\.cache-hit != 'true'/u,
  );
  assert.match(
    cargoSave,
    /if: steps\.cargo-cache\.outputs\.cache-hit != 'true'/u,
  );
  assert.match(source, /CARGO_TARGET_DIR: \.release\/cargo-target/u);
  assert.doesNotMatch(source, /^\s+cache: pnpm$/mu);
});

test("warms release caches only after trusted main changes", async () => {
  const source = await readFile(WORKFLOW, "utf8");

  assert.match(source, /^  push:$/mu);
  assert.match(source, /^    branches: \[main\]$/mu);
  assert.match(source, /^  workflow_dispatch:$/mu);
  assert.match(source, /scripts\/release\/media-toolchain\/\*\*/u);
  assert.match(source, /\.github\/workflows\/desktop-release\.yml/u);
  assert.match(source, /rust-toolchain\.toml/u);
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

test("inspects archives without consulting publication state", async () => {
  const source = await readFile(PUBLISHER, "utf8");

  assert.match(source, /\["pack", "--dry-run", "--json", archive\]/u);
  assert.doesNotMatch(source, /\["publish", "--dry-run", "--json", archive\]/u);
});

test("removes setup-node token fallback before OIDC publication", async () => {
  const source = await readFile(WORKFLOW, "utf8");
  const publishStep = source.slice(
    source.indexOf("- name: Publish admitted packages"),
    source.indexOf("- name: Publish GitHub release"),
  );

  assert.match(publishStep, /^\s+unset NODE_AUTH_TOKEN$/mu);
  assert.ok(
    publishStep.indexOf("unset NODE_AUTH_TOKEN") <
      publishStep.indexOf("publish.mjs"),
  );
});
