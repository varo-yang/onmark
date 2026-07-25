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
  type BundleOptions,
} from "../src/index.js";

// ── Authored document

test("preserves authored DOM and inline styles", async () => {
  await withWorkspace(async (workspace) => {
    const document = await authoredDocument(
      workspace,
      [
        '<om-film id="demo">',
        "  <style>.accent { color: lime; }</style>",
        "  <om-scene><om-shot>",
        '    <om-title>Write <span class="accent">native.</span></om-title>',
        "  </om-shot></om-scene>",
        "</om-film>",
      ].join("\n"),
      "export const motion = { bind() { return { effects: [], resources: [] }; } };",
    );
    const artifact = await bundlePresentation(
      options(document, join(workspace, "bundle")),
    );
    const bundled = await readFile(
      join(artifact.directory, BUNDLE_ENTRY_POINT),
      "utf8",
    );

    assert.match(bundled, /<span class="accent">native\.<\/span>/u);
    assert.match(bundled, /<style>\.accent \{ color: lime; \}<\/style>/u);
    assert.doesNotMatch(bundled, /data-om-motion/u);
    assert.match(bundled, /src="\.\/presentation\.js"/u);
  });
});

test("excludes compiler-only facts from presentation identity", async () => {
  await withWorkspace(async (workspace) => {
    const firstDocument = await authoredDocument(
      workspace,
      [
        '<om-film id="demo">',
        '  <om-cues><om-cue id="reveal" time="1s"></om-cue></om-cues>',
        '  <om-music src="first.mp3" gain="50%"></om-music>',
        '  <om-scene><om-shot duration="2s">',
        '    <om-sfx src="first.wav" delay="250ms"></om-sfx>',
        '    <om-vo src="first-voice.mp3">Narration</om-vo>',
        '    <video id="hero" class="media" src="first.mp4" delay="100ms"></video>',
        '    <om-title id="headline" data-delay="authored" cue="reveal">',
        '      Keep <img src="texture.png" alt="texture"> me',
        "    </om-title>",
        '    <om-cta id="action" delay="500ms">Act now</om-cta>',
        "  </om-shot></om-scene>",
        "</om-film>",
      ].join("\n"),
    );
    const first = await bundlePresentation(
      options(firstDocument, join(workspace, "first")),
    );

    const secondDocument = await authoredDocument(
      workspace,
      [
        '<om-film id="demo">',
        '  <om-cues><om-cue id="later" time="1500ms"></om-cue></om-cues>',
        '  <om-music src="second.mp3" gain="25%"></om-music>',
        '  <om-scene><om-shot duration="3s">',
        '    <om-sfx src="second.wav" delay="750ms"></om-sfx>',
        '    <om-vo src="second-voice.mp3">Narration</om-vo>',
        '    <video id="hero" class="media" src="second.mp4" delay="200ms"></video>',
        '    <om-title id="headline" data-delay="authored" delay="1s">',
        '      Keep <img src="texture.png" alt="texture"> me',
        "    </om-title>",
        '    <om-cta id="action" cue="later">Act now</om-cta>',
        "  </om-shot></om-scene>",
        "</om-film>",
      ].join("\n"),
    );
    const second = await bundlePresentation(
      options(secondDocument, join(workspace, "second")),
    );

    assert.deepEqual(first.manifest, second.manifest);
    const html = await readFile(
      join(first.directory, BUNDLE_ENTRY_POINT),
      "utf8",
    );
    assert.doesNotMatch(html, /om-cues|om-cue|om-music|om-sfx|om-vo/u);
    assert.doesNotMatch(
      html,
      /first\.mp3|first\.mp4|\s(?:cue|delay|duration|gain)="[^"]*"/u,
    );
    assert.match(html, /<video id="hero" class="media"\s*><\/video>/u);
    assert.match(html, /<om-title id="headline" data-delay="authored"\s*>/u);
    assert.match(html, /<img src="texture\.png" alt="texture">/u);
  });
});

