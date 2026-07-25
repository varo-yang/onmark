// Desktop capture identity commits local cache reuse to pixel-affecting host facts.

import { createHash, type Hash } from "node:crypto";
import type { BigIntStats } from "node:fs";
import { lstat, readdir, readlink } from "node:fs/promises";
import { posix, win32 } from "node:path";

import { frameCacheDirectory, type CacheEnvironment } from "./cache.js";
import { BROWSER_BUILD, desktopTarget, type ReleaseHost } from "./release.js";

const IDENTITY_DOMAIN = "onmark-desktop-capture-environment-v1";
const MAX_FONT_ENTRIES = 100_000;

// ── Public contract

export interface CaptureEnvironmentOptions {
  readonly cacheEnvironment: CacheEnvironment;
  readonly fontDirectories?: readonly string[];
  readonly homeDirectory: string;
  readonly host: ReleaseHost;
  readonly osRelease: string;
}

export interface LocalCaptureEnvironment {
  readonly cacheDirectory: string;
  readonly seed: string;
}

interface ScanState {
  entries: number;
}

/** Computes the release-owned seed consumed by the native capture boundary. */
export async function captureEnvironment(
  options: CaptureEnvironmentOptions,
): Promise<LocalCaptureEnvironment> {
  const target = desktopTarget(options.host);
  const hash = createHash("sha256");
  record(hash, IDENTITY_DOMAIN);
  record(hash, options.host.platform);
  record(hash, options.host.arch);
  record(hash, options.osRelease);
  record(hash, BROWSER_BUILD);
  record(hash, target.browser);
  record(hash, target.browserPlatform);
  record(hash, target.sha256);

  const directories =
    options.fontDirectories ??
    systemFontDirectories(
      options.host,
      options.homeDirectory,
      options.cacheEnvironment,
    );
  const state = { entries: 0 };
  for (const [index, directory] of directories.entries()) {
    await scanFontTree(hash, state, directory, index, "");
  }

  return Object.freeze({
    cacheDirectory: frameCacheDirectory(
      options.host,
      options.homeDirectory,
      options.cacheEnvironment,
    ),
    seed: `sha256:${hash.digest("hex")}`,
  });
}

// ── Font inventory

async function scanFontTree(
  hash: Hash,
  state: ScanState,
  root: string,
  rootIndex: number,
  relative: string,
): Promise<void> {
  const paths = optionsPath(root);
  const directory = relative.length === 0 ? root : paths.join(root, relative);
  let entries;
  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    record(hash, `unreadable:${rootIndex}:${relative}:${errorCode(error)}`);
    return;
  }
  entries.sort((left, right) => compareNames(left.name, right.name));

  for (const entry of entries) {
    state.entries += 1;
    if (state.entries > MAX_FONT_ENTRIES) {
      throw new Error(
        `font inventory exceeds the ${MAX_FONT_ENTRIES}-entry capture limit`,
      );
    }
    const child =
      relative.length === 0 ? entry.name : paths.join(relative, entry.name);
    const absolute = paths.join(root, child);
    let metadata;
    try {
      metadata = await lstat(absolute, { bigint: true });
    } catch (error) {
      record(
        hash,
        `missing:${rootIndex}:${portable(child)}:${errorCode(error)}`,
      );
      continue;
    }

    const identity = [
      fontEntryKind(metadata),
      rootIndex,
      portable(child),
      metadata.size,
      metadata.mtimeNs,
    ];
    record(hash, identity.join(":"));
    if (metadata.isDirectory()) {
      await scanFontTree(hash, state, root, rootIndex, child);
    } else if (metadata.isSymbolicLink()) {
      try {
        record(hash, `target:${await readlink(absolute)}`);
      } catch (error) {
        record(hash, `target-unreadable:${errorCode(error)}`);
      }
    }
  }
}

function fontEntryKind(metadata: BigIntStats): string {
  if (metadata.isDirectory()) {
    return "directory";
  }
  if (metadata.isSymbolicLink()) {
    return "symlink";
  }
  return "file";
}

function systemFontDirectories(
  host: ReleaseHost,
  homeDirectory: string,
  environment: CacheEnvironment,
): readonly string[] {
  switch (host.platform) {
    case "darwin":
      return [
        "/System/Library/Fonts",
        "/Library/Fonts",
        posix.join(homeDirectory, "Library", "Fonts"),
      ];
    case "linux":
      return [
        "/usr/share/fonts",
        "/usr/local/share/fonts",
        posix.join(homeDirectory, ".local", "share", "fonts"),
        posix.join(homeDirectory, ".fonts"),
      ];
    case "win32": {
      const windows = environment.windowsDirectory ?? "C:\\Windows";
      return [
        win32.join(windows, "Fonts"),
        win32.join(
          environment.localAppData ??
            win32.join(homeDirectory, "AppData", "Local"),
          "Microsoft",
          "Windows",
          "Fonts",
        ),
      ];
    }
    default:
      return [];
  }
}

// ── Canonical values

function compareNames(left: string, right: string): number {
  if (left < right) {
    return -1;
  }
  if (left > right) {
    return 1;
  }
  return 0;
}

function optionsPath(root: string): typeof posix | typeof win32 {
  return /^[A-Za-z]:\\/u.test(root) ? win32 : posix;
}

function portable(path: string): string {
  return path.replaceAll("\\", "/");
}

function record(hash: Hash, value: string): void {
  const bytes = Buffer.from(value, "utf8");
  const length = Buffer.allocUnsafe(4);
  length.writeUInt32BE(bytes.byteLength);
  hash.update(length);
  hash.update(bytes);
}

function errorCode(error: unknown): string {
  if (error instanceof Error && "code" in error) {
    return String(error.code);
  }
  return "unknown";
}
