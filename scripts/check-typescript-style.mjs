// Repository-shape checks that complement Oxlint, strict tsc, and Prettier.
// This script owns rules that syntax linters cannot express as file-local AST checks.

import { readdirSync, readFileSync } from "node:fs";
import { basename, extname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const REPOSITORY = fileURLToPath(new URL("..", import.meta.url));
const SOURCE_ROOTS = ["packages", "scripts"];
const SOURCE_EXTENSIONS = new Set([".js", ".mjs", ".ts"]);
const IGNORED_DIRECTORIES = new Set(["dist", "generated", "node_modules"]);
const FORBIDDEN_STEMS = new Set(["common", "helpers", "shared", "utils"]);
const SECTION_LINE_LIMIT = 200;
const SECTION_DIVIDER = "// ──";

const diagnostics = sourceFiles().flatMap(fileShapeDiagnostics);
if (diagnostics.length > 0) {
  process.stderr.write(`${diagnostics.join("\n")}\n`);
  process.exitCode = 1;
}

function sourceFiles() {
  return SOURCE_ROOTS.flatMap((root) => collect(join(REPOSITORY, root))).sort();
}

function collect(directory) {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.isDirectory() && IGNORED_DIRECTORIES.has(entry.name)) continue;
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...collect(path));
    } else if (SOURCE_EXTENSIONS.has(extname(entry.name))) {
      files.push(path);
    }
  }
  return files;
}

function fileShapeDiagnostics(path) {
  const sourceText = readFileSync(path, "utf8");
  const relativePath = relative(REPOSITORY, path);
  const diagnostics = [];
  const lines = sourceText.split("\n");
  const first = lines[0] ?? "";
  const second = lines[1] ?? "";
  if (
    !first.startsWith("//") &&
    !(first.startsWith("#!") && second.startsWith("//"))
  ) {
    diagnostics.push(`${relativePath}: missing file header comment`);
  }
  if (
    lines.length > SECTION_LINE_LIMIT &&
    !sourceText.includes(SECTION_DIVIDER)
  ) {
    diagnostics.push(
      `${relativePath}: files over ${SECTION_LINE_LIMIT} lines need section dividers`,
    );
  }

  const stem = basename(path, extname(path));
  if (FORBIDDEN_STEMS.has(stem)) {
    diagnostics.push(`${relativePath}: dumping-ground filename is forbidden`);
  }
  return diagnostics;
}
