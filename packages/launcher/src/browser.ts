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
  rmdir,
  stat,
  utimes,
  writeFile,
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

interface InstallLease {
  readonly directory: string;
  readonly marker: string;
}

interface BrowserInstallation {
  readonly cacheDirectory: string;
  readonly hostPlatform: NodeJS.Platform;
  readonly installBrowser: BrowserInstaller;
  readonly target: DesktopTarget;
}

const INSTALL_LOCK_TIMINGS: InstallLockTimings = {
  heartbeatMilliseconds: 10_000,
  pollMilliseconds: 200,
  staleMilliseconds: 60_000,
  waitMilliseconds: 3 * 60_000,
};
const INSTALL_LOCK_OWNER_PREFIX = ".owner-";
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

  return withInstallLock(options.cacheDirectory, async (lease) => {
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

    await verifyInstallLease(lease);
    await removeAbandonedStaging(admittedCache);
    await removeIncompleteInstall(installed);
    return installVerifiedBrowser(lease, {
      cacheDirectory: admittedCache,
      hostPlatform: options.host.platform,
      installBrowser,
      target,
    });
  });
}

async function installVerifiedBrowser(
  lease: InstallLease,
  installation: BrowserInstallation,
): Promise<string> {
  const staging = await mkdtemp(
    join(installation.cacheDirectory, INSTALL_STAGING_PREFIX),
  );
  return withObservedCleanup(
    () => publishVerifiedBrowser(lease, installation, staging),
    () => rm(staging, { force: true, recursive: true }),
    "browser installation failed and staging cleanup also failed",
  );
}

async function publishVerifiedBrowser(
  lease: InstallLease,
  installation: BrowserInstallation,
  staging: string,
): Promise<string> {
  const { cacheDirectory, hostPlatform, installBrowser, target } = installation;
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
    await verifyInstallLease(lease);
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
  action: (lease: InstallLease) => Promise<T>,
  timings: InstallLockTimings = INSTALL_LOCK_TIMINGS,
): Promise<T> {
  const lock = join(cacheDirectory, ".install.lock");
  const lease = await acquireInstallLock(lock, timings);
  const heartbeat = setInterval(() => {
    void refreshInstallLease(lease);
  }, timings.heartbeatMilliseconds);
  heartbeat.unref();

  return withObservedCleanup(
    () => action(lease),
    async () => {
      clearInterval(heartbeat);
      await releaseInstallLock(lease);
    },
    "browser installation failed and lease cleanup also failed",
  );
}

async function refreshInstallLease(lease: InstallLease): Promise<void> {
  const now = new Date();
  try {
    await utimes(lease.marker, now, now);
  } catch {
    // A reclaimer may have moved this exact lease. The owner-specific marker
    // prevents the resumed heartbeat from touching a successor's lock.
  }
}

async function verifyInstallLease(lease: InstallLease): Promise<void> {
  const now = new Date();
  try {
    await utimes(lease.marker, now, now);
  } catch (error) {
    throw new BrowserInstallError(
      "browser installation lease was lost before cache publication",
      error,
    );
  }
}

async function acquireInstallLock(
  lock: string,
  timings: InstallLockTimings,
): Promise<InstallLease> {
  const deadline = Date.now() + timings.waitMilliseconds;
  while (Date.now() < deadline) {
    const lease = await tryAcquireInstallLock(lock);
    if (lease !== undefined) {
      return lease;
    }

    await reclaimAbandonedLock(lock, timings.staleMilliseconds);
    await delay(timings.pollMilliseconds);
  }

  throw new BrowserInstallError(
    "another Onmark process is still installing the browser",
  );
}

async function tryAcquireInstallLock(
  lock: string,
): Promise<InstallLease | undefined> {
  const owner = randomUUID();
  const candidate = `${lock}.candidate-${owner}`;
  const markerName = `${INSTALL_LOCK_OWNER_PREFIX}${owner}`;
  await mkdir(candidate);
  return withObservedCleanup(
    async () => {
      try {
        await writeFile(join(candidate, markerName), "", { flag: "wx" });
      } catch (error) {
        throw new BrowserInstallError(
          "failed to prepare the browser installation lock",
          error,
        );
      }

      try {
        await rename(candidate, lock);
      } catch (error) {
        if (isLockContention(error) || (await installLockExists(lock))) {
          return undefined;
        }
        throw new BrowserInstallError(
          "failed to acquire the browser installation lock",
          error,
        );
      }

      return {
        directory: lock,
        marker: join(lock, markerName),
      };
    },
    () => rm(candidate, { force: true, recursive: true }),
    "browser installation lock acquisition and candidate cleanup both failed",
  );
}

async function releaseInstallLock(lease: InstallLease): Promise<void> {
  await rm(lease.marker, { force: true });
  try {
    await rmdir(lease.directory);
  } catch (error) {
    if (
      isErrno(error, "ENOENT") ||
      isErrno(error, "ENOTEMPTY") ||
      isErrno(error, "EEXIST")
    ) {
      return;
    }
    throw error;
  }
}

async function reclaimAbandonedLock(
  lock: string,
  staleMilliseconds: number,
): Promise<void> {
  let modified: number;
  try {
    modified = await installLockModifiedAt(lock);
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

async function installLockModifiedAt(lock: string): Promise<number> {
  const owners = (await readdir(lock))
    .filter((entry) => entry.startsWith(INSTALL_LOCK_OWNER_PREFIX))
    .sort();
  const owner = owners[0];
  if (owners.length === 1 && owner !== undefined) {
    return (await stat(join(lock, owner))).mtimeMs;
  }
  return (await stat(lock)).mtimeMs;
}

function isLockContention(error: unknown): boolean {
  return isErrno(error, "EEXIST") || isErrno(error, "ENOTEMPTY");
}

async function installLockExists(lock: string): Promise<boolean> {
  try {
    return (await stat(lock)).isDirectory();
  } catch (error) {
    if (isErrno(error, "ENOENT")) {
      return false;
    }
    throw new BrowserInstallError(
      "failed to inspect the browser installation lock",
      error,
    );
  }
}

function isErrno(error: unknown, code: string): boolean {
  return error instanceof Error && "code" in error && error.code === code;
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, milliseconds);
  });
}

async function withObservedCleanup<T>(
  action: () => Promise<T>,
  cleanup: () => Promise<void>,
  combinedMessage: string,
): Promise<T> {
  let outcome:
    | { readonly status: "fulfilled"; readonly value: T }
    | { readonly reason: unknown; readonly status: "rejected" };
  try {
    outcome = { status: "fulfilled", value: await action() };
  } catch (error) {
    outcome = { reason: error, status: "rejected" };
  }

  try {
    await cleanup();
  } catch (cleanupError) {
    if (outcome.status === "fulfilled") {
      throw cleanupError;
    }
    throw new AggregateError([outcome.reason, cleanupError], combinedMessage);
  }
  if (outcome.status === "rejected") {
    throw outcome.reason;
  }
  return outcome.value;
}
