// Literal DOM sinks for Rust-owned canonical presentation values.
// Missing region fields deliberately retain their truthful authored fallback.

import type { RuntimePlan } from "@onmark/runtime/types";

const BINDING_SELECTOR = "[data-om-text], [data-om-css], [data-om-show]";
const MAX_VARIANT_BINDING_ELEMENTS = 65_536;

/** Applies only the typed fields projected into one Browser Plan. */
export function applyVariantFields(
  document: Document,
  fields: RuntimePlan["variantFields"],
): void {
  if (fields.length === 0) {
    return;
  }

  const values = new Map(fields.map((field) => [field.name, field.value]));
  const applied = new Set<string>();
  const elements = document.querySelectorAll<HTMLElement>(BINDING_SELECTOR);
  if (elements.length > MAX_VARIANT_BINDING_ELEMENTS) {
    throw new Error("authored HTML variant binding count exceeds its limit");
  }

  for (const element of elements) {
    applyTextBinding(element, values, applied);
    applyCssBindings(element, values, applied);
    applyShowBinding(element, values, applied);
  }

  for (const field of fields) {
    if (!applied.has(field.name)) {
      throw new Error(
        `authored HTML has no projected binding for variant field "${field.name}"`,
      );
    }
  }
}

type VariantValues = ReadonlyMap<
  string,
  RuntimePlan["variantFields"][number]["value"]
>;

function applyTextBinding(
  element: HTMLElement,
  values: VariantValues,
  applied: Set<string>,
): void {
  const name = element.getAttribute("data-om-text");
  if (name === null) {
    return;
  }
  const value = values.get(name);
  if (value === undefined) {
    return;
  }
  if (value.kind !== "text") {
    throw incompatibleBinding(name, "data-om-text");
  }
  element.textContent = value.value;
  applied.add(name);
}

function applyCssBindings(
  element: HTMLElement,
  values: VariantValues,
  applied: Set<string>,
): void {
  const attribute = element.getAttribute("data-om-css");
  if (attribute === null) {
    return;
  }

  for (const name of attribute.split(/[\t\n\f\r ]+/u)) {
    const value = values.get(name);
    if (value === undefined) {
      continue;
    }
    if (value.kind !== "color" && value.kind !== "integer") {
      throw incompatibleBinding(name, "data-om-css");
    }
    element.style.setProperty(`--${name}`, String(value.value));
    applied.add(name);
  }
}

function applyShowBinding(
  element: HTMLElement,
  values: VariantValues,
  applied: Set<string>,
): void {
  const name = element.getAttribute("data-om-show");
  if (name === null) {
    return;
  }
  const value = values.get(name);
  if (value === undefined) {
    return;
  }
  if (value.kind !== "boolean") {
    throw incompatibleBinding(name, "data-om-show");
  }
  element.hidden = !value.value;
  applied.add(name);
}

function incompatibleBinding(name: string, attribute: string): Error {
  return new Error(
    `variant field "${name}" is incompatible with authored ${attribute}`,
  );
}
