// Process-boundary tests for the presentation bundler executable.

import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";
import test from "node:test";

const COMMAND = fileURLToPath(new URL("../src/command.js", import.meta.url));

test("publishes a bundle through the executable boundary", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "onmark-bundle-command-"));
  try {
    const entryPoint = join(workspace, "presentation.ts");
    const outputDirectory = join(workspace, "bundle");
    await writeFile(
      entryPoint,
      `
        import { installRuntimeHost } from "@onmark/runtime";
        installRuntimeHost({
          async load() {},
          async prepare() {},
          async seek() {},
          async confirm() {},
          async dispose() {},
        });
      `,
      "utf8",
    );

    const result = await invoke([
      "--entry",
      entryPoint,
      "--output",
      outputDirectory,
      "--max-output-bytes",
      "1000000",
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
