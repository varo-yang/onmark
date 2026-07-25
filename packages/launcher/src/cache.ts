// Host cache selection keeps platform conventions out of browser installation.

import { posix, win32 } from "node:path";

import type { ReleaseHost } from "./release.js";

export interface CacheEnvironment {
  readonly localAppData?: string;
  readonly windowsDirectory?: string;
  readonly xdgCacheHome?: string;
}

export function frameCacheDirectory(
  host: ReleaseHost,
  homeDirectory: string,
  environment: CacheEnvironment,
): string {
  return cacheDirectory(host, homeDirectory, environment, "frames");
}

export function browserCacheDirectory(
  host: ReleaseHost,
  homeDirectory: string,
  environment: CacheEnvironment,
): string {
  return cacheDirectory(host, homeDirectory, environment, "browser");
}

function cacheDirectory(
  host: ReleaseHost,
  homeDirectory: string,
  environment: CacheEnvironment,
  leaf: string,
): string {
  switch (host.platform) {
    case "darwin":
      return posix.join(homeDirectory, "Library", "Caches", "onmark", leaf);
    case "linux":
      return posix.join(
        environment.xdgCacheHome ?? posix.join(homeDirectory, ".cache"),
        "onmark",
        leaf,
      );
    case "win32":
      return win32.join(
        environment.localAppData ??
          win32.join(homeDirectory, "AppData", "Local"),
        "onmark",
        leaf,
      );
    default:
      return posix.join(homeDirectory, ".onmark", leaf);
  }
}
