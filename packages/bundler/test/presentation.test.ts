// Public behavior tests for deterministic, staged presentation artifacts.

import assert from "node:assert/strict";
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { BundleError, bundlePresentation } from "../src/index.js";

const ENTRY_SOURCE = `
  import "./presentation.css";
  import { installRuntimeHost } from "@onmark/runtime";
  installRuntimeHost({
    async load() {},
    async prepare() {},
    async seek() {},
    async dispose() {},
  });
`;

test("builds a deterministic immutable presentation artifact", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "onmark-bundler-test-"));
  try {
    const entryPoint = join(workspace, "presentation.ts");
    await writeFile(entryPoint, ENTRY_SOURCE, "utf8");
    await writeFile(
      join(workspace, "presentation.css"),
      "html { background: black; }\n",
      "utf8",
    );

    const first = await bundlePresentation({
      entryPoint,
      outputDirectory: join(workspace, "first"),
      maxOutputBytes: 1_000_000,
    });
    const second = await bundlePresentation({
      entryPoint,
      outputDirectory: join(workspace, "second"),
      maxOutputBytes: 1_000_000,
    });

    assert.deepEqual(first.manifest, second.manifest);
    assert.match(first.manifest.bundleId, /^sha256:[0-9a-f]{64}$/u);
    assert.deepEqual(
      first.manifest.files.map((file) => file.path),
      ["index.html", "presentation.css", "presentation.js"],
    );
    const html = await readFile(join(first.directory, "index.html"), "utf8");
    const script = await readFile(
      join(first.directory, "presentation.js"),
      "utf8",
    );
    const savedManifest: unknown = JSON.parse(
      await readFile(join(first.directory, "manifest.json"), "utf8"),
    );
    assert.match(html, /src="\.\/presentation\.js"/u);
    assert.match(html, /href="\.\/presentation\.css"/u);
    assert.match(script, /__ONMARK_RUNTIME__/u);
    assert.deepEqual(savedManifest, first.manifest);
  } finally {
    await rm(workspace, { force: true, recursive: true });
  }
});

test("does not publish an oversized or pre-existing artifact", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "onmark-bundler-test-"));
  try {
    const entryPoint = join(workspace, "presentation.ts");
    const outputDirectory = join(workspace, "bundle");
    await writeFile(entryPoint, ENTRY_SOURCE, "utf8");
    await writeFile(
      join(workspace, "presentation.css"),
      "html { background: black; }\n",
      "utf8",
    );

    await assert.rejects(
      bundlePresentation({
        entryPoint,
        outputDirectory,
        maxOutputBytes: 1,
      }),
      (error: unknown) =>
        error instanceof BundleError && error.kind === "outputLimit",
    );
    assert.deepEqual((await readdir(workspace)).sort(), [
      "presentation.css",
      "presentation.ts",
    ]);
    await bundlePresentation({
      entryPoint,
      outputDirectory,
      maxOutputBytes: 1_000_000,
    });
    await assert.rejects(
      bundlePresentation({
        entryPoint,
        outputDirectory,
        maxOutputBytes: 1_000_000,
      }),
      (error: unknown) =>
        error instanceof BundleError && error.kind === "output",
    );
  } finally {
    await rm(workspace, { force: true, recursive: true });
  }
});
