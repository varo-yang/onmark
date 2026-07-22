// Deterministic Node boundary for producing one immutable browser presentation.
// It owns esbuild and filesystem effects; runtime code remains browser-only.

import { createHash } from "node:crypto";
import { lstat, mkdir, mkdtemp, rename, rm, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { build, type OutputFile } from "esbuild";
import type {
  PresentationTemporalCapability,
  PresentationVisualCapability,
} from "@onmark/runtime/types";

import {
  BUNDLE_ENTRY_POINT,
  BUNDLE_MANIFEST_FILE,
  BUNDLE_TEMPORAL_CAPABILITIES,
  BUNDLE_VISUAL_CAPABILITIES,
  BUNDLE_VERSION,
  type BundleFile as WireBundleFile,
  type BundleManifest as WireBundleManifest,
} from "./generated/bundle-manifest.js";

const AUTHORING_ENTRY = fileURLToPath(import.meta.resolve("@onmark/authoring"));
const RUNTIME_ENTRY = fileURLToPath(import.meta.resolve("@onmark/runtime"));
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

type NonEmpty<T> = readonly [T, ...T[]];

// ── Build pipeline

/** Builds one immutable presentation through a private staging directory. */
export async function bundlePresentation(
  options: BundleOptions,
): Promise<BundleArtifact> {
  const input = validateOptions(options);
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
  input: BundleOptions,
  staging: string,
): Promise<BundleArtifact> {
  const generated = await compilePresentation(input.entryPoint, staging);
  const pending = presentationFiles(generated, staging);
  const manifest = createManifest(
    pending,
    input.temporalCapability,
    input.visualCapability,
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

function validateOptions(options: BundleOptions): BundleOptions {
  if (options.entryPoint.length === 0) {
    throw new BundleError("configuration", "entry point cannot be empty");
  }
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

  return Object.freeze({
    entryPoint: resolve(options.entryPoint),
    outputDirectory: resolve(options.outputDirectory),
    maxOutputBytes: options.maxOutputBytes,
    temporalCapability: validateTemporalCapability(options.temporalCapability),
    visualCapability: validateVisualCapability(options.visualCapability),
  });
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

async function compilePresentation(
  entryPoint: string,
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
      entryPoints: [entryPoint],
      format: "esm",
      legalComments: "none",
      loader: VISUAL_RESOURCE_LOADERS,
      minify: true,
      outdir: staging,
      platform: "browser",
      target: "es2024",
      write: false,
    });
    return result.outputFiles;
  } catch (error) {
    throw new BundleError("build", "presentation compilation failed", error);
  }
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
): BundleManifest {
  const entries = manifestFiles(files);
  const identity = JSON.stringify({
    version: BUNDLE_VERSION,
    entryPoint: BUNDLE_ENTRY_POINT,
    temporalCapability,
    visualCapability,
    files: entries,
  });

  return Object.freeze({
    version: BUNDLE_VERSION,
    bundleId: sha256(new TextEncoder().encode(identity)),
    entryPoint: BUNDLE_ENTRY_POINT,
    temporalCapability,
    visualCapability,
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
