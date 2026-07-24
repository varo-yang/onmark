// Process-boundary tests for the presentation bundler executable.

import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";
import test from "node:test";

const COMMAND = fileURLToPath(new URL("../src/command.js", import.meta.url));

test("publishes a bundle through the executable boundary", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "onmark-bundle-command-"));
  try {
    const document = join(workspace, "film.html");
    const outputDirectory = join(workspace, "bundle");
    await writeFile(document, "<om-film></om-film>\n", "utf8");

    const result = await invoke([
      "--html",
      document,
      "--output",
      outputDirectory,
      "--max-output-bytes",
      "1000000",
      "--frame-behavior",
      "perFrame",
      "--temporal-capability",
      "sequential",
      "--visual-capability",
      "browserComposite",
    ]);

    assert.equal(result.code, 0, result.stderr);
    assert.equal(result.stdout, "");
    const manifest = JSON.parse(
      await readFile(join(outputDirectory, "manifest.json"), "utf8"),
    ) as unknown;
    assert.equal(typeof manifest, "object");
  } finally {
    await rm(workspace, { force: true, recursive: true });
  }
});

test("resolves snapshot imports from the authored project", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "onmark-bundle-command-"));
  try {
    const snapshot = join(workspace, "snapshot", "film.html");
    const authored = join(workspace, "authored");
    const outputDirectory = join(workspace, "bundle");
    await mkdir(dirname(snapshot), { recursive: true });
    await mkdir(authored);
    await writeFile(
      snapshot,
      [
        "<om-film></om-film>",
        '<script type="module" data-om-motion>',
        '  export { motion } from "./motion.js";',
        "</script>",
      ].join("\n"),
      "utf8",
    );
    await writeFile(
      join(authored, "motion.js"),
      "export const motion = { bind() { return { effects: [], resources: [] }; } };\n",
      "utf8",
    );

    const result = await invoke([
      "--html",
      snapshot,
      "--resolve-directory",
      authored,
      "--output",
      outputDirectory,
      "--max-output-bytes",
      "1000000",
      "--frame-behavior",
      "perFrame",
      "--temporal-capability",
      "sequential",
      "--visual-capability",
      "browserComposite",
    ]);

    assert.equal(result.code, 0, result.stderr);
  } finally {
    await rm(workspace, { force: true, recursive: true });
  }
});

test("reports typed configuration failures on stderr", async () => {
  const result = await invoke([]);

  assert.equal(result.code, 2);
  assert.equal(result.stdout, "");
  assert.match(result.stderr, /^configuration: /u);
});

interface CommandResult {
  readonly code: number | null;
  readonly stderr: string;
  readonly stdout: string;
}

async function invoke(arguments_: readonly string[]): Promise<CommandResult> {
  const child = spawn(process.execPath, [COMMAND, ...arguments_], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  const [code, stdout, stderr] = await Promise.all([
    new Promise<number | null>((resolve) => child.once("close", resolve)),
    collect(child.stdout),
    collect(child.stderr),
  ]);
  return { code, stderr, stdout };
}

async function collect(stream: NodeJS.ReadableStream | null): Promise<string> {
  if (stream === null) {
    throw new Error("command output stream is unavailable");
  }
  let output = "";
  for await (const chunk of stream) {
    output += String(chunk);
  }
  return output;
}