test("publishes one independently identified browser bundle per shot", async () => {
  await withWorkspace(async (workspace) => {
    const baseline = await authoredDocument(
      workspace,
      [
        "<om-film><om-scene>",
        '  <om-shot duration="1s"><om-title>Opening</om-title></om-shot>',
        '  <om-shot duration="1s"><om-title>Closing</om-title></om-shot>',
        "</om-scene></om-film>",
      ].join("\n"),
    );
    const first = await bundlePresentation({
      ...options(baseline, join(workspace, "baseline-regions")),
      temporalCapability: "randomAccess",
    });
    const edited = await authoredDocument(
      workspace,
      [
        "<om-film><om-scene>",
        '  <om-shot duration="1s"><om-title>Opening</om-title></om-shot>',
        '  <om-shot duration="1s"><om-title>Edited closing</om-title></om-shot>',
        "</om-scene></om-film>",
      ].join("\n"),
    );
    const second = await bundlePresentation({
      ...options(edited, join(workspace, "edited-regions")),
      temporalCapability: "randomAccess",
    });

    assert.equal(first.regions.length, 2);
    assert.equal(second.regions.length, 2);
    assert.equal(
      first.regions[0]?.manifest.bundleId,
      second.regions[0]?.manifest.bundleId,
    );
    assert.notEqual(
      first.regions[1]?.manifest.bundleId,
      second.regions[1]?.manifest.bundleId,
    );

    const opening = await readFile(
      join(first.regions[0]!.directory, BUNDLE_ENTRY_POINT),
      "utf8",
    );
    assert.match(opening, /Opening/u);
    assert.doesNotMatch(opening, /Closing/u);
  });
});

test("scopes presentation styles to their semantic owner", async () => {
  await withWorkspace(async (workspace) => {
    const baseline = await regionIdentities(
      workspace,
      "baseline-scope",
      styledScenes("#111111", "#ff0000"),
    );
    const sceneEdit = await regionIdentities(
      workspace,
      "scene-scope",
      styledScenes("#111111", "#0000ff"),
    );
    const filmEdit = await regionIdentities(
      workspace,
      "film-scope",
      styledScenes("#222222", "#ff0000"),
    );

    assert.equal(sceneEdit[0], baseline[0]);
    assert.notEqual(sceneEdit[1], baseline[1]);
    assert.notEqual(filmEdit[0], baseline[0]);
    assert.notEqual(filmEdit[1], baseline[1]);
  });
});

test("invalidates identity for every presentation-owned input", async () => {
  await withWorkspace(async (workspace) => {
    const baselineMarkup = [
      "<style>om-title { color: #152238; }</style>",
      film("<om-title><span>Opening</span></om-title>"),
    ].join("\n");
    const styleEdit = [
      "<style>om-title { color: #d02020; }</style>",
      film("<om-title><span>Opening</span></om-title>"),
    ].join("\n");
    const structureEdit = [
      "<style>om-title { color: #152238; }</style>",
      film("<om-title><em>Opening</em></om-title>"),
    ].join("\n");

    const baseline = await presentationIdentity(
      workspace,
      "baseline",
      baselineMarkup,
      markingMotion("baseline"),
    );
    const styleIdentity = await presentationIdentity(
      workspace,
      "style-edit",
      styleEdit,
      markingMotion("baseline"),
    );
    const structureIdentity = await presentationIdentity(
      workspace,
      "structure-edit",
      structureEdit,
      markingMotion("baseline"),
    );
    const motionIdentity = await presentationIdentity(
      workspace,
      "motion-edit",
      baselineMarkup,
      markingMotion("edited"),
    );
    const edits = [
      ["global style", styleIdentity],
      ["semantic structure", structureIdentity],
      ["motion module", motionIdentity],
    ] as const;

    for (const [kind, identity] of edits) {
      assert.notEqual(identity, baseline, `${kind} must invalidate the bundle`);
    }
  });
});

