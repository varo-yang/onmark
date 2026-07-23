// Platform tests lock the generated package names and executable layout.

import assert from "node:assert/strict";
import test from "node:test";

import {
  MissingPlatformArtifactError,
  UnsupportedReleaseTargetError,
  platformArtifact,
} from "../src/platform.js";

test("resolves one platform package into the native tool layout", () => {
  const artifact = platformArtifact(
    { arch: "arm64", platform: "darwin" },
    (specifier) => `/modules/${specifier}`,
  );

  assert.deepEqual(artifact, {
    ffmpeg: "/modules/@onmark/cli-darwin-arm64/bin/ffmpeg",
    ffprobe: "/modules/@onmark/cli-darwin-arm64/bin/ffprobe",
    nativeCli: "/modules/@onmark/cli-darwin-arm64/bin/onmark",
    packageName: "@onmark/cli-darwin-arm64",
  });
});

test("uses executable suffixes owned by the target package", () => {
  const artifact = platformArtifact(
    { arch: "x64", platform: "win32" },
    (specifier) => `C:\\modules\\${specifier}`,
  );

  assert.deepEqual(artifact, {
    ffmpeg: "C:\\modules\\@onmark\\cli-win32-x64\\bin\\ffmpeg.exe",
    ffprobe: "C:\\modules\\@onmark\\cli-win32-x64\\bin\\ffprobe.exe",
    nativeCli: "C:\\modules\\@onmark\\cli-win32-x64\\bin\\onmark.exe",
    packageName: "@onmark/cli-win32-x64",
  });
});

test("rejects unsupported targets before package resolution", () => {
  assert.throws(
    () =>
      platformArtifact({ arch: "arm64", platform: "win32" }, () =>
        assert.fail("unsupported targets do not resolve packages"),
      ),
    UnsupportedReleaseTargetError,
  );
});

test("translates an omitted optional dependency once", () => {
  assert.throws(
    () =>
      platformArtifact({ arch: "x64", platform: "linux" }, () => {
        throw new Error("module not found");
      }),
    MissingPlatformArtifactError,
  );
});
