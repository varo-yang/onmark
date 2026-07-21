// Standard image and font resources for deterministic browser preparation.
// Each adapter owns the platform object it releases at terminal disposal.

import type { PresentationResource } from "@onmark/runtime/types";

/** Inputs required to create one decoded image resource. */
export interface ImageResourceOptions {
  readonly document: Document;
  readonly id: string;
  readonly source: string;
}

/** An owned image resource whose element remains available for authored layout. */
export interface ImageResource extends PresentationResource {
  readonly element: HTMLImageElement;
  readonly kind: "image";
  prepare(): Promise<void>;
  dispose(): void;
}

/** Inputs required to load one exact font face into a document font set. */
export interface FontResourceOptions {
  readonly face: FontFace;
  readonly fonts: FontFaceSet;
  readonly id: string;
}

/** An owned font face installed into one document font set. */
export interface FontResource extends PresentationResource {
  readonly kind: "font";
  prepare(): Promise<void>;
  dispose(): void;
}

/** Creates one image whose decoded pixels gate browser preparation. */
export function createImageResource(
  options: ImageResourceOptions,
): ImageResource {
  return new OwnedImageResource(options);
}

/** Creates one font resource installed only after its exact face loads. */
export function createFontResource(options: FontResourceOptions): FontResource {
  return new OwnedFontResource(options);
}

class OwnedImageResource implements ImageResource {
  readonly element: HTMLImageElement;
  readonly id: string;
  readonly kind = "image" as const;
  #disposed = false;

  constructor({ document, id, source }: ImageResourceOptions) {
    this.element = document.createElement("img");
    this.element.src = source;
    this.id = id;
  }

  async prepare(): Promise<void> {
    this.#requireActive();
    await this.element.decode();
  }

  dispose(): void {
    if (this.#disposed) {
      return;
    }
    this.#disposed = true;
    this.element.removeAttribute("src");
    this.element.remove();
  }

  #requireActive(): void {
    if (this.#disposed) {
      throw new TypeError("image resource is disposed");
    }
  }
}

class OwnedFontResource implements FontResource {
  readonly id: string;
  readonly kind = "font" as const;
  readonly #face: FontFace;
  readonly #fonts: FontFaceSet;
  #disposed = false;

  constructor({ face, fonts, id }: FontResourceOptions) {
    this.#face = face;
    this.#fonts = fonts;
    this.id = id;
  }

  async prepare(): Promise<void> {
    this.#requireActive();
    const face = await this.#face.load();
    this.#requireActive();
    this.#fonts.add(face);
  }

  dispose(): void {
    if (this.#disposed) {
      return;
    }
    this.#disposed = true;
    this.#fonts.delete(this.#face);
  }

  #requireActive(): void {
    if (this.#disposed) {
      throw new TypeError("font resource is disposed");
    }
  }
}
