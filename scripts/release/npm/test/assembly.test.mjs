// Product assembly tests pin the complete external runtime dependency budget.

import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import test from "node:test";

const execute = promisify(execFile);
const REPOSITORY = fileURLToPath(new URL("../../../..", import.meta.url));

test("assembles the complete pinned runtime dependency budget", async () => {
  const workspace = await mkdtemp(join(tmpdir(), "onmark-assembly-test-"));
  const product = join(workspace, "product");

  try {
    await execute(process.execPath, [
      join(REPOSITORY, "scripts/release/npm/assemble-package.mjs"),
      "--source-revision",
      "0000000000000000000000000000000000000000",
      "--output",
      product,
    ]);
    const package_ = JSON.parse(
      await readFile(join(product, "package.json"), "utf8"),
    );

    assert.deepEqual(package_.dependencies, {
      "@puppeteer/browsers": "3.0.6",
      esbuild: "0.28.1",
      gsap: "3.15.0",
      parse5: "8.0.1",
      "proxy-agent": "8.0.2",
      yauzl: "3.4.0",
    });
  } finally {
    await rm(workspace, { force: true, recursive: true });
  }
});
