// Public npm package assembly stays with the Node package contract it projects.

import { createHash } from "node:crypto";
import {
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { dirname, join, relative, resolve, sep } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const MAX_PRODUCT_BYTES = 32 * 1024 * 1024;
const MAX_SOURCE_REVISION_BYTES = 256;
const MANIFEST_NAME = "onmark-release.json";
const REPOSITORY = fileURLToPath(new URL("../../", import.meta.url));
const SOURCE_TREES = Object.freeze([
  "packages/runtime/dist/src",
  "packages/authoring/dist/src",
  "packages/motion-gsap/dist/src",
  "packages/bundler/dist/src",
  "packages/launcher/dist/src",
]);
const DECLARATION_TARGETS = Object.freeze({
  "@onmark/authoring/types": "packages/authoring/dist/src/index.js",
  "@onmark/runtime/types": "packages/runtime/dist/src/index.js",
});

async function main() {
  const request = parseArguments(process.argv.slice(2));
  await refuseExistingOutput(request.output);
  await mkdir(dirname(request.output), { recursive: true });

  const staging = await mkdtemp(
    join(dirname(request.output), ".onmark-product-"),
  );
  try {
    await assembleProduct(staging, request.sourceRevision);
    await rename(staging, request.output);
  } finally {
    await rm(staging, { force: true, recursive: true });
  }
}

function parseArguments(arguments_) {
  const values = new Map();
  for (let index = 0; index < arguments_.length; index += 2) {
    const flag = arguments_[index];
    const value = arguments_[index + 1];
    if (flag === undefined || value === undefined) {
      throw new Error(`${flag ?? "release option"} requires a value`);
    }
    if (values.has(flag)) {
      throw new Error(`duplicate release option ${flag}`);
    }
    values.set(flag, value);
  }

  const sourceRevision = take(values, "--source-revision");
  const output = resolve(take(values, "--output"));
  if (values.size > 0) {
    throw new Error(`unknown release option ${values.keys().next().value}`);
  }
  if (
    Buffer.byteLength(sourceRevision) > MAX_SOURCE_REVISION_BYTES ||
    sourceRevision.trim().length === 0 ||
    containsControlCharacter(sourceRevision)
  ) {
    throw new Error(
      `--source-revision must be non-empty single-line text of at most ${MAX_SOURCE_REVISION_BYTES} bytes`,
    );
  }
  return Object.freeze({
    output,
    sourceRevision,
  });
}

function containsControlCharacter(value) {
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    if (codePoint !== undefined && (codePoint < 0x20 || codePoint === 0x7f)) {
      return true;
    }
  }
  return false;
}

function take(values, flag) {
  const value = values.get(flag);
  if (value === undefined) {
    throw new Error(`missing release option ${flag}`);
  }
  values.delete(flag);
  return value;
}

async function refuseExistingOutput(path) {
  try {
    await lstat(path);
  } catch (error) {
    if (isErrno(error, "ENOENT")) {
      return;
    }
    throw error;
  }
  throw new Error(`release output already exists: ${path}`);
}

// ── Product projection

async function assembleProduct(staging, sourceRevision) {
  const state = { files: [], retainedBytes: 0, root: staging };
  for (const source of SOURCE_TREES) {
    await copyTree(join(REPOSITORY, source), join(staging, source), state);
  }
  await copyArtifact(
    join(REPOSITORY, "packages/launcher/desktop-release.json"),
    join(staging, "packages/launcher/dist/desktop-release.json"),
    state,
  );
  for (const name of ["LICENSE", "README.md"]) {
    await copyArtifact(join(REPOSITORY, name), join(staging, name), state);
  }

  await writeArtifact(
    join(staging, "package.json"),
    json(await productPackage()),
    state,
  );
  state.files.sort();
  const manifest = await productManifest(staging, sourceRevision, state.files);
  await writeArtifact(join(staging, MANIFEST_NAME), json(manifest), state);
}

async function copyTree(source, destination, state) {
  const metadata = await lstat(source);
  if (!metadata.isDirectory()) {
    throw new Error(`product source is not a directory: ${source}`);
  }

  const entries = await readdir(source, { withFileTypes: true });
  entries.sort((left, right) => compareText(left.name, right.name));
  for (const entry of entries) {
    const sourcePath = join(source, entry.name);
    const destinationPath = join(destination, entry.name);
    if (entry.isDirectory()) {
      await copyTree(sourcePath, destinationPath, state);
    } else if (entry.isFile()) {
      await copyArtifact(sourcePath, destinationPath, state);
    } else {
      throw new Error(`product source cannot contain links: ${sourcePath}`);
    }
  }
}

async function copyArtifact(source, destination, state) {
  const metadata = await lstat(source);
  if (!metadata.isFile()) {
    throw new Error(`product source is not a regular file: ${source}`);
  }
  ensureCapacity(state, metadata.size);
  await mkdir(dirname(destination), { recursive: true });
  await copyFile(source, destination);
  await projectDeclaration(destination, state.root);
  retain(state, (await lstat(destination)).size);
  state.files.push(productPath(state.root, destination));
}

async function writeArtifact(path, contents, state) {
  retain(state, Buffer.byteLength(contents));
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, contents, { flag: "wx" });
  state.files.push(productPath(state.root, path));
}

function retain(state, bytes) {
  ensureCapacity(state, bytes);
  state.retainedBytes += bytes;
}

function ensureCapacity(state, bytes) {
  const retainedBytes = state.retainedBytes + bytes;
  if (
    !Number.isSafeInteger(retainedBytes) ||
    retainedBytes > MAX_PRODUCT_BYTES
  ) {
    throw new Error(
      `desktop product exceeds its ${MAX_PRODUCT_BYTES}-byte retained limit`,
    );
  }
}

