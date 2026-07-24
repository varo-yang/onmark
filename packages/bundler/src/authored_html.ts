// Bounded HTML ingestion and exact motion-module extraction.
// Parsing identifies source ranges; publication preserves all other bytes.

import { open } from "node:fs/promises";
import { dirname, resolve } from "node:path";

import { parse, type DefaultTreeAdapterTypes, type ParserError } from "parse5";

const MAX_HTML_BYTES = 8 * 1024 * 1024;
const MOTION_ATTRIBUTE = "data-om-motion";
const RUNTIME_SCRIPT =
  '<script type="module" src="./presentation.js"></script>';

// ── Public contract

/** Authored document bytes and optional inline module prepared for esbuild. */
export interface AuthoredHtml {
  readonly document: string;
  readonly motion: string | undefined;
  readonly resolveDirectory: string;
  readonly runtimeOffset: number;
}

/** Invalid or unreadable authored HTML at the Node bundling boundary. */
export class AuthoredHtmlError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "AuthoredHtmlError";
  }
}

/** Reads one bounded UTF-8 document and isolates its motion module. */
export async function readAuthoredHtml(
  path: string,
  resolveDirectory?: string,
): Promise<AuthoredHtml> {
  const absolute = resolve(path);
  const source = await readBoundedSource(absolute);
  const extracted = extractMotion(source);
  return Object.freeze({
    ...extracted,
    resolveDirectory:
      resolveDirectory === undefined
        ? dirname(absolute)
        : resolve(resolveDirectory),
  });
}

// ── Bounded input

async function readBoundedSource(path: string): Promise<string> {
  let file;
  try {
    file = await open(path, "r");
  } catch (error) {
    throw new AuthoredHtmlError(`cannot open authored HTML ${path}`, {
      cause: error,
    });
  }

  try {
    const bytes = Buffer.allocUnsafe(MAX_HTML_BYTES + 1);
    let length = 0;
    while (length < bytes.length) {
      const { bytesRead } = await file.read(
        bytes,
        length,
        bytes.length - length,
        length,
      );
      if (bytesRead === 0) {
        break;
      }
      length += bytesRead;
    }
    if (length > MAX_HTML_BYTES) {
      throw new AuthoredHtmlError(
        `authored HTML exceeds the ${MAX_HTML_BYTES}-byte limit`,
      );
    }
    return new TextDecoder("utf-8", { fatal: true }).decode(
      bytes.subarray(0, length),
    );
  } catch (error) {
    if (error instanceof AuthoredHtmlError) {
      throw error;
    }
    throw new AuthoredHtmlError(`cannot read authored HTML ${path}`, {
      cause: error,
    });
  } finally {
    await file.close();
  }
}

// ── Document projection

interface HtmlProjection {
  readonly document: string;
  readonly motion: string | undefined;
  readonly runtimeOffset: number;
}

function extractMotion(source: string): HtmlProjection {
  const errors: ParserError[] = [];
  const document = parse(source, {
    onParseError(error) {
      if (error.code !== "missing-doctype") {
        errors.push(error);
      }
    },
    sourceCodeLocationInfo: true,
  });
  if (errors.length > 0) {
    throw new AuthoredHtmlError(
      `authored HTML cannot be bundled after parse error ${errors[0]!.code}`,
    );
  }
  const scripts = collectScripts(document);
  const motion = scripts.filter(isMotionScript);
  const unsupported = scripts.filter((script) => !isMotionScript(script));
  if (unsupported.length > 0) {
    throw new AuthoredHtmlError(
      'authored HTML scripts must use type="module" and data-om-motion',
    );
  }
  if (motion.length > 1) {
    throw new AuthoredHtmlError(
      "authored HTML may contain at most one motion module",
    );
  }

  const script = motion[0];
  if (script === undefined) {
    return insertRuntimeScript(source, document);
  }

  const location = script.sourceCodeLocation;
  const startTag = location?.startTag;
  const endTag = location?.endTag;
  if (location == null || startTag === undefined || endTag === undefined) {
    throw new AuthoredHtmlError(
      "the motion module must have explicit opening and closing tags",
    );
  }

  return {
    document:
      source.slice(0, location.startOffset) +
      RUNTIME_SCRIPT +
      source.slice(location.endOffset),
    motion: source.slice(startTag.endOffset, endTag.startOffset),
    runtimeOffset: location.startOffset,
  };
}

function insertRuntimeScript(
  source: string,
  document: DefaultTreeAdapterTypes.Document,
): HtmlProjection {
  const body = findElement(document, "body");
  const endTag = body?.sourceCodeLocation?.endTag;
  if (endTag === undefined) {
    return {
      document: `${source}\n${RUNTIME_SCRIPT}`,
      motion: undefined,
      runtimeOffset: source.length + 1,
    };
  }

  const lineStart = source.lastIndexOf("\n", endTag.startOffset - 1) + 1;
  const indentation = source.slice(lineStart, endTag.startOffset);
  if (indentation.trim().length === 0) {
    const before = source.slice(0, lineStart);
    const after = source.slice(lineStart);
    const prefix = `${before}${indentation}  `;
    return {
      document: `${prefix}${RUNTIME_SCRIPT}\n${after}`,
      motion: undefined,
      runtimeOffset: prefix.length,
    };
  }
  const before = source.slice(0, endTag.startOffset);
  const after = source.slice(endTag.startOffset);
  return {
    document: `${before}\n${RUNTIME_SCRIPT}\n${after}`,
    motion: undefined,
    runtimeOffset: before.length + 1,
  };
}

// ── Parsed-tree queries

function collectScripts(
  node: DefaultTreeAdapterTypes.Node,
): DefaultTreeAdapterTypes.Element[] {
  const scripts: DefaultTreeAdapterTypes.Element[] = [];
  visit(node, (element) => {
    if (element.tagName === "script") {
      scripts.push(element);
    }
  });
  return scripts;
}

function findElement(
  node: DefaultTreeAdapterTypes.Node,
  name: string,
): DefaultTreeAdapterTypes.Element | undefined {
  if ("tagName" in node && node.tagName === name) {
    return node;
  }
  if ("childNodes" in node) {
    for (const child of node.childNodes) {
      const found = findElement(child, name);
      if (found !== undefined) {
        return found;
      }
    }
  }
  return undefined;
}

function visit(
  node: DefaultTreeAdapterTypes.Node,
  visitor: (element: DefaultTreeAdapterTypes.Element) => void,
): void {
  if ("tagName" in node) {
    visitor(node);
  }
  if ("childNodes" in node) {
    for (const child of node.childNodes) {
      visit(child, visitor);
    }
  }
}

function isMotionScript(element: DefaultTreeAdapterTypes.Element): boolean {
  const attributes = new Map(
    element.attrs.map(({ name, value }) => [name, value]),
  );
  return (
    attributes.get("type") === "module" &&
    attributes.has(MOTION_ATTRIBUTE) &&
    !attributes.has("src")
  );
}
