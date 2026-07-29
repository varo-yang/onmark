// Bounded HTML ingestion and exact motion-module extraction.
// Parsing identifies source ranges; publication preserves all other bytes.

import { open } from "node:fs/promises";
import { dirname, resolve } from "node:path";

import { parse, type DefaultTreeAdapterTypes, type ParserError } from "parse5";
import type { PresentationVisualCapability } from "@onmark/runtime/types";

import { freezeHtmlImages, type HtmlImageResource } from "./html_image.js";

const MAX_HTML_BYTES = 8 * 1024 * 1024;
const MOTION_ATTRIBUTE = "data-om-motion";
const VISUAL_CAPABILITY_META = "onmark:visual-capability";
const RUNTIME_SCRIPT =
  '<script type="module" src="./presentation.js"></script>';

// ── Public contract

/** Authored document bytes and optional inline module prepared for esbuild. */
export interface AuthoredHtml {
  readonly document: string;
  readonly motion: string | undefined;
  readonly visualCapability: PresentationVisualCapability | undefined;
  readonly resources: readonly HtmlImageResource[];
  readonly shots: readonly AuthoredHtmlShot[];
  readonly regionStructure: AuthoredHtmlRegionStructure;
  readonly resolveDirectory: string;
  readonly runtimeOffset: number;
}

/** One dense screenplay-order shot within the authored document. */
export interface AuthoredHtmlShot {
  readonly scene: number;
  readonly shot: number;
}

/** Shared source ranges used to materialize one region at a time. */
export interface AuthoredHtmlRegionStructure {
  readonly scenes: readonly AuthoredHtmlScene[];
}

export interface AuthoredHtmlScene {
  readonly range: SourceRange;
  readonly shots: readonly AuthoredHtmlShotRange[];
  readonly transitions: readonly AuthoredHtmlTransitionRange[];
}

export interface AuthoredHtmlShotRange {
  readonly index: number;
  readonly range: SourceRange;
}

export interface AuthoredHtmlTransitionRange {
  readonly incomingShot: number;
  readonly outgoingShot: number;
  readonly range: SourceRange;
}

