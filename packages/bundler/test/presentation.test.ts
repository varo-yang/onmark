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
  bundleDomPresentation,
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

// ── Artifact construction ──

test("builds the semantic DOM presentation without an authored entry", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "onmark-bundler-test-"));
  try {
    const stylesheet = join(workspace, "film.css");
    await writeFile(stylesheet, ".onmark-title { color: #baff29; }\n", "utf8");

    const artifact = await bundleDomPresentation({
      stylesheet,
      outputDirectory: join(workspace, "bundle"),
      maxOutputBytes: 1_000_000,
      temporalCapability: "sequential",
      visualCapability: "browserComposite",
    });

    assert.deepEqual(
      artifact.manifest.files.map((file) => file.path),
      [BUNDLE_ENTRY_POINT, "presentation.css", "presentation.js"],
    );
    assert.equal(
      await readFile(join(artifact.directory, "presentation.css"), "utf8"),
      ".onmark-title{color:#baff29}\n",
    );
  } finally {
    await rm(workspace, { force: true, recursive: true });
  }
});

test("rejects semantic stylesheet resources even when motion is present", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "onmark-bundler-test-"));
  try {
    const stylesheet = join(workspace, "film.css");
    const motion = join(workspace, "film.motion.ts");
    await writeFile(join(workspace, "hero.png"), new Uint8Array([1, 2, 3]));
    await writeFile(
      stylesheet,
      '.onmark-film { background: url("./hero.png"); }\n',
      "utf8",
    );
    await writeFile(
      motion,
      `
        export const motion = {
          bind() {
            return { effects: [], resources: [] };
          },
        };
      `,
      "utf8",
    );

    for (const [index, motionEntry] of [undefined, motion].entries()) {
      await assert.rejects(
        bundleDomPresentation({
          stylesheet,
          ...(motionEntry === undefined ? {} : { motion: motionEntry }),
          outputDirectory: join(workspace, `bundle-${index}`),
          maxOutputBytes: 1_000_000,
          temporalCapability: "sequential",
          visualCapability: "browserComposite",
        }),
        (error: unknown) =>
          error instanceof BundleError &&
          error.message ===
            "semantic stylesheet resources have no explicit readiness owner",
      );
    }
  } finally {
    await rm(workspace, { force: true, recursive: true });
  }
});

test("adds no visual defaults when the semantic DOM has no stylesheet", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "onmark-bundler-test-"));
  try {
    const artifact = await bundleDomPresentation({
      outputDirectory: join(workspace, "bundle"),
      maxOutputBytes: 1_000_000,
      temporalCapability: "sequential",
      visualCapability: "browserComposite",
    });

    assert.deepEqual(
      artifact.manifest.files.map((file) => file.path),
      [BUNDLE_ENTRY_POINT, "presentation.js"],
    );
  } finally {
    await rm(workspace, { force: true, recursive: true });
  }
});

test("bundles same-stem vendor-neutral motion through the semantic entry", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "onmark-bundler-test-"));
  try {
    const motion = join(workspace, "film.motion.ts");
    await writeFile(
      motion,
      `
        import { gsapMotion } from "onmark/motion/gsap";
        export const motion = gsapMotion({
          title({ element, timeline }) {
            timeline.from(element, { opacity: 0, duration: 0.25 });
          },
        });
      `,
      "utf8",
    );

    const artifact = await bundleDomPresentation({
      motion,
      outputDirectory: join(workspace, "bundle"),
      maxOutputBytes: 1_000_000,
      temporalCapability: "randomAccess",
      visualCapability: "browserComposite",
    });
    assert.equal(artifact.manifest.temporalCapability, "randomAccess");
  } finally {
    await rm(workspace, { force: true, recursive: true });
  }
});

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
      temporalCapability: "sequential",
      visualCapability: "browserComposite",
    });
    const second = await bundlePresentation({
      entryPoint,
      outputDirectory: join(workspace, "second"),
      maxOutputBytes: 1_000_000,
      temporalCapability: "sequential",
      visualCapability: "browserComposite",
    });
    const randomAccess = await bundlePresentation({
      entryPoint,
      outputDirectory: join(workspace, "random-access"),
      maxOutputBytes: 1_000_000,
      temporalCapability: "randomAccess",
      visualCapability: "browserComposite",
    });
    const separableOverlay = await bundlePresentation({
      entryPoint,
      outputDirectory: join(workspace, "separable-overlay"),
      maxOutputBytes: 1_000_000,
      temporalCapability: "sequential",
      visualCapability: "separableOverlay",
    });

    assert.deepEqual(first.manifest, second.manifest);
    assert.equal(first.manifest.bundleId, bundleIdentity(first.manifest));
    assert.deepEqual(first.manifest.files, randomAccess.manifest.files);
    assert.notEqual(first.manifest.bundleId, randomAccess.manifest.bundleId);
    assert.equal(
      randomAccess.manifest.bundleId,
      bundleIdentity(randomAccess.manifest),
    );
    assert.deepEqual(first.manifest.files, separableOverlay.manifest.files);
    assert.notEqual(
      first.manifest.bundleId,
      separableOverlay.manifest.bundleId,
    );
    assert.equal(
      separableOverlay.manifest.bundleId,
      bundleIdentity(separableOverlay.manifest),
    );
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

