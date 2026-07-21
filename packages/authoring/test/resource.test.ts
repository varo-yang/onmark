// Standard browser-resource behavior over narrow image and font capabilities.

import assert from "node:assert/strict";
import test from "node:test";

import { createFontResource, createImageResource } from "../src/index.js";

test("decodes one owned image and releases its browser element", async () => {
  const image = new FakeImage();
  const resource = createImageResource({
    document: imageDocument(image),
    id: "poster",
    source: "./resources/poster.svg",
  });

  assert.equal(resource.element, image);
  assert.equal(image.src, "./resources/poster.svg");
  await resource.prepare();
  assert.equal(image.decodeCalls, 1);

  resource.dispose();
  resource.dispose();
  assert.equal(image.sourceRemoved, true);
  assert.equal(image.removeCalls, 1);
});

test("loads one exact font face and removes it terminally", async () => {
  const face = new FakeFontFace();
  const fonts = new FakeFontFaceSet();
  const resource = createFontResource({
    face: face as unknown as FontFace,
    fonts: fonts as unknown as FontFaceSet,
    id: "body",
  });

  await resource.prepare();
  assert.deepEqual(fonts.added, [face]);
  assert.equal(face.loadCalls, 1);

  resource.dispose();
  resource.dispose();
  assert.deepEqual(fonts.deleted, [face]);
});

test("does not install a font whose load completes after disposal", async () => {
  const face = new DeferredFontFace();
  const fonts = new FakeFontFaceSet();
  const resource = createFontResource({
    face: face as unknown as FontFace,
    fonts: fonts as unknown as FontFaceSet,
    id: "body",
  });

  const preparation = resource.prepare();
  resource.dispose();
  face.ready();

  await assert.rejects(preparation, TypeError);
  assert.deepEqual(fonts.added, []);
});

class FakeImage {
  decodeCalls = 0;
  removeCalls = 0;
  sourceRemoved = false;
  src = "";

  async decode(): Promise<void> {
    this.decodeCalls += 1;
  }

  removeAttribute(name: string): void {
    assert.equal(name, "src");
    this.sourceRemoved = true;
  }

  remove(): void {
    this.removeCalls += 1;
  }
}

class FakeFontFace {
  loadCalls = 0;

  async load(): Promise<FontFace> {
    this.loadCalls += 1;
    return this as unknown as FontFace;
  }
}

class DeferredFontFace {
  readonly #loaded = Promise.withResolvers<FontFace>();

  load(): Promise<FontFace> {
    return this.#loaded.promise;
  }

  ready(): void {
    this.#loaded.resolve(this as unknown as FontFace);
  }
}

class FakeFontFaceSet {
  readonly added: FakeFontFace[] = [];
  readonly deleted: FakeFontFace[] = [];

  add(face: FontFace): FontFaceSet {
    this.added.push(face as unknown as FakeFontFace);
    return this as unknown as FontFaceSet;
  }

  delete(face: FontFace): boolean {
    this.deleted.push(face as unknown as FakeFontFace);
    return true;
  }
}

function imageDocument(image: FakeImage): Document {
  return {
    createElement(tagName: string): HTMLImageElement {
      assert.equal(tagName, "img");
      return image as unknown as HTMLImageElement;
    },
  } as Document;
}
