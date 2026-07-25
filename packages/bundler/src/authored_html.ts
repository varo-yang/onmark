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
  readonly regions: readonly AuthoredHtmlRegion[];
  readonly regionStructure: AuthoredHtmlRegionStructure;
  readonly resolveDirectory: string;
  readonly runtimeOffset: number;
}

/** One shot selection within the shared authored-document structure. */
export interface AuthoredHtmlRegion {
  readonly scene: number;
  readonly shot: number;
}

/** Shared source ranges used to materialize one region at a time. */
export interface AuthoredHtmlRegionStructure {
  readonly scenes: readonly AuthoredHtmlScene[];
}

export interface AuthoredHtmlScene {
  readonly range: SourceRange;
  readonly shots: readonly SourceRange[];
}

/** One materialized shot-scoped browser document. */
export interface AuthoredHtmlRegionDocument {
  readonly document: string;
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

interface InstalledHtml {
  readonly document: string;
  readonly motion: string | undefined;
  readonly runtimeOffset: number;
}

interface HtmlProjection extends InstalledHtml {
  readonly regions: readonly AuthoredHtmlRegion[];
  readonly regionStructure: AuthoredHtmlRegionStructure;
}

function extractMotion(source: string): HtmlProjection {
  // Reparse after projection because source edits invalidate parse5 offsets.
  // The second tree owns runtime insertion; no code translates stale ranges.
  const browserSource = projectBrowserDocument(source);
  const installed = installRuntime(browserSource);
  const projection = projectShotStructure(installed.document);
  return {
    ...installed,
    ...projection,
  };
}

function projectBrowserDocument(source: string): string {
  const document = parseDocument(source);
  const edits: SourceRange[] = [];
  collectCompilerFacts(document, source, edits);
  return removeRanges(source, edits);
}

function installRuntime(source: string): InstalledHtml {
  const document = parseDocument(source);
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

  const motionSource = source.slice(startTag.endOffset, endTag.startOffset);
  const browserSource = removeRanges(source, [
    { end: location.endOffset, start: location.startOffset },
  ]);
  return {
    ...insertRuntimeScript(browserSource, parseDocument(browserSource)),
    motion: motionSource,
  };
}

function projectShotStructure(
  source: string,
): Pick<HtmlProjection, "regions" | "regionStructure"> {
  const document = parseDocument(source);
  const film = findElement(document, "om-film");
  if (film === undefined) {
    return {
      regions: Object.freeze([]),
      regionStructure: Object.freeze({ scenes: Object.freeze([]) }),
    };
  }

  const scenes = childElements(film, "om-scene").map((scene) => {
    const shots = childElements(scene, "om-shot").map((shot) =>
      removableElementRange(source, shot),
    );
    return Object.freeze({
      range: removableElementRange(source, scene),
      shots: Object.freeze(shots),
    });
  });
  const regions = scenes.flatMap((scene, sceneIndex) =>
    scene.shots.map((_, shotIndex) =>
      Object.freeze({ scene: sceneIndex, shot: shotIndex }),
    ),
  );
  return {
    regions: Object.freeze(regions),
    regionStructure: Object.freeze({ scenes: Object.freeze(scenes) }),
  };
}

/** Materializes one shot projection without retaining every projected document. */
export function projectShotDocument(
  html: AuthoredHtml,
  region: AuthoredHtmlRegion,
): AuthoredHtmlRegionDocument {
  const selectedScene = html.regionStructure.scenes[region.scene];
  const selectedShot = selectedScene?.shots[region.shot];
  if (selectedScene === undefined || selectedShot === undefined) {
    throw new AuthoredHtmlError("browser projection selects an unknown shot");
  }

  const ranges: SourceRange[] = [];
  for (const [sceneIndex, scene] of html.regionStructure.scenes.entries()) {
    if (sceneIndex !== region.scene) {
      ranges.push(scene.range);
      continue;
    }
    for (const [shotIndex, shot] of scene.shots.entries()) {
      if (shotIndex !== region.shot) {
        ranges.push(shot);
      }
    }
  }

  return Object.freeze({
    document: removeRanges(html.document, ranges),
    runtimeOffset: projectOffset(html.runtimeOffset, ranges),
  });
}

function parseDocument(source: string): DefaultTreeAdapterTypes.Document {
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
  return document;
}

function insertRuntimeScript(
  source: string,
  document: DefaultTreeAdapterTypes.Document,
): InstalledHtml {
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

function childElements(
  parent: DefaultTreeAdapterTypes.Element,
  name: string,
): DefaultTreeAdapterTypes.Element[] {
  return parent.childNodes.filter(
    (child): child is DefaultTreeAdapterTypes.Element =>
      "tagName" in child && child.tagName === name,
  );
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

// ── Compiler/browser ownership projection

export interface SourceRange {
  readonly end: number;
  readonly start: number;
}

function projectOffset(offset: number, ranges: readonly SourceRange[]): number {
  let projected = offset;
  for (const range of ranges) {
    if (range.start < offset && offset < range.end) {
      throw new AuthoredHtmlError(
        "browser projection cannot remove its runtime entry",
      );
    }
    if (range.end <= offset) {
      projected -= range.end - range.start;
    }
  }
  return projected;
}

interface SourceOffsets {
  readonly endOffset: number;
  readonly startOffset: number;
}

function collectCompilerFacts(
  node: DefaultTreeAdapterTypes.Node,
  source: string,
  ranges: SourceRange[],
): void {
  if ("tagName" in node) {
    if (isCompilerOnlyElement(node.tagName)) {
      ranges.push(removableElementRange(source, node));
      return;
    }
    collectCompilerAttributes(node, source, ranges);
  }
  if ("childNodes" in node) {
    for (const child of node.childNodes) {
      collectCompilerFacts(child, source, ranges);
    }
  }
}

function collectCompilerAttributes(
  element: DefaultTreeAdapterTypes.Element,
  source: string,
  ranges: SourceRange[],
): void {
  for (const attribute of element.attrs) {
    if (!isCompilerOnlyAttribute(element.tagName, attribute.name)) {
      continue;
    }
    const location = element.sourceCodeLocation?.attrs?.[attribute.name];
    if (location === undefined) {
      throw new AuthoredHtmlError(
        `browser projection cannot locate ${attribute.name} on <${element.tagName}>`,
      );
    }
    ranges.push(attributeRange(source, location));
  }
}

function attributeRange(source: string, location: SourceOffsets): SourceRange {
  let start = location.startOffset;
  while (start > 0 && isHtmlSpace(source[start - 1]!)) {
    start -= 1;
  }
  return { end: location.endOffset, start };
}

function isCompilerOnlyElement(tagName: string): boolean {
  // This closed list mirrors the language boundary, not HTML vocabulary.
  // New spellings require the Rust compiler, specification, and identity
  // conformance to change together before they may disappear from the browser.
  return (
    tagName === "om-cues" ||
    tagName === "om-music" ||
    tagName === "om-sfx" ||
    tagName === "om-vo"
  );
}

function isCompilerOnlyAttribute(
  tagName: string,
  attributeName: string,
): boolean {
  switch (tagName) {
    case "om-cta":
    case "om-title":
      return attributeName === "cue" || attributeName === "delay";
    case "om-shot":
      return attributeName === "duration";
    case "video":
      return attributeName === "delay" || attributeName === "src";
    default:
      return false;
  }
}

function isHtmlSpace(character: string): boolean {
  return (
    character === "\t" ||
    character === "\n" ||
    character === "\f" ||
    character === "\r" ||
    character === " "
  );
}

function elementRange(element: DefaultTreeAdapterTypes.Element): SourceRange {
  const location = element.sourceCodeLocation;
  if (location == null) {
    throw new AuthoredHtmlError(
      `browser projection cannot locate <${element.tagName}>`,
    );
  }
  return { end: location.endOffset, start: location.startOffset };
}

function removableElementRange(
  source: string,
  element: DefaultTreeAdapterTypes.Element,
): SourceRange {
  const range = elementRange(element);
  // Indentation on an otherwise empty line belongs to the removed element.
  // Retaining it creates a browser text node and invalid source artifacts.
  const lineStart = source.lastIndexOf("\n", range.start - 1) + 1;
  const newline = source.indexOf("\n", range.end);
  const lineEnd = newline === -1 ? source.length : newline + 1;
  const leading = source.slice(lineStart, range.start);
  const trailing = source.slice(range.end, lineEnd);

  if (isHtmlWhitespace(leading) && isHtmlWhitespace(trailing)) {
    return { end: lineEnd, start: lineStart };
  }
  return range;
}

function isHtmlWhitespace(value: string): boolean {
  for (const character of value) {
    if (!isHtmlSpace(character)) {
      return false;
    }
  }
  return true;
}

function removeRanges(source: string, ranges: readonly SourceRange[]): string {
  const ordered = ranges.toSorted((left, right) => left.start - right.start);
  const parts: string[] = [];
  let cursor = 0;

  for (const range of ordered) {
    if (range.start < cursor || range.end < range.start) {
      throw new AuthoredHtmlError("browser projection ranges overlap");
    }
    parts.push(source.slice(cursor, range.start));
    cursor = range.end;
  }
  parts.push(source.slice(cursor));
  return parts.join("");
}
