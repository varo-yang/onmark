#!/usr/bin/env node
// npm release boundary resolves product tools before delegating to native CLI.

import { createRequire } from "node:module";
import process from "node:process";
import { fileURLToPath } from "node:url";

import { platformArtifact } from "./platform.js";
import { runNative } from "./native.js";

try {
  const host = { arch: process.arch, platform: process.platform };
  const releasePackage = createRequire(
    new URL("../../../../package.json", import.meta.url),
  );
  const artifact = platformArtifact(
    host,
    createRequire(import.meta.url).resolve,
  );
  const browserProvisioner = fileURLToPath(
    new URL("./browser-command.js", import.meta.url),
  );
  const bundler = releasePackage.resolve("#onmark-bundler-command");
  const result = await runNative(
    process.argv.slice(2),
    {
      browserProvisioner,
      bundler,
      ffmpeg: artifact.ffmpeg,
      ffprobe: artifact.ffprobe,
      nativeCli: artifact.nativeCli,
      node: process.execPath,
    },
    process.env,
  );

  if (result.signal !== null) {
    process.kill(process.pid, result.signal);
  } else {
    process.exitCode = result.code ?? 1;
  }
} catch (error) {
  const message =
    error instanceof Error ? error.message : "unknown launcher failure";
  process.stderr.write(`onmark: ${message}\n`);
  process.exitCode = 1;
}
