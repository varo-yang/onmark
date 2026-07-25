#!/usr/bin/env node
// npm release boundary resolves product tools before delegating to native CLI.

import { createRequire } from "node:module";
import { homedir, release } from "node:os";
import process from "node:process";
import { fileURLToPath } from "node:url";

import { captureEnvironment } from "./capture-environment.js";
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
  const localCapture = await captureEnvironment({
    cacheEnvironment: cacheEnvironment(process.env),
    homeDirectory: homedir(),
    host,
    osRelease: release(),
  });
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
    {
      ...process.env,
      ONMARK_CAPTURE_ENVIRONMENT_SEED: localCapture.seed,
      ONMARK_FRAME_CACHE: localCapture.cacheDirectory,
    },
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

function cacheEnvironment(environment: NodeJS.ProcessEnv): {
  readonly localAppData?: string;
  readonly windowsDirectory?: string;
  readonly xdgCacheHome?: string;
} {
  return {
    ...(environment["LOCALAPPDATA"] === undefined
      ? {}
      : { localAppData: environment["LOCALAPPDATA"] }),
    ...(environment["WINDIR"] === undefined
      ? {}
      : { windowsDirectory: environment["WINDIR"] }),
    ...(environment["XDG_CACHE_HOME"] === undefined
      ? {}
      : { xdgCacheHome: environment["XDG_CACHE_HOME"] }),
  };
}
