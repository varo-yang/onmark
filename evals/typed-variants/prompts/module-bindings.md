Do not use tools or inspect the filesystem; this prompt is the complete spec.
Complete six small Onmark variant-authoring tasks. Return only the JSON object
required by the output schema.

Candidate: explicit typed module binding.

- Emit one `filmHtml` string and one `variantJson` string per case.
- `variantJson` is the compact JSON serialization of the requested flat
  external variant object. This string form is only a structured-output harness
  detail; the candidate design still consumes the decoded object.
- The semantic root is `<om-film>`, containing `<om-scene>` and `<om-shot>`.
  Every shot contains one native `<video src="..."></video>` whose source media
  determines shot duration.
- Put the schema and binding code in one
  `<script type="module" data-om-motion>` child. Import `booleanField`,
  `colorField`, `defineVariant`, `integerField`, and `textField` from
  `onmark/variant`.
- Export `variantBinding = defineVariant({ fields, bind })`.
  `fields` is an object whose values use the matching field constructor with
  the default value, for example `headline: textField("Hello")`.
  `bind({document, values})` applies the immutable typed values to authored DOM
  with `textContent`, `style.setProperty`, or the `hidden` property.
- The external `variant` object contains overrides only. A missing key uses its
  declared default. Unknown keys and wrong JSON types are errors.
- Field names use lower camel case and are unique. Integer values are exact
  base-10 integers, colors are `#rrggbb` or `#rrggbbaa`, and booleans are JSON
  booleans.
- Variant fields are presentation-only. Never apply them to `src`, `duration`,
  `cue`, `delay`, screenplay containment, frame rate, dimensions, or output
  settings. Never author `start`, `end`, frame numbers, or timeline tracks.
- Select the narrowest stable authored element for every write. Keep default
  text in the authored HTML so the document remains readable without executing
  the module.
- Authored fallback state matches every declared default: bound text contains
  the default text, CSS custom properties start at their default values, and
  boolean targets have `hidden` exactly when their default is false.
- Use ordinary CSS in one `<style>` child. The module contains no animation.
- Non-void HTML elements have explicit closing tags. Emit no prose or Markdown
  fences.

Cases:

1. `product-offer`: Generate one shot using `media/product.mp4`. Declare
   `headline` text defaulting to `Summer edit`, `accent` color defaulting to
   `#ff4d36`, `progress` integer defaulting to `72`, and `featured` boolean
   defaulting to `false`. Bind the headline to an `<om-title>`, bind accent and
   progress as CSS custom properties on the shot, and use them for the title
   color and a progress bar width. A `Featured` badge is shown by `featured`.
   The variant overrides all four values with `Night edition`, `#72f1b8`, `88`,
   and `true`.
2. `regional-copy`: Generate two shots using `media/hello.mp4` and
   `media/offer.mp4`. Declare text fields `greeting` defaulting to `Hello` and
   `offer` defaulting to `Save 20%`. Bind `greeting` only inside the first shot
   and `offer` only inside the second. The variant overrides only `offer` with
   `Save 30%`.
3. `boolean-badge`: Generate one shot using `media/status.mp4`. Declare
   `status` text defaulting to `Ready` and `showStatus` boolean defaulting to
   `true`. Bind both to a compact status pill. The variant sets status to
   `Rendering` and showStatus to `false`.
4. `rename-field`: Start from a one-shot project whose `fields` declaration and
   binding both use a text field named `title`, default `Exact video`. Rename
   the field to `headline` everywhere without changing unrelated markup or
   styles. The decoded final variant is `{"headline":"Exact variants"}` and contains no
   `title` key.
5. `add-local-field`: Start from a two-shot project using `media/a.mp4` and
   `media/b.mp4`; each shot already has one static `<om-title>`. Add a new text
   field `legal` defaulting to `Terms apply`, and a bound `.legal` element only
   inside the second shot. The variant overrides it with `Limited regions`.
6. `literal-untrusted-text`: Generate one shot using `media/safe.mp4`. Declare
   and bind a text field `message` whose default is `Safe text`. The variant
   value is exactly `</om-title><script>alert(1)</script>`. Apply it with
   `textContent`; do not use `innerHTML`, `insertAdjacentHTML`, or string-built
   markup.

Return every case exactly once and preserve the case IDs.
