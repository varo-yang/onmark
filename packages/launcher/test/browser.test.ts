// Browser tests prove checksum forwarding and atomic cache publication.

import assert from "node:assert/strict";
import {
  access,
  mkdir,
  mkdtemp,
  rm,
  utimes,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";

import {
  Browser,
  Cache,
  InstalledBrowser,
  type InstallOptions,
} from "@puppeteer/browsers";

import { ensureBrowser } from "../src/browser.js";
import { BROWSER_BUILD, desktopTarget } from "../src/release.js";

test("installs the pinned archive once and reuses the published browser", async () => {
  const root = await mkdtemp(join(tmpdir(), "onmark-launcher-"));
  try {
    const calls: InstallOptions[] = [];
    const installBrowser = async (
      options: InstallOptions & { readonly unpack?: true },
    ): Promise<InstalledBrowser> => {
      calls.push(options);
      return writeBrowserFixture(options);
    };
    const options = browserOptions(root);
    const target = desktopTarget(options.host);

    const first = await ensureBrowser(options, installBrowser);
    const second = await ensureBrowser(options, installBrowser);

    assert.equal(first, second);
    assert.ok(first.includes(`${BROWSER_BUILD}/${target.sha256}`));
    assert.equal(calls.length, 1);
    assert.equal(calls[0]?.browser, Browser.CHROME);
    assert.equal(calls[0]?.buildId, BROWSER_BUILD);
    assert.equal(calls[0]?.expectedHash, target.sha256);
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

test("keeps Linux on the admitted headless-shell backend", async () => {
  const root = await mkdtemp(join(tmpdir(), "onmark-launcher-"));
  try {
    const host = { arch: "x64", platform: "linux" } as const;
    let call: InstallOptions | undefined;
    await ensureBrowser(
      {
        cacheDirectory: join(root, "cache"),
        host,
      },
      async (options) => {
        call = options;
        return writeBrowserFixture(options);
      },
    );

    assert.equal(call?.browser, Browser.CHROMEHEADLESSSHELL);
    assert.equal(call?.expectedHash, desktopTarget(host).sha256);
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

test("keeps Windows on the admitted portable Chrome backend", async () => {
  const root = await mkdtemp(join(tmpdir(), "onmark-launcher-"));
  try {
    const host = { arch: "x64", platform: "win32" } as const;
    let call: InstallOptions | undefined;
    await ensureBrowser(
      {
        cacheDirectory: join(root, "cache"),
        host,
      },
      async (options) => {
        call = options;
        return writeBrowserFixture(options);
      },
    );

    assert.equal(call?.browser, Browser.CHROME);
    assert.equal(call?.expectedHash, desktopTarget(host).sha256);
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

test("serializes concurrent installers around one published browser", async () => {
  const root = await mkdtemp(join(tmpdir(), "onmark-launcher-"));
  try {
    const started = Promise.withResolvers<void>();
    const release = Promise.withResolvers<void>();
    let calls = 0;
    const installBrowser = async (
      options: InstallOptions & { readonly unpack?: true },
    ): Promise<InstalledBrowser> => {
      calls += 1;
      started.resolve();
      await release.promise;
      return writeBrowserFixture(options);
    };

    const first = ensureBrowser(browserOptions(root), installBrowser);
    await started.promise;
    const second = ensureBrowser(browserOptions(root), installBrowser);
    release.resolve();

    assert.equal(await first, await second);
    assert.equal(calls, 1);
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

test("removes an interrupted installation before admitting new bytes", async () => {
  const root = await mkdtemp(join(tmpdir(), "onmark-launcher-"));
  try {
    const abandoned = join(
      root,
      "cache",
      BROWSER_BUILD,
      desktopTarget(browserOptions(root).host).sha256,
      ".browser-install-abandoned",
    );
    await mkdir(abandoned, { recursive: true });

    await ensureBrowser(browserOptions(root), writeBrowserFixture);

    await assert.rejects(access(abandoned));
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

test("reclaims an abandoned installer within the caller wait budget", async () => {
  const root = await mkdtemp(join(tmpdir(), "onmark-launcher-"));
  try {
    const lock = join(root, "cache", ".install.lock");
    await mkdir(lock, { recursive: true });
    const abandonedAt = new Date(Date.now() - 2 * 60_000);
    await utimes(lock, abandonedAt, abandonedAt);

    await ensureBrowser(browserOptions(root), writeBrowserFixture);

    await assert.rejects(access(lock));
  } finally {
    await rm(root, { force: true, recursive: true });
  }
});

function browserOptions(root: string) {
  return {
    cacheDirectory: join(root, "cache"),
    host: { arch: "arm64", platform: "darwin" } as const,
  };
}

async function writeBrowserFixture(
  options: InstallOptions,
): Promise<InstalledBrowser> {
  if (options.platform === undefined) {
    throw new Error("the browser fixture requires an admitted platform");
  }
  const installed = new InstalledBrowser(
    new Cache(options.cacheDir),
    options.browser,
    options.buildId,
    options.platform,
  );
  await mkdir(dirname(installed.executablePath), { recursive: true });
  await writeFile(installed.executablePath, "browser", { mode: 0o755 });
  return installed;
}
