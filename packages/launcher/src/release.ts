// Desktop release metadata is the single source for supported platform artifacts.

import { Browser, BrowserPlatform } from "@puppeteer/browsers";

import manifest from "../desktop-release.json" with { type: "json" };

export interface ReleaseHost {
  readonly arch: NodeJS.Architecture;
  readonly platform: NodeJS.Platform;
}

export interface DesktopTarget {
  readonly browser: Browser;
  readonly browserPlatform: BrowserPlatform;
  readonly packageName: string;
  readonly sha256: string;
}

export const BROWSER_BUILD = manifest.browserBuild;

export class UnsupportedReleaseTargetError extends Error {
  constructor(host: ReleaseHost) {
    super(`Onmark has no release artifact for ${host.platform}-${host.arch}`);
    this.name = "UnsupportedReleaseTargetError";
  }
}

export function desktopTarget(host: ReleaseHost): DesktopTarget {
  const name = releaseTarget(host);
  const value = manifest.targets[name];
  return Object.freeze({
    browser: browser(value.browser),
    browserPlatform: browserPlatform(value.browserPlatform),
    packageName: `@onmark/cli-${name}`,
    sha256: value.sha256,
  });
}

function releaseTarget(host: ReleaseHost): keyof typeof manifest.targets {
  const target = `${host.platform}-${host.arch}`;
  if (target in manifest.targets) {
    return target as keyof typeof manifest.targets;
  }
  throw new UnsupportedReleaseTargetError(host);
}

function browser(value: string): Browser {
  switch (value) {
    case Browser.CHROME:
      return Browser.CHROME;
    case Browser.CHROMEHEADLESSSHELL:
      return Browser.CHROMEHEADLESSSHELL;
    default:
      throw new Error(`desktop release declares unsupported browser ${value}`);
  }
}

function browserPlatform(value: string): BrowserPlatform {
  switch (value) {
    case BrowserPlatform.LINUX:
      return BrowserPlatform.LINUX;
    case BrowserPlatform.MAC_ARM:
      return BrowserPlatform.MAC_ARM;
    case BrowserPlatform.WIN64:
      return BrowserPlatform.WIN64;
    default:
      throw new Error(
        `desktop release declares unsupported browser platform ${value}`,
      );
  }
}