/** One materialized render-region browser document. */
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
  maxOutputBytes: number,
  resolveDirectory?: string,
): Promise<AuthoredHtml> {
  const absolute = resolve(path);
  const source = await readBoundedSource(absolute);
  const visualCapability = declaredVisualCapability(parseDocument(source));
  const directory =
    resolveDirectory === undefined
      ? dirname(absolute)
      : resolve(resolveDirectory);
  const browserSource = projectBrowserDocument(source);
  const frozen = await freezeHtmlImages(
    browserSource,
    directory,
    maxOutputBytes,
  );
  const extracted = extractMotion(frozen.document);
  return Object.freeze({
    ...extracted,
    resources: frozen.resources,
    resolveDirectory: directory,
    visualCapability,
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
  readonly shots: readonly AuthoredHtmlShot[];
  readonly regionStructure: AuthoredHtmlRegionStructure;
}

function extractMotion(source: string): HtmlProjection {
  // Reparse after each source edit because parse5 offsets are not transferable.
  // The final tree owns runtime insertion and projected shot ranges.
  const installed = installRuntime(source);
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
): Pick<HtmlProjection, "shots" | "regionStructure"> {
  const document = parseDocument(source);
  const film = findElement(document, "om-film");
  if (film === undefined) {
    return {
      shots: Object.freeze([]),
      regionStructure: Object.freeze({ scenes: Object.freeze([]) }),
    };
  }

  const shots: AuthoredHtmlShot[] = [];
  const scenes = childElements(film, "om-scene").map((scene, sceneIndex) =>
    projectSceneStructure(source, scene, sceneIndex, shots),
  );
  return {
    shots: Object.freeze(shots),
    regionStructure: Object.freeze({ scenes: Object.freeze(scenes) }),
  };
}

function projectSceneStructure(
  source: string,
  scene: DefaultTreeAdapterTypes.Element,
  sceneIndex: number,
  documentShots: AuthoredHtmlShot[],
): AuthoredHtmlScene {
  const shots: AuthoredHtmlShotRange[] = [];
  const transitions: AuthoredHtmlTransitionRange[] = [];
  let outgoingShot: number | undefined;
  let pendingTransition: SourceRange | undefined;

  for (const element of directElements(scene)) {
    if (element.tagName === "om-transition") {
      if (outgoingShot === undefined || pendingTransition !== undefined) {
        throw transitionProjectionError();
      }
      pendingTransition = removableElementRange(source, element);
      continue;
    }
    if (element.tagName !== "om-shot") {
      continue;
    }

    const index = documentShots.length;
    const shot = Object.freeze({ scene: sceneIndex, shot: shots.length });
    documentShots.push(shot);
    shots.push(
      Object.freeze({
        index,
        range: removableElementRange(source, element),
      }),
    );
    if (pendingTransition !== undefined && outgoingShot !== undefined) {
      transitions.push(
        Object.freeze({
          incomingShot: index,
          outgoingShot,
          range: pendingTransition,
        }),
      );
      pendingTransition = undefined;
    }
    outgoingShot = index;
  }

  if (pendingTransition !== undefined) {
    throw transitionProjectionError();
  }
  return Object.freeze({
    range: removableElementRange(source, scene),
    shots: Object.freeze(shots),
    transitions: Object.freeze(transitions),
  });
}

function transitionProjectionError(): AuthoredHtmlError {
  return new AuthoredHtmlError(
    "compiled transition boundaries cannot be projected into browser regions",
  );
}

/** Materializes one Rust-planned region without retaining every projection. */
export function projectRegionDocument(
  html: AuthoredHtml,
  shotIndices: readonly number[],
): AuthoredHtmlRegionDocument {
  const selectedShots = selectedShotSet(html, shotIndices);
  const ranges: SourceRange[] = [];
  for (const scene of html.regionStructure.scenes) {
    if (!scene.shots.some((shot) => selectedShots.has(shot.index))) {
      ranges.push(scene.range);
      continue;
    }
    for (const shot of scene.shots) {
      if (!selectedShots.has(shot.index)) {
        ranges.push(shot.range);
      }
    }
    for (const transition of scene.transitions) {
      const retained =
        selectedShots.has(transition.outgoingShot) &&
        selectedShots.has(transition.incomingShot);
      if (!retained) {
        ranges.push(transition.range);
      }
    }
  }

  return Object.freeze({
    document: removeRanges(html.document, ranges),
    runtimeOffset: projectOffset(html.runtimeOffset, ranges),
  });
}

function selectedShotSet(
  html: AuthoredHtml,
  shotIndices: readonly number[],
): ReadonlySet<number> {
  const selected = new Set<number>();
  for (const index of shotIndices) {
    if (html.shots[index] === undefined) {
      throw new AuthoredHtmlError(
        `browser projection selects unknown shot ${index}`,
      );
    }
    selected.add(index);
  }
  return selected;
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

function declaredVisualCapability(
  document: DefaultTreeAdapterTypes.Document,
): PresentationVisualCapability | undefined {
  const declarations: string[] = [];
  visit(document, (element) => {
    if (element.tagName !== "meta") {
      return;
    }
    const attributes = new Map(
      element.attrs.map(({ name, value }) => [name, value]),
    );
    if (attributes.get("name") === VISUAL_CAPABILITY_META) {
      declarations.push(attributes.get("content") ?? "");
    }
  });
  if (declarations.length > 1) {
    throw new AuthoredHtmlError(
      `authored HTML may contain at most one ${VISUAL_CAPABILITY_META} declaration`,
    );
  }
  const [declaration] = declarations;
  if (declaration === undefined) {
    return undefined;
  }
  switch (declaration) {
    case "browserComposite":
    case "separableBackdrop":
    case "separableOverlay":
      return declaration;
    default:
      throw new AuthoredHtmlError(
        `${VISUAL_CAPABILITY_META} must be browserComposite, separableBackdrop, or separableOverlay`,
      );
  }
}

function findElement(
  node: DefaultTreeAdapterTypes.Node,
  name: string,
): DefaultTreeAdapterTypes.Element | undefined {
  const pending = [node];
  while (pending.length > 0) {
    const current = pending.pop();
    if (current === undefined) {
      break;
    }
    if ("tagName" in current && current.tagName === name) {
      return current;
    }
    if ("childNodes" in current) {
      pushChildren(pending, current.childNodes);
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

function directElements(
  parent: DefaultTreeAdapterTypes.Element,
): DefaultTreeAdapterTypes.Element[] {
  return parent.childNodes.filter(
    (child): child is DefaultTreeAdapterTypes.Element => "tagName" in child,
  );
}

function visit(
  node: DefaultTreeAdapterTypes.Node,
  visitor: (element: DefaultTreeAdapterTypes.Element) => void,
): void {
  const pending = [node];
  while (pending.length > 0) {
    const current = pending.pop();
    if (current === undefined) {
      break;
    }
    if ("tagName" in current) {
      visitor(current);
    }
    if ("childNodes" in current) {
      pushChildren(pending, current.childNodes);
    }
  }
}

function pushChildren(
  pending: DefaultTreeAdapterTypes.Node[],
  children: readonly DefaultTreeAdapterTypes.Node[],
): void {
  for (let index = children.length - 1; index >= 0; index -= 1) {
    const child = children[index];
    if (child !== undefined) {
      pending.push(child);
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
  const pending = [node];
  while (pending.length > 0) {
    const current = pending.pop();
    if (current === undefined) {
      break;
    }
    if ("tagName" in current) {
      if (isCompilerOnlyElement(current.tagName)) {
        ranges.push(removableElementRange(source, current));
        continue;
      }
      collectCompilerAttributes(current, source, ranges);
    }
    if ("childNodes" in current) {
      pushChildren(pending, current.childNodes);
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
    tagName === "om-fields" ||
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
    case "om-transition":
      return attributeName === "duration";
    case "video":
      return (
        attributeName === "delay" ||
        attributeName === "hold-last" ||
        attributeName === "plays" ||
        attributeName === "speed" ||
        attributeName === "src" ||
        attributeName === "trim"
      );
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
