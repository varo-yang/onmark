// Platform release artifacts keep native tools out of the portable npm facade.

import { posix, win32 } from "node:path";

import { desktopTarget, type ReleaseHost } from "./release.js";

export { UnsupportedReleaseTargetError } from "./release.js";

export interface PlatformArtifact {
  readonly ffmpeg: string;
  readonly ffprobe: string;
  readonly nativeCli: string;
  readonly packageName: string;
}

type PackageResolver = (specifier: string) => string;

export class MissingPlatformArtifactError extends Error {
  constructor(packageName: string, cause: unknown) {
    super(
      `Onmark's platform package ${packageName} is missing; reinstall onmark without omitting optional dependencies`,
      { cause },
    );
    this.name = "MissingPlatformArtifactError";
  }
}

export function platformArtifact(
  host: ReleaseHost,
  resolvePackage: PackageResolver,
): PlatformArtifact {
  const target = desktopTarget(host);

  const paths = host.platform === "win32" ? win32 : posix;
  let packageRoot: string;
  try {
    packageRoot = paths.dirname(
      resolvePackage(`${target.packageName}/package.json`),
    );
  } catch (error) {
    throw new MissingPlatformArtifactError(target.packageName, error);
  }

  const extension = host.platform === "win32" ? ".exe" : "";
  const executable = (name: string) =>
    paths.join(packageRoot, "bin", `${name}${extension}`);
  return Object.freeze({
    ffmpeg: executable("ffmpeg"),
    ffprobe: executable("ffprobe"),
    nativeCli: executable("onmark"),
    packageName: target.packageName,
  });
}