async function projectDeclaration(path, productRoot) {
  if (!path.endsWith(".d.ts")) {
    return;
  }

  let contents = await readFile(path, "utf8");
  for (const [specifier, target] of Object.entries(DECLARATION_TARGETS)) {
    const projected = moduleSpecifier(dirname(path), join(productRoot, target));
    contents = contents.replaceAll(`"${specifier}"`, `"${projected}"`);
  }
  if (/["']@onmark\//.test(contents)) {
    throw new Error(`public declaration retains an internal import: ${path}`);
  }
  await writeFile(path, contents);
}

function moduleSpecifier(from, target) {
  const path = relative(from, target).split(sep).join("/");
  return path.startsWith(".") ? path : `./${path}`;
}

function productPath(root, path) {
  return relative(root, path).split(sep).join("/");
}

// ── Canonical metadata

async function productPackage() {
  const [authoring, bundler, launcher, motion, runtime, release] =
    await Promise.all([
      packageMetadata("authoring"),
      packageMetadata("bundler"),
      packageMetadata("launcher"),
      packageMetadata("motion-gsap"),
      packageMetadata("runtime"),
      desktopRelease(),
    ]);
  requireOneVersion([authoring, bundler, launcher, motion, runtime]);

  const optionalDependencies = {};
  for (const target of Object.keys(release.targets)) {
    optionalDependencies[`@onmark/cli-${target}`] = launcher.version;
  }
  return {
    name: "onmark",
    version: launcher.version,
    description: "Screenplay-first deterministic browser video compiler",
    license: "MIT",
    repository: "https://github.com/varo-yang/onmark",
    type: "module",
    engines: { node: ">=22.12" },
    bin: { onmark: "./packages/launcher/dist/src/command.js" },
    exports: {
      "./authoring": {
        types: "./packages/authoring/dist/src/index.d.ts",
        default: "./packages/authoring/dist/src/index.js",
      },
      "./motion/gsap": {
        types: "./packages/motion-gsap/dist/src/index.d.ts",
        default: "./packages/motion-gsap/dist/src/index.js",
      },
    },
    imports: {
      "#onmark-authoring": "./packages/authoring/dist/src/index.js",
      "#onmark-bundler-command": "./packages/bundler/dist/src/command.js",
      "#onmark-runtime": "./packages/runtime/dist/src/index.js",
    },
    files: ["packages", "LICENSE", "README.md", MANIFEST_NAME],
    dependencies: {
      "@puppeteer/browsers": dependency(launcher, "@puppeteer/browsers"),
      esbuild: dependency(bundler, "esbuild"),
      gsap: dependency(motion, "gsap"),
      "proxy-agent": dependency(launcher, "proxy-agent"),
      yauzl: dependency(launcher, "yauzl"),
    },
    optionalDependencies: sortedObject(optionalDependencies),
  };
}

async function productManifest(root, sourceRevision, files) {
  const package_ = await readJson(join(root, "package.json"));
  return {
    schemaVersion: 1,
    packageName: package_.name,
    version: package_.version,
    sourceRevision,
    files: await Promise.all(
      files.map(async (path) => {
        const artifact = join(root, path);
        return {
          path,
          bytes: (await lstat(artifact)).size,
          sha256: await sha256(artifact),
        };
      }),
    ),
  };
}

async function packageMetadata(name) {
  return readJson(join(REPOSITORY, "packages", name, "package.json"));
}

async function desktopRelease() {
  const path = join(REPOSITORY, "packages/launcher/desktop-release.json");
  const release = await readJson(path);
  if (
    release.schemaVersion !== 1 ||
    typeof release.browserBuild !== "string" ||
    release.browserBuild.length === 0 ||
    !isObject(release.targets) ||
    Object.keys(release.targets).length === 0
  ) {
    throw new Error(`desktop release contract is malformed: ${path}`);
  }
  for (const target of Object.keys(release.targets)) {
    if (!/^(darwin|linux|win32)-(arm64|x64)$/.test(target)) {
      throw new Error(`desktop release target is malformed: ${target}`);
    }
  }
  return release;
}

function requireOneVersion(packages) {
  const versions = new Set(packages.map((package_) => package_.version));
  if (versions.size !== 1) {
    throw new Error("desktop product packages do not share one version");
  }
}

function dependency(package_, name) {
  const value = package_.dependencies?.[name];
  if (typeof value !== "string" || value.startsWith("workspace:")) {
    throw new Error(`${package_.name} does not pin release dependency ${name}`);
  }
  return value;
}

function sortedObject(value) {
  return Object.fromEntries(
    Object.entries(value).sort(([left], [right]) => compareText(left, right)),
  );
}

function compareText(left, right) {
  if (left < right) {
    return -1;
  }
  if (left > right) {
    return 1;
  }
  return 0;
}

async function readJson(path) {
  const value = JSON.parse(await readFile(path, "utf8"));
  if (!isObject(value)) {
    throw new Error(`release metadata is not an object: ${path}`);
  }
  return value;
}

async function sha256(path) {
  return createHash("sha256")
    .update(await readFile(path))
    .digest("hex");
}

function json(value) {
  return `${JSON.stringify(value, null, 2)}\n`;
}

function isObject(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isErrno(error, code) {
  return isObject(error) && error.code === code;
}

main().catch((error) => {
  const message = error instanceof Error ? error.message : String(error);
  process.stderr.write(`assemble-desktop-package: ${message}\n`);
  process.exitCode = 1;
});