test("injects the runtime inside an explicit document body", async () => {
  await withWorkspace(async (workspace) => {
    const document = join(workspace, "film.html");
    await writeFile(
      document,
      [
        "<!doctype html>",
        "<html><head><title>Demo</title></head><body>",
        film(""),
        "</body></html>",
      ].join("\n"),
      "utf8",
    );
    const artifact = await bundlePresentation(
      options(document, join(workspace, "bundle")),
    );
    const bundled = await readFile(
      join(artifact.directory, BUNDLE_ENTRY_POINT),
      "utf8",
    );

    assert.match(
      bundled,
      /\n[\t ]*<script type="module" src="\.\/presentation\.js"><\/script>\n[\t ]*<\/body>/u,
    );
    assert.doesNotMatch(
      bundled,
      /\n[\t ]+\n<script type="module" src="\.\/presentation\.js"/u,
    );
    assert.equal(bundled.trimEnd().endsWith("</html>"), true);
  });
});

test("keeps runtime infrastructure outside every projected shot", async () => {
  await withWorkspace(async (workspace) => {
    const document = join(workspace, "film.html");
    await writeFile(
      document,
      [
        "<!doctype html>",
        "<html><body><om-film><om-scene>",
        "<om-shot>",
        '<script type="module" data-om-motion>',
        "export const motion = { bind() { return { effects: [], resources: [] }; } };",
        "</script>",
        "<om-title>Opening</om-title></om-shot>",
        "<om-shot><om-title>Closing</om-title></om-shot>",
        "</om-scene></om-film></body></html>",
      ].join("\n"),
      "utf8",
    );
    const artifact = await bundlePresentation({
      ...options(document, join(workspace, "bundle")),
      temporalCapability: "randomAccess",
    });
    const closing = await readFile(
      join(artifact.regions[1]!.directory, BUNDLE_ENTRY_POINT),
      "utf8",
    );

    assert.match(closing, /src="\.\/presentation\.js"/u);
    assert.doesNotMatch(closing, /data-om-motion|Opening/u);
    assert.match(closing, /Closing/u);
  });
});

test("bundles public motion adapters from the inline module", async () => {
  await withWorkspace(async (workspace) => {
    const document = await authoredDocument(
      workspace,
      film("<om-title>Motion</om-title>"),
      [
        'import { gsapMotion } from "onmark/motion/gsap";',
        "export const motion = gsapMotion({",
        "  title({ element, timeline }) {",
        "    timeline.from(element, { opacity: 0, duration: 0.25 });",
        "  },",
        "});",
      ].join("\n"),
    );
    const artifact = await bundlePresentation({
      ...options(document, join(workspace, "bundle")),
      temporalCapability: "randomAccess",
    });

    assert.equal(artifact.manifest.temporalCapability, "randomAccess");
  });
});

// ── Artifact contract

test("builds deterministic artifacts with capability-owned identity", async () => {
  await withWorkspace(async (workspace) => {
    const document = await authoredDocument(workspace, film(""));
    const first = await bundlePresentation(
      options(document, join(workspace, "first")),
    );
    const second = await bundlePresentation(
      options(document, join(workspace, "second")),
    );
    const randomAccess = await bundlePresentation({
      ...options(document, join(workspace, "random-access")),
      temporalCapability: "randomAccess",
    });
    const separableOverlay = await bundlePresentation({
      ...options(document, join(workspace, "separable-overlay")),
      visualCapability: "separableOverlay",
    });
    const placementBounded = await bundlePresentation({
      ...options(document, join(workspace, "placement-bounded")),
      frameBehavior: "placementBounded",
      temporalCapability: "randomAccess",
    });

    assert.deepEqual(first.manifest, second.manifest);
    assert.equal(first.manifest.bundleId, bundleIdentity(first.manifest));
    assert.deepEqual(first.manifest.files, randomAccess.manifest.files);
    assert.notEqual(first.manifest.bundleId, randomAccess.manifest.bundleId);
    assert.notEqual(
      first.manifest.bundleId,
      separableOverlay.manifest.bundleId,
    );
    assert.notEqual(
      first.manifest.bundleId,
      placementBounded.manifest.bundleId,
    );
    assert.equal(
      randomAccess.manifest.bundleId,
      bundleIdentity(randomAccess.manifest),
    );
    assert.equal(
      separableOverlay.manifest.bundleId,
      bundleIdentity(separableOverlay.manifest),
    );
    assert.equal(
      placementBounded.manifest.bundleId,
      bundleIdentity(placementBounded.manifest),
    );
    const savedManifest: unknown = JSON.parse(
      await readFile(join(first.directory, BUNDLE_MANIFEST_FILE), "utf8"),
    );
    assert.deepEqual(savedManifest, first.manifest);
  });
});

