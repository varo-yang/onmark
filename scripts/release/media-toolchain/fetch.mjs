// Release media sources are downloaded once, bounded, and admitted by digest.

import { createHash, randomUUID } from "node:crypto";
import { mkdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { basename, join, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import { withObservedCleanup } from "../observed-cleanup.mjs";

const MAX_SOURCE_BYTES = 16 * 1024 * 1024;
const MAX_DOWNLOAD_REDIRECTS = 5;
const DOWNLOAD_ATTEMPT_TIMEOUT_MILLISECONDS = 30_000;
const DOWNLOAD_RETRY_DELAYS_MILLISECONDS = Object.freeze([
  1_000, 4_000, 12_000,
]);
const REDIRECT_STATUSES = Object.freeze([301, 302, 303, 307, 308]);
const MANIFEST_PATH = fileURLToPath(new URL("./sources.json", import.meta.url));

class DownloadContractError extends Error {}

async function main() {
  const output = process.argv[2];
  if (output === undefined || process.argv.length !== 3) {
    throw new Error(
      "expected `node scripts/release/media-toolchain/fetch.mjs <output-directory>`",
    );
  }

  const manifest = parseManifest(await readFile(MANIFEST_PATH, "utf8"));
  const directory = resolve(output);
  await mkdir(directory, { recursive: true });
  await Promise.all(
    manifest.sources.map((source) => admitSource(directory, source)),
  );
}

// ── Source contract

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
    !value.url.startsWith("https://") ||
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

// ── Bounded download

async function admitSource(directory, source) {
  const destination = join(directory, source.name);
  if (await matchesSource(destination, source)) {
    return;
  }

  const staging = join(directory, `.${source.name}.${randomUUID()}`);
  await withObservedCleanup(
    async () => {
      const response = await fetchSource(source);
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
    },
    () => rm(staging, { force: true }),
    `downloading ${source.name} failed and staging cleanup also failed`,
  );
}

export async function fetchSource(
  source,
  request = globalThis.fetch,
  pause = wait,
) {
  for (let attempt = 0; ; attempt += 1) {
    const retryDelay = DOWNLOAD_RETRY_DELAYS_MILLISECONDS[attempt];
    try {
      const response = await fetchSourceAttempt(source, request);
      if (!isRetryableStatus(response.status) || retryDelay === undefined) {
        return response;
      }
      await response.body?.cancel();
    } catch (error) {
      if (error instanceof DownloadContractError) {
        throw error;
      }
      if (retryDelay === undefined) {
        throw downloadError(source, error);
      }
    }
    await pause(retryDelay);
  }
}

async function fetchSourceAttempt(source, request) {
  const signal = AbortSignal.timeout(DOWNLOAD_ATTEMPT_TIMEOUT_MILLISECONDS);
  let url = new URL(source.url);

  for (let redirects = 0; redirects <= MAX_DOWNLOAD_REDIRECTS; redirects += 1) {
    // GitLab's archive API rejects CORS fetch metadata even for a direct
    // server-side download. Each redirect becomes a new same-origin request.
    const response = await request(url, {
      mode: "same-origin",
      redirect: "manual",
      signal,
    });
    if (!REDIRECT_STATUSES.includes(response.status)) {
      return response;
    }

    const location = response.headers.get("location");
    await response.body?.cancel();
    if (location === null) {
      throw new DownloadContractError(
        `${source.name} redirects without a location`,
      );
    }
    url = new URL(location, url);
    if (url.protocol !== "https:") {
      throw new DownloadContractError(`${source.name} redirects outside HTTPS`);
    }
  }

  throw new DownloadContractError(
    `${source.name} exceeds its ${MAX_DOWNLOAD_REDIRECTS}-redirect limit`,
  );
}

function isRetryableStatus(status) {
  return (
    status === 408 ||
    status === 425 ||
    status === 429 ||
    (status >= 500 && status <= 599)
  );
}

function downloadError(source, cause) {
  const detail = cause instanceof Error ? cause.message : String(cause);
  return new Error(`cannot download ${source.name}: ${detail}`, { cause });
}

function wait(milliseconds) {
  return new Promise((resolvePromise) => {
    setTimeout(resolvePromise, milliseconds);
  });
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

if (
  process.argv[1] !== undefined &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((error) => {
    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`fetch-release-media: ${message}\n`);
    process.exitCode = 1;
  });
}
