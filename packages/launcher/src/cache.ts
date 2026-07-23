// Host cache selection keeps platform conventions out of browser installation.

import { posix, win32 } from "node:path";

import type { ReleaseHost } from "./release.js";

export interface CacheEnvironment {
  readonly localAppData?: string;
  readonly xdgCacheHome?: string;
}

export function browserCacheDirectory(
  host: ReleaseHost,
  homeDirectory: string,
  environment: CacheEnvironment,
): string {
  switch (host.platform) {
    case "darwin":
      return posix.join(
        homeDirectory,
        "Library",
        "Caches",
        "onmark",
        "browser",
      );
    case "linux":
      return posix.join(
        environment.xdgCacheHome ?? posix.join(homeDirectory, ".cache"),
        "onmark",
        "browser",
      );
    case "win32":
      return win32.join(
        environment.localAppData ??
          win32.join(homeDirectory, "AppData", "Local"),
        "onmark",
        "browser",
      );
    default:
      return posix.join(homeDirectory, ".onmark", "browser");
  }
}
