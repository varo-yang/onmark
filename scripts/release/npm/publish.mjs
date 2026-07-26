// npm publication validates admitted archives before crossing the registry boundary.

import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { readdir } from "node:fs/promises";
import { resolve } from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

import {
  PUBLICATION_PACKAGE_COUNT,
  admitPublication,
  publishAdmittedRelease,
} from "./publication.mjs";

const execFileAsync = promisify(execFile);
const MAX_NPM_OUTPUT_BYTES = 2 * 1024 * 1024;
const NPM_TIMEOUT_MILLISECONDS = 2 * 60_000;

async function main() {
  const request = parseArguments(process.argv.slice(2));
  const archives = await inspectArchives(request);
  const release = admitPublication(request.tag, archives);

  if (request.mode === "check") {
    writeSummary(
      release.packages.map((archive) => ({
        name: archive.name,
        outcome: "admitted",
      })),
    );
    return;
  }

  const registry = npmRegistry(request.npmCli, request.directory);
  writeSummary(await publishAdmittedRelease(release, registry));
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

  const mode = take(values, "--mode");
  if (mode !== "check" && mode !== "publish") {
    throw new Error("--mode must be check or publish");
  }
  const request = Object.freeze({
    directory: resolve(take(values, "--directory")),
    mode,
    npmCli: resolve(take(values, "--npm-cli")),
    tag: take(values, "--tag"),
  });
  if (values.size > 0) {
    throw new Error(`unknown release option ${values.keys().next().value}`);
  }
  return request;
}

function take(values, flag) {
  const value = values.get(flag);
  if (value === undefined) {
    throw new Error(`missing release option ${flag}`);
  }
  values.delete(flag);
  return value;
}

// ── Archive admission

async function inspectArchives(request) {
  const entries = await readdir(request.directory, { withFileTypes: true });
  const archives = entries
    .filter((entry) => entry.isFile() && entry.name.endsWith(".tgz"))
    .map((entry) => resolve(request.directory, entry.name))
    .sort();
  if (archives.length !== PUBLICATION_PACKAGE_COUNT) {
    throw new Error(
      `release directory must contain exactly ${PUBLICATION_PACKAGE_COUNT} package archives`,
    );
  }
  return Promise.all(
    archives.map((archive) =>
      inspectArchive(request.npmCli, request.directory, archive),
    ),
  );
}

async function inspectArchive(npmCli, directory, archive) {
  const result = await invokeNpm(
    npmCli,
    ["pack", "--dry-run", "--json", archive],
    directory,
  );
  requireSuccess(result, `inspect ${archive}`);

  const document = parseJson(result.stdout, `npm metadata for ${archive}`);
  const records =
    typeof document === "object" && document !== null
      ? Object.values(document)
      : [];
  const [record] = records;
  if (
    records.length !== 1 ||
    typeof record !== "object" ||
    record === null ||
    typeof record.name !== "string" ||
    typeof record.version !== "string" ||
    typeof record.integrity !== "string"
  ) {
    throw new Error(`npm returned invalid package metadata for ${archive}`);
  }
  return Object.freeze({
    integrity: record.integrity,
    name: record.name,
    path: archive,
    version: record.version,
  });
}

// ── Registry boundary

function npmRegistry(npmCli, directory) {
  return Object.freeze({
    async integrity(name, version) {
      const result = await invokeNpm(
        npmCli,
        ["view", `${name}@${version}`, "dist.integrity", "--json"],
        directory,
      );
      if (result.status === 0) {
        const integrity = parseJson(
          result.stdout,
          `registry integrity for ${name}@${version}`,
        );
        if (typeof integrity !== "string") {
          throw new Error(
            `registry returned no integrity for ${name}@${version}`,
          );
        }
        return integrity;
      }
      if (isMissingPackage(result.stdout)) {
        return undefined;
      }
      throw npmFailure(result, `look up ${name}@${version}`);
    },

    async publish(archive, distributionTag) {
      const result = await invokeNpm(
        npmCli,
        ["publish", archive, "--access", "public", "--tag", distributionTag],
        directory,
      );
      requireSuccess(result, `publish ${archive}`);
    },
  });
}

async function invokeNpm(npmCli, arguments_, directory) {
  try {
    const result = await execFileAsync(
      process.execPath,
      [npmCli, ...arguments_],
      {
        cwd: directory,
        encoding: "utf8",
        killSignal: "SIGKILL",
        maxBuffer: MAX_NPM_OUTPUT_BYTES,
        timeout: NPM_TIMEOUT_MILLISECONDS,
      },
    );
    return Object.freeze({
      status: 0,
      stderr: result.stderr,
      stdout: result.stdout,
    });
  } catch (error) {
    if (isProcessExit(error)) {
      return Object.freeze({
        status: error.code,
        stderr: error.stderr,
        stdout: error.stdout,
      });
    }
    throw error;
  }
}

function isProcessExit(error) {
  return (
    typeof error === "object" &&
    error !== null &&
    typeof error.code === "number" &&
    typeof error.stdout === "string" &&
    typeof error.stderr === "string"
  );
}

function isMissingPackage(stdout) {
  try {
    const document = JSON.parse(stdout);
    return (
      typeof document === "object" &&
      document !== null &&
      "error" in document &&
      typeof document.error === "object" &&
      document.error !== null &&
      document.error.code === "E404"
    );
  } catch {
    return false;
  }
}

function requireSuccess(result, operation) {
  if (result.status !== 0) {
    throw npmFailure(result, operation);
  }
}

function npmFailure(result, operation) {
  const details = result.stderr.trim() || result.stdout.trim();
  return new Error(
    details.length === 0
      ? `npm failed to ${operation} with status ${result.status}`
      : `npm failed to ${operation}: ${details}`,
  );
}

function parseJson(contents, label) {
  try {
    return JSON.parse(contents);
  } catch (error) {
    throw new Error(`${label} is not valid JSON`, { cause: error });
  }
}

function writeSummary(outcomes) {
  for (const { name, outcome } of outcomes) {
    process.stdout.write(`${name}: ${outcome}\n`);
  }
}

const entry = process.argv[1];
if (
  entry !== undefined &&
  import.meta.url === pathToFileURL(resolve(entry)).href
) {
  main().catch((error) => {
    const message = error instanceof Error ? error.stack : String(error);
    process.stderr.write(`publish-release: ${message}\n`);
    process.exitCode = 1;
  });
}
