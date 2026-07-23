#!/usr/bin/env node
// Browser provisioner publishes one verified executable path for native CLI.

import { writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import process from "node:process";
import { parseArgs } from "node:util";

import { ensureBrowser } from "./browser.js";
import { browserCacheDirectory } from "./cache.js";

try {
  const output = outputPath(process.argv.slice(2));
  const host = { arch: process.arch, platform: process.platform };
  const localAppData = process.env["LOCALAPPDATA"];
  const xdgCacheHome = process.env["XDG_CACHE_HOME"];
  const browser = await ensureBrowser({
    cacheDirectory: browserCacheDirectory(host, homedir(), {
      ...(localAppData === undefined ? {} : { localAppData }),
      ...(xdgCacheHome === undefined ? {} : { xdgCacheHome }),
    }),
    host,
  });
  await writeFile(output, browser, { encoding: "utf8", flag: "wx" });
} catch (error) {
  const message =
    error instanceof Error
      ? error.message
      : "unknown browser installer failure";
  process.stderr.write(`onmark-browser: ${message}\n`);
  process.exitCode = 1;
}

function outputPath(arguments_: readonly string[]): string {
  const values = parseArgs({
    args: arguments_,
    allowPositionals: false,
    options: {
      output: { type: "string" },
    },
    strict: true,
  }).values;
  if (values.output === undefined || values.output.length === 0) {
    throw new Error("--output is required");
  }
  return values.output;
}
