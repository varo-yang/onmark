// Deterministic Node boundary for producing one immutable browser presentation.
// It owns esbuild and filesystem effects; runtime code remains browser-only.

import { createHash } from "node:crypto";
import { lstat, mkdir, mkdtemp, rename, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";

import {
  build,
  type BuildOptions,
  type OnResolveArgs,
  type OnResolveResult,
  type OutputFile,
  type Plugin,
} from "esbuild";
import type {
  PresentationFrameBehavior,
  PresentationTemporalCapability,
  PresentationVisualCapability,
} from "@onmark/runtime/types";

import {
  BUNDLE_ENTRY_POINT,
  BUNDLE_FRAME_BEHAVIORS,
  BUNDLE_MANIFEST_FILE,
  BUNDLE_TEMPORAL_CAPABILITIES,
  BUNDLE_VISUAL_CAPABILITIES,
  BUNDLE_VERSION,
  type BundleFile as WireBundleFile,
  type BundleManifest as WireBundleManifest,
} from "./generated/bundle-manifest.js";

// Authored files live outside the package tree, so public facades resolve from
// Onmark's own export map rather than from the temporary source directory.
const resolveOnmarkExport = createRequire(
  new URL("../../../../package.json", import.meta.url),
).resolve;
const AUTHORING_ENTRY = resolveOnmarkExport("#onmark-authoring");
const RUNTIME_ENTRY = resolveOnmarkExport("#onmark-runtime");
const VISUAL_RESOURCE_LOADERS = {
  ".avif": "file",
  ".gif": "file",
  ".jpeg": "file",
  ".jpg": "file",
  ".otf": "file",
  ".png": "file",
  ".svg": "file",
  ".ttf": "file",
  ".webp": "file",
  ".woff": "file",
  ".woff2": "file",
} as const;

// ── Public contract

type Immutable<T> = T extends object
  ? { readonly [Key in keyof T]: Immutable<T[Key]> }
  : T;

/** Immutable view of one Rust-owned bundle payload entry. */
export type BundleFile = Immutable<WireBundleFile>;

/** Immutable view of the versioned Rust-owned bundle manifest. */
export type BundleManifest = Immutable<WireBundleManifest>;

/** Explicit inputs and retained-output bound for one presentation build. */
export interface BundleOptions {
  readonly entryPoint: string;
  readonly outputDirectory: string;
  readonly maxOutputBytes: number;
  readonly temporalCapability: PresentationTemporalCapability;
  readonly visualCapability: PresentationVisualCapability;
  readonly frameBehavior: PresentationFrameBehavior;
}

/** Explicit inputs for the neutral semantic DOM presentation. */
export interface DomBundleOptions {
  readonly motion?: string;
  readonly stylesheet?: string;
  readonly outputDirectory: string;
  readonly maxOutputBytes: number;
  readonly temporalCapability: PresentationTemporalCapability;
  readonly visualCapability: PresentationVisualCapability;
  readonly frameBehavior: PresentationFrameBehavior;
}

/** Published directory and its owned immutable manifest snapshot. */
export interface BundleArtifact {
  readonly directory: string;
  readonly manifest: BundleManifest;
}

export type BundleErrorKind =
  "configuration" | "build" | "output" | "outputLimit";

/** Typed failure from presentation compilation or artifact publication. */
export class BundleError extends Error {
  readonly kind: BundleErrorKind;

  constructor(kind: BundleErrorKind, message: string, cause?: unknown) {
    super(message, cause === undefined ? undefined : { cause });
    this.name = "BundleError";
    this.kind = kind;
  }
}

interface PendingFile {
  readonly contents: Uint8Array;
  readonly path: string;
}

interface BundleInput {
  readonly source: PresentationSource;
  readonly outputDirectory: string;
  readonly maxOutputBytes: number;
  readonly temporalCapability: PresentationTemporalCapability;
  readonly visualCapability: PresentationVisualCapability;
  readonly frameBehavior: PresentationFrameBehavior;
}

type PresentationSource =
  | { readonly kind: "custom"; readonly path: string }
  | {
      readonly kind: "semanticDom";
      readonly motion: string | undefined;
      readonly stylesheet: string | undefined;
    };

type NonEmpty<T> = readonly [T, ...T[]];

// ── Build pipeline

/** Builds one immutable presentation through a private staging directory. */
export async function bundlePresentation(
  options: BundleOptions,
): Promise<BundleArtifact> {
  return bundle({
    maxOutputBytes: options.maxOutputBytes,
    outputDirectory: options.outputDirectory,
    source: { kind: "custom", path: options.entryPoint },
    temporalCapability: options.temporalCapability,
    visualCapability: options.visualCapability,
    frameBehavior: options.frameBehavior,
  });
}

/** Builds the semantic DOM presentation without an authored entry module. */
export async function bundleDomPresentation(
  options: DomBundleOptions,
): Promise<BundleArtifact> {
  return bundle({
    maxOutputBytes: options.maxOutputBytes,
    outputDirectory: options.outputDirectory,
    source: {
      kind: "semanticDom",
      motion: options.motion,
      stylesheet: options.stylesheet,
    },
    temporalCapability: options.temporalCapability,
    visualCapability: options.visualCapability,
    frameBehavior: options.frameBehavior,
  });
}

async function bundle(options: BundleInput): Promise<BundleArtifact> {
  const input = validateInput(options);
  await requireAbsent(input.outputDirectory);
  await mkdir(dirname(input.outputDirectory), { recursive: true });
  const staging = await mkdtemp(
    join(dirname(input.outputDirectory), ".onmark-bundle-"),
  );

  try {
    return await buildArtifact(input, staging);
  } catch (error) {
    const failure = bundleFailure(error);
    await removeFailedStaging(staging, failure);
    throw failure;
  }
}

async function buildArtifact(
  input: BundleInput,
  staging: string,
): Promise<BundleArtifact> {
  const generated = await compilePresentation(input.source, staging);
  const pending = presentationFiles(generated, staging);
  const manifest = createManifest(
    pending,
    input.temporalCapability,
    input.visualCapability,
    input.frameBehavior,
  );
  const manifestBytes = encodeManifest(manifest);
  enforceOutputLimit(pending, manifestBytes, input.maxOutputBytes);

  await writePendingFiles(staging, pending);
  await writeFile(join(staging, BUNDLE_MANIFEST_FILE), manifestBytes);
  await requireAbsent(input.outputDirectory);
  await rename(staging, input.outputDirectory);

  return Object.freeze({
    directory: input.outputDirectory,
    manifest,
  });
}

function validateInput(options: BundleInput): BundleInput {
  if (options.outputDirectory.length === 0) {
    throw new BundleError("configuration", "output directory cannot be empty");
  }
  if (
    !Number.isSafeInteger(options.maxOutputBytes) ||
    options.maxOutputBytes <= 0
  ) {
    throw new BundleError(
      "configuration",
      "maximum output bytes must be a positive safe integer",
    );
  }

  const temporalCapability = validateTemporalCapability(
    options.temporalCapability,
  );
  const visualCapability = validateVisualCapability(options.visualCapability);
  const frameBehavior = validateFrameBehavior(options.frameBehavior);
  if (
    frameBehavior === "placementBounded" &&
    temporalCapability !== "randomAccess"
  ) {
    throw new BundleError(
      "configuration",
      "placement-bounded frames require random-access presentation timing",
    );
  }

  return Object.freeze({
    outputDirectory: resolve(options.outputDirectory),
    maxOutputBytes: options.maxOutputBytes,
    source: validateSource(options.source),
    temporalCapability,
    visualCapability,
    frameBehavior,
  });
}

function validateSource(source: PresentationSource): PresentationSource {
  switch (source.kind) {
    case "custom":
      if (source.path.length === 0) {
        throw new BundleError("configuration", "entry point cannot be empty");
      }
      return Object.freeze({ kind: source.kind, path: resolve(source.path) });
    case "semanticDom":
      return Object.freeze({
        kind: source.kind,
        motion: optionalSourcePath(source.motion, "motion entry"),
        stylesheet: optionalSourcePath(source.stylesheet, "stylesheet"),
      });
  }
}

function optionalSourcePath(
  path: string | undefined,
  role: string,
): string | undefined {
  if (path === undefined) {
    return undefined;
  }
  if (path.length === 0) {
    throw new BundleError("configuration", `${role} cannot be empty`);
  }
  return resolve(path);
}

function validateTemporalCapability(
  capability: PresentationTemporalCapability,
): PresentationTemporalCapability {
  const admitted = BUNDLE_TEMPORAL_CAPABILITIES.find(
    (candidate) => candidate === capability,
  );
  if (admitted !== undefined) {
    return admitted;
  }
  throw new BundleError(
    "configuration",
    "temporal capability must be sequential or randomAccess",
  );
}

function validateVisualCapability(
  capability: PresentationVisualCapability,
): PresentationVisualCapability {
  const admitted = BUNDLE_VISUAL_CAPABILITIES.find(
    (candidate) => candidate === capability,
  );
  if (admitted !== undefined) {
    return admitted;
  }
  throw new BundleError(
    "configuration",
    "visual capability must be browserComposite or separableOverlay",
  );
}

function validateFrameBehavior(
  behavior: PresentationFrameBehavior,
): PresentationFrameBehavior {
  const admitted = BUNDLE_FRAME_BEHAVIORS.find(
    (candidate) => candidate === behavior,
  );
  if (admitted !== undefined) {
    return admitted;
  }
  throw new BundleError(
    "configuration",
    "frame behavior must be perFrame or placementBounded",
  );
}

async function compilePresentation(
  source: PresentationSource,
  staging: string,
): Promise<readonly OutputFile[]> {
  try {
    const result = await build({
      alias: {
        "@onmark/authoring": AUTHORING_ENTRY,
        "@onmark/runtime": RUNTIME_ENTRY,
      },
      assetNames: "resources/[hash]",
      bundle: true,
      entryNames: "presentation",
      ...buildSource(source),
      format: "esm",
      legalComments: "none",
      loader: VISUAL_RESOURCE_LOADERS,
      minify: true,
      outdir: staging,
      platform: "browser",
      plugins: [publicOnmarkImports()],
      target: "es2024",
      write: false,
    });
    rejectUnobservedStylesheetResources(source, result.outputFiles, staging);
    return result.outputFiles;
  } catch (error) {
    if (error instanceof BundleError) {
      throw error;
    }
    throw new BundleError("build", "presentation compilation failed", error);
  }
}

function publicOnmarkImports(): Plugin {
  return {
    name: "onmark-public-imports",
    setup(buildContext) {
      buildContext.onResolve({ filter: /^onmark\// }, resolvePublicImport);
    },
  };
}

function resolvePublicImport(args: OnResolveArgs): OnResolveResult {
  try {
    return { path: resolveOnmarkExport(args.path) };
  } catch (error) {
    const failure = {
      detail: error,
      text: `cannot resolve public Onmark import ${args.path}`,
    };
    return { errors: [failure] };
  }
}

function rejectUnobservedStylesheetResources(
  source: PresentationSource,
  outputFiles: readonly OutputFile[],
  staging: string,
): void {
  if (source.kind !== "semanticDom" || source.stylesheet === undefined) {
    return;
  }
  const resourcePaths = outputFiles
    .map((file) => artifactPath(staging, file.path))
    .filter((path) => path.startsWith("resources/"));
  const stylesheetReferencesResource = outputFiles
    .filter((file) => file.path.endsWith(".css"))
    .map((file) => new TextDecoder().decode(file.contents))
    .some((css) => resourcePaths.some((path) => css.includes(path)));
  if (stylesheetReferencesResource) {
    throw new BundleError(
      "build",
      "semantic stylesheet resources have no explicit readiness owner",
    );
  }
}

function buildSource(source: PresentationSource): BuildOptions {
  switch (source.kind) {
    case "custom":
      return { entryPoints: [source.path] };
    case "semanticDom":
      return { stdin: semanticDomEntry(source.stylesheet, source.motion) };
  }
}

function semanticDomEntry(
  stylesheet: string | undefined,
  motion: string | undefined,
): NonNullable<BuildOptions["stdin"]> {
  return {
    contents: semanticDomModule(stylesheet, motion),
    loader: "ts",
    resolveDir: dirname(AUTHORING_ENTRY),
    sourcefile: "onmark-semantic-dom.ts",
  };
}

function semanticDomModule(
  stylesheet: string | undefined,
  motion: string | undefined,
): string {
  const stylesheetImport =
    stylesheet === undefined ? [] : [`import ${JSON.stringify(stylesheet)};`];
  const motionImport =
    motion === undefined
      ? []
      : [`import { motion } from ${JSON.stringify(motion)};`];
  const motionOption = motion === undefined ? [] : ["  motion,"];
  return [
    ...stylesheetImport,
    ...motionImport,
    'import { createDomPresentationBindings } from "@onmark/authoring";',
    "import {",
    "  installRuntimeHost,",
    "  materializedVideoSource,",
    "  PresentationRuntimeAdapter,",
    '} from "@onmark/runtime";',
    "",
    "const bindings = createDomPresentationBindings({",
    "  document,",
    ...motionOption,
    "  videoSource: materializedVideoSource,",
    "});",
    "installRuntimeHost(new PresentationRuntimeAdapter(bindings, 5_000));",
    "",
  ].join("\n");
}

// ── Artifact assembly

function presentationFiles(
  outputFiles: readonly OutputFile[],
  staging: string,
): NonEmpty<PendingFile> {
  const emitted = outputFiles.map((file) => ({
    contents: file.contents,
    path: artifactPath(staging, file.path),
  }));
  const generated = canonicalResourcePaths(emitted);
  const scripts = generated.filter((file) => file.path.endsWith(".js"));
  if (scripts.length !== 1 || scripts[0]?.path !== "presentation.js") {
    throw new BundleError(
      "build",
      "presentation must produce one JavaScript entry",
    );
  }
  const styles = generated
    .filter((file) => file.path.endsWith(".css"))
    .map((file) => file.path)
    .sort();
  const document = new TextEncoder().encode(entryDocument(styles));
  const files = [
    { contents: document, path: BUNDLE_ENTRY_POINT },
    ...generated,
  ];
  requireDistinctPaths(files);

  return canonicalFiles(files);
}

function canonicalResourcePaths(files: readonly PendingFile[]): PendingFile[] {
  // Esbuild emits uppercase Base32 hashes, while the bundle wire contract owns
  // lowercase portable paths. Normalize names and generated references at the
  // same compiler boundary.
  const renames = new Map<string, string>();
  for (const file of files) {
    if (file.path.startsWith("resources/")) {
      renames.set(file.path, file.path.toLowerCase());
    }
  }

  return files.map((file) => ({
    contents: isGeneratedText(file.path)
      ? rewriteResourceReferences(file.contents, renames)
      : file.contents,
    path: renames.get(file.path) ?? file.path,
  }));
}

function rewriteResourceReferences(
  contents: Uint8Array,
  renames: ReadonlyMap<string, string>,
): Uint8Array {
  let source = new TextDecoder().decode(contents);
  for (const [emitted, canonical] of renames) {
    source = source.replaceAll(emitted, canonical);
  }
  return new TextEncoder().encode(source);
}

function isGeneratedText(path: string): boolean {
  return path.endsWith(".css") || path.endsWith(".js");
}

function entryDocument(styles: readonly string[]): string {
  const lines = [
    "<!doctype html>",
    '<html lang="en">',
    "  <head>",
    '    <meta charset="utf-8" />',
    '    <meta name="viewport" content="width=device-width, initial-scale=1" />',
    ...styles.map((path) => `    <link rel="stylesheet" href="./${path}" />`),
    "  </head>",
    "  <body>",
    '    <script type="module" src="./presentation.js"></script>',
    "  </body>",
    "</html>",
  ];
  return `${lines.join("\n")}\n`;
}

function createManifest(
  files: NonEmpty<PendingFile>,
  temporalCapability: PresentationTemporalCapability,
  visualCapability: PresentationVisualCapability,
  frameBehavior: PresentationFrameBehavior,
): BundleManifest {
  const entries = manifestFiles(files);
  const identity = JSON.stringify({
    version: BUNDLE_VERSION,
    entryPoint: BUNDLE_ENTRY_POINT,
    temporalCapability,
    visualCapability,
    frameBehavior,
    files: entries,
  });

  return Object.freeze({
    version: BUNDLE_VERSION,
    bundleId: sha256(new TextEncoder().encode(identity)),
    entryPoint: BUNDLE_ENTRY_POINT,
    temporalCapability,
    visualCapability,
    frameBehavior,
    files: entries,
  });
}

function manifestFiles(files: NonEmpty<PendingFile>): NonEmpty<BundleFile> {
  const [first, ...rest] = files;
  return Object.freeze([manifestFile(first), ...rest.map(manifestFile)]);
}

function manifestFile(file: PendingFile): BundleFile {
  return Object.freeze({
    bytes: file.contents.byteLength,
    path: file.path,
    sha256: sha256(file.contents),
  });
}

function encodeManifest(manifest: BundleManifest): Uint8Array {
  return new TextEncoder().encode(`${JSON.stringify(manifest, null, 2)}\n`);
}

function enforceOutputLimit(
  files: readonly PendingFile[],
  manifest: Uint8Array,
  limit: number,
): void {
  let remaining = limit;
  for (const file of files) {
    remaining = consumeOutputBudget(remaining, file.contents.byteLength);
  }
  consumeOutputBudget(remaining, manifest.byteLength);
}

function consumeOutputBudget(remaining: number, bytes: number): number {
  if (bytes > remaining) {
    throw new BundleError(
      "outputLimit",
      "presentation exceeds its output-byte limit",
    );
  }
  return remaining - bytes;
}

// ── Publication and failure translation

async function writePendingFiles(
  staging: string,
  files: readonly PendingFile[],
): Promise<void> {
  for (const file of files) {
    const output = join(staging, file.path);
    await mkdir(dirname(output), { recursive: true });
    await writeFile(output, file.contents);
  }
}

async function requireAbsent(outputDirectory: string): Promise<void> {
  try {
    await lstat(outputDirectory);
  } catch (error) {
    if (isMissingPath(error)) {
      return;
    }
    throw new BundleError(
      "output",
      "failed to inspect output directory",
      error,
    );
  }
  throw new BundleError("output", "presentation output already exists");
}

async function removeFailedStaging(
  staging: string,
  failure: BundleError,
): Promise<void> {
  try {
    await rm(staging, { force: true, recursive: true });
  } catch (cleanupError) {
    throw new BundleError(
      "output",
      "failed to clean an unpublished presentation bundle",
      new AggregateError([failure, cleanupError]),
    );
  }
}

function bundleFailure(error: unknown): BundleError {
  if (error instanceof BundleError) {
    return error;
  }
  return new BundleError(
    "output",
    "failed to publish presentation bundle",
    error,
  );
}

function isMissingPath(error: unknown): boolean {
  if (!(error instanceof Error) || !("code" in error)) {
    return false;
  }
  return error.code === "ENOENT";
}

// ── Mechanical artifact values

function artifactPath(staging: string, output: string): string {
  const path = relative(staging, output);
  if (
    path.length === 0 ||
    isAbsolute(path) ||
    path === ".." ||
    path.startsWith(`..${sep}`)
  ) {
    throw new BundleError("build", "compiler produced an invalid output path");
  }
  return portablePath(path);
}

function requireDistinctPaths(files: readonly PendingFile[]): void {
  const paths = new Set(files.map((file) => file.path));
  if (paths.size !== files.length) {
    throw new BundleError("build", "compiler produced duplicate output paths");
  }
}

function portablePath(path: string): string {
  return sep === "/" ? path : path.split(sep).join("/");
}

function comparePaths(left: string, right: string): number {
  if (left < right) {
    return -1;
  }
  if (left > right) {
    return 1;
  }
  return 0;
}

function canonicalFiles(files: PendingFile[]): NonEmpty<PendingFile> {
  const [first, ...rest] = files.sort((left, right) =>
    comparePaths(left.path, right.path),
  );
  if (first === undefined) {
    throw new BundleError("build", "presentation produced no payload files");
  }
  return Object.freeze([first, ...rest]);
}

function sha256(contents: Uint8Array): string {
  const digest = createHash("sha256").update(contents).digest("hex");
  return `sha256:${digest}`;
}
