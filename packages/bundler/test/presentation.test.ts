// Public behavior tests for deterministic, staged presentation artifacts.

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  BUNDLE_ENTRY_POINT,
  BUNDLE_MANIFEST_FILE,
  BundleError,
  bundlePresentation,
  type BundleManifest,
} from "../src/index.js";

const ENTRY_SOURCE = `
  import "./presentation.css";
  import { installRuntimeHost } from "@onmark/runtime";
  installRuntimeHost({
    async load() {},
    async prepare() {},
    async seek() {},
    async confirm() {},
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
    assert.equal(first.manifest.bundleId, bundleIdentity(first.manifest));
    assert.deepEqual(
      first.manifest.files.map((file) => file.path),
      [BUNDLE_ENTRY_POINT, "presentation.css", "presentation.js"],
    );
    const html = await readFile(
      join(first.directory, BUNDLE_ENTRY_POINT),
      "utf8",
    );
    const savedManifest: unknown = JSON.parse(
      await readFile(join(first.directory, BUNDLE_MANIFEST_FILE), "utf8"),
    );
    assert.match(html, /src="\.\/presentation\.js"/u);
    assert.match(html, /href="\.\/presentation\.css"/u);
    assert.deepEqual(savedManifest, first.manifest);
  } finally {
    await rm(workspace, { force: true, recursive: true });
  }
});

function bundleIdentity(manifest: BundleManifest): string {
  const identity = JSON.stringify({
    version: manifest.version,
    entryPoint: manifest.entryPoint,
    files: manifest.files,
  });
  const digest = createHash("sha256").update(identity).digest("hex");
  return `sha256:${digest}`;
}

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

test("keeps the checked-in video presentation bundle current", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "onmark-bundler-test-"));
  try {
    const repository = fileURLToPath(new URL("../../../..", import.meta.url));
    const expected = join(repository, "conformance/protocol/bundle-v1");
    const outputDirectory = join(workspace, "bundle");
    await bundlePresentation({
      entryPoint: join(repository, "conformance/browser/video-presentation.ts"),
      outputDirectory,
      maxOutputBytes: 1_000_000,
    });

    const files = (await readdir(expected)).sort();
    assert.deepEqual((await readdir(outputDirectory)).sort(), files);
    for (const file of files) {
      assert.deepEqual(
        await readFile(join(outputDirectory, file)),
        await readFile(join(expected, file)),
        `${file} is stale`,
      );
    }
  } finally {
    await rm(workspace, { force: true, recursive: true });
  }
});
