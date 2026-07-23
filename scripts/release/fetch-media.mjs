// Release media sources are downloaded once, bounded, and admitted by digest.

import { createHash, randomUUID } from "node:crypto";
import { mkdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { basename, join, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const MAX_SOURCE_BYTES = 16 * 1024 * 1024;
const DOWNLOAD_TIMEOUT_MILLISECONDS = 2 * 60_000;
const MANIFEST_PATH = fileURLToPath(
  new URL("./media-sources.json", import.meta.url),
);

async function main() {
  const output = process.argv[2];
  if (output === undefined || process.argv.length !== 3) {
    throw new Error(
      "expected `node scripts/release/fetch-media.mjs <output-directory>`",
    );
  }

  const manifest = parseManifest(await readFile(MANIFEST_PATH, "utf8"));
  const directory = resolve(output);
  await mkdir(directory, { recursive: true });
  await Promise.all(
    manifest.sources.map((source) => admitSource(directory, source)),
  );
}

function parseManifest(contents) {
  const value = JSON.parse(contents);
  if (!isObject(value) || value.schemaVersion !== 1) {
    throw new Error("media source manifest has an unsupported schema");
  }

  const sources = ["ffmpeg", "x264", "zlib"].map((key) =>
    parseSource(value[key], key),
  );
  return Object.freeze({ sources: Object.freeze(sources) });
}

function parseSource(value, key) {
  if (
    !isObject(value) ||
    typeof value.name !== "string" ||
    basename(value.name) !== value.name ||
    typeof value.url !== "string" ||
    !Number.isSafeInteger(value.bytes) ||
    value.bytes <= 0 ||
    value.bytes > MAX_SOURCE_BYTES ||
    typeof value.sha256 !== "string" ||
    !/^[0-9a-f]{64}$/.test(value.sha256)
  ) {
    throw new Error(`media source ${key} is invalid`);
  }
  return Object.freeze({
    bytes: value.bytes,
    name: value.name,
    sha256: value.sha256,
    url: value.url,
  });
}

async function admitSource(directory, source) {
  const destination = join(directory, source.name);
  if (await matchesSource(destination, source)) {
    return;
  }

  const staging = join(directory, `.${source.name}.${randomUUID()}`);
  try {
    const response = await fetch(source.url, {
      redirect: "follow",
      signal: AbortSignal.timeout(DOWNLOAD_TIMEOUT_MILLISECONDS),
    });
    if (!response.ok) {
      throw new Error(
        `cannot download ${source.name}: HTTP ${response.status}`,
      );
    }

    const declared = response.headers.get("content-length");
    if (declared !== null && Number(declared) !== source.bytes) {
      throw new Error(`${source.name} has an unexpected content length`);
    }
    const bytes = await readBounded(response, source);
    verifySource(bytes, source);
    await writeFile(staging, bytes, { flag: "wx" });
    await rm(destination, { force: true });
    await rename(staging, destination);
  } finally {
    await rm(staging, { force: true });
  }
}

async function readBounded(response, source) {
  if (response.body === null) {
    throw new Error(`${source.name} has no response body`);
  }

  const chunks = [];
  const reader = response.body.getReader();
  let received = 0;
  while (true) {
    const result = await reader.read();
    if (result.done) {
      break;
    }
    received += result.value.byteLength;
    if (received > source.bytes) {
      await reader.cancel();
      throw new Error(`${source.name} exceeds its admitted size`);
    }
    chunks.push(result.value);
  }

  const bytes = new Uint8Array(received);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

async function matchesSource(path, source) {
  try {
    const metadata = await stat(path);
    if (!metadata.isFile() || metadata.size !== source.bytes) {
      return false;
    }
    const bytes = new Uint8Array(await readFile(path));
    return digest(bytes) === source.sha256;
  } catch (error) {
    if (isErrno(error, "ENOENT")) {
      return false;
    }
    throw error;
  }
}

function verifySource(bytes, source) {
  if (bytes.byteLength !== source.bytes || digest(bytes) !== source.sha256) {
    throw new Error(`${source.name} does not match its admitted digest`);
  }
}

function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

function isObject(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isErrno(error, code) {
  return (
    typeof error === "object" &&
    error !== null &&
    "code" in error &&
    error.code === code
  );
}

main().catch((error) => {
  const message = error instanceof Error ? error.message : String(error);
  process.stderr.write(`fetch-release-media: ${message}\n`);
  process.exitCode = 1;
});
