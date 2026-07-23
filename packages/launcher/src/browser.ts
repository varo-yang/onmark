// Verified browser installation owns the desktop capture environment cache.

import { randomUUID } from "node:crypto";
import { constants } from "node:fs";
import {
  access,
  mkdir,
  mkdtemp,
  readdir,
  rename,
  rm,
  stat,
  utimes,
} from "node:fs/promises";
import { dirname, join } from "node:path";

import {
  Cache,
  InstalledBrowser,
  install,
  type Browser,
  type BrowserPlatform,
  type InstallOptions,
} from "@puppeteer/browsers";

import {
  BROWSER_BUILD,
  desktopTarget,
  type DesktopTarget,
  type ReleaseHost,
} from "./release.js";

// ── Admitted release archives

export interface BrowserInstallOptions {
  readonly cacheDirectory: string;
  readonly host: ReleaseHost;
}

type BrowserInstaller = (
  options: InstallOptions & { readonly unpack?: true },
) => Promise<InstalledBrowser>;

interface InstallLockTimings {
  readonly heartbeatMilliseconds: number;
  readonly pollMilliseconds: number;
  readonly staleMilliseconds: number;
  readonly waitMilliseconds: number;
}

const INSTALL_LOCK_TIMINGS: InstallLockTimings = {
  heartbeatMilliseconds: 10_000,
  pollMilliseconds: 200,
  staleMilliseconds: 60_000,
  waitMilliseconds: 3 * 60_000,
};
const INSTALL_STAGING_PREFIX = ".browser-install-";

export class BrowserInstallError extends Error {
  constructor(message: string, cause?: unknown) {
    super(message, cause === undefined ? undefined : { cause });
    this.name = "BrowserInstallError";
  }
}

// ── Installation pipeline

export async function ensureBrowser(
  options: BrowserInstallOptions,
  installBrowser: BrowserInstaller = install,
): Promise<string> {
  const target = desktopTarget(options.host);
  await mkdir(options.cacheDirectory, { recursive: true });

  return withInstallLock(options.cacheDirectory, async () => {
    const admittedCache = join(
      options.cacheDirectory,
      BROWSER_BUILD,
      target.sha256,
    );
    await mkdir(admittedCache, { recursive: true });
    const installed = installedBrowser(
      admittedCache,
      target.browser,
      target.browserPlatform,
    );
    if (await isExecutable(installed.executablePath, options.host.platform)) {
      return installed.executablePath;
    }

    await removeAbandonedStaging(admittedCache);
    await removeIncompleteInstall(installed);
    return installVerifiedBrowser(
      admittedCache,
      target,
      options.host.platform,
      installBrowser,
    );
  });
}

async function installVerifiedBrowser(
  cacheDirectory: string,
  target: DesktopTarget,
  hostPlatform: NodeJS.Platform,
  installBrowser: BrowserInstaller,
): Promise<string> {
  const staging = await mkdtemp(join(cacheDirectory, INSTALL_STAGING_PREFIX));
  try {
    const staged = await installBrowser({
      browser: target.browser,
      buildId: BROWSER_BUILD,
      cacheDir: staging,
      expectedHash: target.sha256,
      platform: target.browserPlatform,
    });
    if (!(await isExecutable(staged.executablePath, hostPlatform))) {
      throw new BrowserInstallError(
        "the verified browser archive did not produce an executable",
      );
    }

    const destination = installedBrowser(
      cacheDirectory,
      target.browser,
      target.browserPlatform,
    );
    await mkdir(dirname(destination.path), { recursive: true });
    await rename(staged.path, destination.path);
    if (!(await isExecutable(destination.executablePath, hostPlatform))) {
      throw new BrowserInstallError(
        "the published browser installation has no executable",
      );
    }
    return destination.executablePath;
  } catch (error) {
    if (error instanceof BrowserInstallError) {
      throw error;
    }
    throw new BrowserInstallError(
      `failed to install ${target.browser} ${BROWSER_BUILD}`,
      error,
    );
  } finally {
    await rm(staging, { force: true, recursive: true });
  }
}