test("rejects placement-bounded pixels without random access", async () => {
  await withWorkspace(async (workspace) => {
    const document = await authoredDocument(workspace, film(""));

    await assert.rejects(
      bundlePresentation({
        ...options(document, join(workspace, "bundle")),
        frameBehavior: "placementBounded",
      }),
      (error: unknown) =>
        error instanceof BundleError &&
        error.message ===
          "placement-bounded frames require random-access presentation timing",
    );
  });
});

test("carries imported visual resources into the immutable bundle", async () => {
  await withWorkspace(async (workspace) => {
    const fontBytes = "font fixture bytes";
    const svgBytes =
      '<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1" />';
    await writeFile(
      join(workspace, "presentation.css"),
      '@font-face { font-family: "Onmark Test"; src: url("./body.woff2"); }\n' +
        'body { background-image: url("./poster.svg"); }\n',
      "utf8",
    );
    await writeFile(join(workspace, "body.woff2"), fontBytes);
    await writeFile(join(workspace, "poster.svg"), svgBytes, "utf8");
    const document = await authoredDocument(
      workspace,
      [
        "<style>",
        "  .sample::before {",
        `    content: '<script type="module" src="./presentation.js"></script>';`,
        "  }",
        "</style>",
        film(""),
      ].join("\n"),
      [
        'import "./presentation.css";',
        "export const motion = { bind() {",
        "  return { effects: [], resources: [] };",
        "} };",
      ].join("\n"),
    );
    const first = await bundlePresentation(
      options(document, join(workspace, "first")),
    );
    const second = await bundlePresentation(
      options(document, join(workspace, "second")),
    );

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
    const html = await readFile(
      join(first.directory, BUNDLE_ENTRY_POINT),
      "utf8",
    );
    assert.match(
      html,
      /content: '<script type="module" src="\.\/presentation\.js"><\/script>';/u,
    );
    assert.ok(html.lastIndexOf("<link rel=") > html.indexOf("</style>"));

    await writeFile(
      join(workspace, "poster.svg"),
      svgBytes.replace('width="1"', 'width="2"'),
      "utf8",
    );
    const changed = await bundlePresentation(
      options(document, join(workspace, "changed-resource")),
    );
    assert.notEqual(changed.manifest.bundleId, first.manifest.bundleId);
  });
});

test("does not publish an oversized or pre-existing artifact", async () => {
  await withWorkspace(async (workspace) => {
    const document = await authoredDocument(workspace, film(""));
    const outputDirectory = join(workspace, "bundle");

    await assert.rejects(
      bundlePresentation({
        ...options(document, outputDirectory),
        maxOutputBytes: 1,
      }),
      (error: unknown) =>
        error instanceof BundleError && error.kind === "outputLimit",
    );
    assert.deepEqual(await readdir(workspace), ["film.html"]);
    await bundlePresentation(options(document, outputDirectory));
    await assert.rejects(
      bundlePresentation(options(document, outputDirectory)),
      (error: unknown) =>
        error instanceof BundleError && error.kind === "output",
    );
  });
});

