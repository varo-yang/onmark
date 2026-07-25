// Capture-environment tests prove stable host identity and font invalidation.

import assert from "node:assert/strict";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { captureEnvironment } from "../src/capture-environment.js";

test("changes local capture identity when installed fonts change", async () => {
  const root = await mkdtemp(join(tmpdir(), "onmark-capture-environment-"));
  try {
    const fonts = join(root, "fonts");
    await mkdir(fonts);
    const font = join(fonts, "fixture.woff2");
    await writeFile(font, "first");
    const options = {
      cacheEnvironment: {},
      fontDirectories: [fonts],
      homeDirectory: root,
      host: { arch: "arm64", platform: "darwin" },
      osRelease: "fixture-release",
    } as const;

    const first = await captureEnvironment(options);
    const repeated = await captureEnvironment(options);
    await writeFile(font, "changed-font");
    const changed = await captureEnvironment(options);

    assert.equal(first.seed, repeated.seed);
    assert.notEqual(first.seed, changed.seed);
    assert.equal(
      first.cacheDirectory,
      join(root, "Library", "Caches", "onmark", "frames"),
    );
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});