async function removeAbandonedStaging(cacheDirectory: string): Promise<void> {
  const entries = await readdir(cacheDirectory, { withFileTypes: true });
  for (const entry of entries) {
    if (entry.name.startsWith(INSTALL_STAGING_PREFIX)) {
      await rm(join(cacheDirectory, entry.name), {
        force: true,
        recursive: true,
      });
    }
  }
}

function installedBrowser(
  cacheDirectory: string,
  browser: Browser,
  platform: BrowserPlatform,
): InstalledBrowser {
  return new InstalledBrowser(
    new Cache(cacheDirectory),
    browser,
    BROWSER_BUILD,
    platform,
  );
}

async function removeIncompleteInstall(
  installed: InstalledBrowser,
): Promise<void> {
  try {
    await access(installed.path, constants.F_OK);
  } catch (error) {
    if (isErrno(error, "ENOENT")) {
      return;
    }
    throw error;
  }
  await rm(installed.path, { force: true, recursive: true });
}

async function isExecutable(
  path: string,
  hostPlatform: NodeJS.Platform,
): Promise<boolean> {
  try {
    const metadata = await stat(path);
    if (!metadata.isFile()) {
      return false;
    }
    await access(
      path,
      hostPlatform === "win32" ? constants.F_OK : constants.X_OK,
    );
    return true;
  } catch {
    return false;
  }
}

// ── Cross-process installation lease

async function withInstallLock<T>(
  cacheDirectory: string,
  action: () => Promise<T>,
  timings: InstallLockTimings = INSTALL_LOCK_TIMINGS,
): Promise<T> {
  const lock = join(cacheDirectory, ".install.lock");
  await acquireInstallLock(lock, timings);
  const heartbeat = setInterval(() => {
    void refreshInstallLock(lock);
  }, timings.heartbeatMilliseconds);
  heartbeat.unref();

  try {
    return await action();
  } finally {
    clearInterval(heartbeat);
    await rm(lock, { force: true, recursive: true });
  }
}

async function refreshInstallLock(lock: string): Promise<void> {
  const now = new Date();
  try {
    await utimes(lock, now, now);
  } catch {
    // A concurrent reclaimer may already own the abandoned directory. Losing
    // that race cannot admit partial browser bytes.
  }
}

async function acquireInstallLock(
  lock: string,
  timings: InstallLockTimings,
): Promise<void> {
  const deadline = Date.now() + timings.waitMilliseconds;
  while (Date.now() < deadline) {
    try {
      await mkdir(lock);
      return;
    } catch (error) {
      if (!isErrno(error, "EEXIST")) {
        throw new BrowserInstallError(
          "failed to acquire the browser installation lock",
          error,
        );
      }
    }

    await reclaimAbandonedLock(lock, timings.staleMilliseconds);
    await delay(timings.pollMilliseconds);
  }

  throw new BrowserInstallError(
    "another Onmark process is still installing the browser",
  );
}

async function reclaimAbandonedLock(
  lock: string,
  staleMilliseconds: number,
): Promise<void> {
  let modified: number;
  try {
    modified = (await stat(lock)).mtimeMs;
  } catch (error) {
    if (isErrno(error, "ENOENT")) {
      return;
    }
    throw error;
  }
  if (Date.now() - modified <= staleMilliseconds) {
    return;
  }

  const abandoned = `${lock}.abandoned-${randomUUID()}`;
  try {
    await rename(lock, abandoned);
  } catch (error) {
    if (isErrno(error, "ENOENT")) {
      return;
    }
    throw error;
  }
  await rm(abandoned, { force: true, recursive: true });
}

function isErrno(error: unknown, code: string): boolean {
  return error instanceof Error && "code" in error && error.code === code;
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, milliseconds);
  });
}