test("keeps the checked-in browser bundle current", async () => {
  await withWorkspace(async (workspace) => {
    const repository = fileURLToPath(new URL("../../../..", import.meta.url));
    const expected = join(repository, "conformance/protocol/bundle-v1");
    const outputDirectory = join(workspace, "bundle");
    await bundlePresentation({
      ...options(
        join(repository, "conformance/browser/video-presentation.html"),
        outputDirectory,
      ),
      temporalCapability: "randomAccess",
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
  });
});

test("bundles the temporal experiment with its browser libraries", async () => {
  await withWorkspace(async (workspace) => {
    const repository = fileURLToPath(new URL("../../../..", import.meta.url));
    const outputDirectory = join(workspace, "bundle");
    await bundlePresentation({
      ...options(
        join(repository, "conformance/browser/temporal-experiment.html"),
        outputDirectory,
      ),
      maxOutputBytes: 2_000_000,
      temporalCapability: "randomAccess",
    });

    assert.deepEqual((await readdir(outputDirectory)).sort(), [
      BUNDLE_ENTRY_POINT,
      BUNDLE_MANIFEST_FILE,
      "presentation.js",
      "regions",
    ]);
  });
});

// ── Test support

function options(document: string, outputDirectory: string): BundleOptions {
  return {
    document,
    frameBehavior: "perFrame",
    maxOutputBytes: 1_000_000,
    outputDirectory,
    temporalCapability: "sequential",
    visualCapability: "browserComposite",
  };
}

function film(content: string): string {
  return [
    "<om-film>",
    "  <om-scene><om-shot>",
    `    ${content}`,
    "  </om-shot></om-scene>",
    "</om-film>",
  ].join("\n");
}

async function authoredDocument(
  workspace: string,
  markup: string,
  motion?: string,
): Promise<string> {
  const document = join(workspace, "film.html");
  const script =
    motion === undefined
      ? ""
      : ['<script type="module" data-om-motion>', motion, "</script>"].join(
          "\n",
        );
  await writeFile(
    document,
    ["<!doctype html>", markup, script, ""].join("\n"),
    "utf8",
  );
  return document;
}

async function presentationIdentity(
  workspace: string,
  label: string,
  markup: string,
  motion: string,
): Promise<string> {
  const document = await authoredDocument(workspace, markup, motion);
  const artifact = await bundlePresentation(
    options(document, join(workspace, label)),
  );
  return artifact.manifest.bundleId;
}

async function regionIdentities(
  workspace: string,
  label: string,
  markup: string,
): Promise<readonly string[]> {
  const document = await authoredDocument(workspace, markup);
  const artifact = await bundlePresentation({
    ...options(document, join(workspace, label)),
    temporalCapability: "randomAccess",
  });
  return artifact.regions.map((region) => region.manifest.bundleId);
}

function styledScenes(filmColor: string, closingColor: string): string {
  return [
    "<om-film>",
    `  <style>body { background: ${filmColor}; }</style>`,
    "  <om-scene>",
    "    <style>om-title { color: white; }</style>",
    "    <om-shot><om-title>Opening</om-title></om-shot>",
    "  </om-scene>",
    "  <om-scene>",
    `    <style>om-title { color: ${closingColor}; }</style>`,
    "    <om-shot><om-title>Closing</om-title></om-shot>",
    "  </om-scene>",
    "</om-film>",
  ].join("\n");
}

function markingMotion(value: string): string {
  return `export const motion = {
  bind() {
    return {
      effects: [{
        apply() { document.body.dataset.motion = ${JSON.stringify(value)}; },
        dispose() { delete document.body.dataset.motion; },
      }],
      resources: [],
    };
  },
};`;
}

function bundleIdentity(manifest: BundleManifest): string {
  const identity = JSON.stringify({
    version: manifest.version,
    entryPoint: manifest.entryPoint,
    documentScope: manifest.documentScope,
    temporalCapability: manifest.temporalCapability,
    visualCapability: manifest.visualCapability,
    frameBehavior: manifest.frameBehavior,
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
  const files = await artifactFiles(directory);
  const generated = files.filter(
    (path) => path.endsWith(".css") || path.endsWith(".js"),
  );
  const contents = await Promise.all(
    generated.map((path) => readFile(join(directory, path), "utf8")),
  );
  return contents.join("\n");
}

async function withWorkspace(
  run: (workspace: string) => Promise<void>,
): Promise<void> {
  const workspace = await mkdtemp(join(tmpdir(), "onmark-bundler-test-"));
  try {
    await run(workspace);
  } finally {
    await rm(workspace, { force: true, recursive: true });
  }
}