test("carries local visual resources into the immutable bundle", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "onmark-bundler-test-"));
  try {
    const entryPoint = join(workspace, "presentation.ts");
    const fontBytes = "font fixture bytes";
    const svgBytes =
      '<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" />';
    await writeFile(entryPoint, ENTRY_SOURCE, "utf8");
    await writeFile(
      join(workspace, "presentation.css"),
      '@font-face { font-family: "Onmark Test"; src: url("./body.woff2"); }\n' +
        'body { background-image: url("./poster.svg"); }\n',
      "utf8",
    );
    await writeFile(join(workspace, "body.woff2"), fontBytes);
    await writeFile(join(workspace, "poster.svg"), svgBytes, "utf8");

    const first = await bundlePresentation({
      entryPoint,
      outputDirectory: join(workspace, "first"),
      maxOutputBytes: 1_000_000,
      temporalCapability: "sequential",
      visualCapability: "browserComposite",
    });
    const second = await bundlePresentation({
      entryPoint,
      outputDirectory: join(workspace, "second"),
      maxOutputBytes: 1_000_000,
      temporalCapability: "sequential",
      visualCapability: "browserComposite",
    });
    assert.deepEqual(first.manifest, second.manifest);

    const resources = first.manifest.files
      .map((file) => file.path)
      .filter((path) => path.startsWith("resources/"));
    assert.equal(resources.length, 2);
    assert.equal(
      resources.every((path) => path === path.toLowerCase()),
      true,
    );
    const svg = resources.find((path) => path.endsWith(".svg"));
    const font = resources.find((path) => path.endsWith(".woff2"));
    assert.ok(svg);
    assert.ok(font);
    assert.equal(await readFile(join(first.directory, svg), "utf8"), svgBytes);
    assert.equal(
      await readFile(join(first.directory, font), "utf8"),
      fontBytes,
    );
    const references = await generatedText(first.directory);
    for (const resource of resources) {
      assert.equal(references.includes(resource), true);
    }
  } finally {
    await rm(workspace, { force: true, recursive: true });
  }
});

// ── Publication failures ──

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
        temporalCapability: "sequential",
        visualCapability: "browserComposite",
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
      temporalCapability: "sequential",
      visualCapability: "browserComposite",
    });
    await assert.rejects(
      bundlePresentation({
        entryPoint,
        outputDirectory,
        maxOutputBytes: 1_000_000,
        temporalCapability: "sequential",
        visualCapability: "browserComposite",
      }),
      (error: unknown) =>
        error instanceof BundleError && error.kind === "output",
    );
  } finally {
    await rm(workspace, { force: true, recursive: true });
  }
});

// ── Checked fixtures ──

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
      temporalCapability: "randomAccess",
      visualCapability: "browserComposite",
    });

    const files = await artifactFiles(expected);
    assert.deepEqual(await artifactFiles(outputDirectory), files);
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

test("bundles the Gate-five temporal experiment with its browser libraries", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "onmark-bundler-test-"));
  try {
    const repository = fileURLToPath(new URL("../../../..", import.meta.url));
    const outputDirectory = join(workspace, "bundle");
    await bundlePresentation({
      entryPoint: join(
        repository,
        "conformance/browser/temporal-experiment.ts",
      ),
      outputDirectory,
      maxOutputBytes: 2_000_000,
      temporalCapability: "randomAccess",
      visualCapability: "browserComposite",
    });

    assert.deepEqual((await readdir(outputDirectory)).sort(), [
      BUNDLE_ENTRY_POINT,
      BUNDLE_MANIFEST_FILE,
      "presentation.css",
      "presentation.js",
    ]);
  } finally {
    await rm(workspace, { force: true, recursive: true });
  }
});

// ── Test support ──

function bundleIdentity(manifest: BundleManifest): string {
  const identity = JSON.stringify({
    version: manifest.version,
    entryPoint: manifest.entryPoint,
    temporalCapability: manifest.temporalCapability,
    visualCapability: manifest.visualCapability,
    files: manifest.files,
  });
  const digest = createHash("sha256").update(identity).digest("hex");
  return `sha256:${digest}`;
}

async function artifactFiles(root: string, directory = ""): Promise<string[]> {
  const files: string[] = [];
  const entries = await readdir(join(root, directory), {
    withFileTypes: true,
  });
  for (const entry of entries) {
    const path = directory === "" ? entry.name : `${directory}/${entry.name}`;
    if (entry.isDirectory()) {
      files.push(...(await artifactFiles(root, path)));
    } else if (entry.isFile()) {
      files.push(path);
    } else {
      throw new Error(`bundle fixture contains unsupported entry ${path}`);
    }
  }
  return files.sort();
}

async function generatedText(directory: string): Promise<string> {
  const [script, style] = await Promise.all([
    readFile(join(directory, "presentation.js"), "utf8"),
    readFile(join(directory, "presentation.css"), "utf8"),
  ]);
  return `${script}\n${style}`;
}
