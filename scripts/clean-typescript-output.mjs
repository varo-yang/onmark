// Removes one package's generated TypeScript output before a fresh build.
// A clean boundary keeps deleted tests and modules out of local verification.

import { rm } from "node:fs/promises";
import { dirname, isAbsolute, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const repository = dirname(dirname(fileURLToPath(import.meta.url)));
const packages = join(repository, "packages");
const packageDirectory = process.cwd();
const packageName = relative(packages, packageDirectory);

if (
  packageName.length === 0 ||
  packageName.startsWith("..") ||
  isAbsolute(packageName) ||
  packageName.includes("/") ||
  packageName.includes("\\")
) {
  throw new Error("TypeScript output cleanup must run from one package root");
}

await rm(join(packageDirectory, "dist"), { force: true, recursive: true });
