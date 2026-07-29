Do not use tools or inspect the filesystem; this prompt is the complete spec.
Complete six small Onmark variant-authoring tasks. Return only the JSON object
required by the output schema.

Candidate: typed source placeholders.

- Emit one `filmHtml` string and one `variantJson` string per case.
- `variantJson` is the compact JSON serialization of the requested flat
  external variant object. This string form is only a structured-output harness
  detail; the candidate design still consumes the decoded object.
- The semantic root is `<om-film>`, containing `<om-scene>` and `<om-shot>`.
  Every shot contains one native `<video src="..."></video>` whose source media
  determines shot duration.
- Declare variant fields in one `<om-fields>` direct child of `<om-film>`.
  Each field is
  `<om-field name="..." type="..." default="..."></om-field>`.
- Field names use lower camel case and are unique. Every field has one of four
  types: `text`, `integer`, `color`, or `boolean`. Integer defaults are base-10
  integers, color defaults are `#rrggbb` or `#rrggbbaa`, and boolean defaults
  are `true` or `false`.
- The external `variant` object contains overrides only. A missing key uses its
  declared default. Unknown keys and wrong JSON types are errors.
- After default expansion, text, CSS custom properties, and conditional
  visibility must match every declared default.
- Insert a field with `{{field}}`. A placeholder in an HTML text node is
  HTML-escaped before parsing; a color or integer placeholder may occupy one
  complete CSS custom-property value. Conditional authored HTML uses
  `{{#if field}}...{{/if}}` with a boolean field.
- Placeholders are expanded before HTML parsing. They may not appear in tag
  names, attribute names or values, `src`, scripts, comments, `duration`, `cue`,
  `delay`, screenplay containment, frame rate, dimensions, or output settings.
- A field affects only the semantic shot containing its placeholder. A
  placeholder outside every shot affects every render region that retains it.
- Variant fields are presentation-only. Never author `start`, `end`, frame
  numbers, or timeline tracks.
- Use ordinary CSS in one `<style>` child. Do not use a motion module.
- Non-void HTML elements have explicit closing tags. Emit no prose or Markdown
  fences.

Cases:

1. `product-offer`: Generate one shot using `media/product.mp4`. Declare
   `headline` text defaulting to `Summer edit`, `accent` color defaulting to
   `#ff4d36`, `progress` integer defaulting to `72`, and `featured` boolean
   defaulting to `false`. Insert the headline in an `<om-title>`, insert accent
   and progress as complete CSS custom-property values on the shot, and use
   them for the title color and a progress bar width. Conditionally include a
   `Featured` badge. The variant overrides all four values with `Night edition`,
   `#72f1b8`, `88`, and `true`.
2. `regional-copy`: Generate two shots using `media/hello.mp4` and
   `media/offer.mp4`. Declare text fields `greeting` defaulting to `Hello` and
   `offer` defaulting to `Save 20%`. Insert `greeting` only inside the first
   shot and `offer` only inside the second. The variant overrides only `offer`
   with `Save 30%`.
3. `boolean-badge`: Generate one shot using `media/status.mp4`. Declare
   `status` text defaulting to `Ready` and `showStatus` boolean defaulting to
   `true`. Insert the status in a compact pill controlled by one conditional.
   The variant sets status to `Rendering` and showStatus to `false`.
4. `rename-field`: Start from a one-shot project whose declaration and title
   placeholder both use a text field named `title`, default `Exact video`.
   Rename the field to `headline` everywhere without changing unrelated markup
   or styles. The decoded final variant is `{"headline":"Exact variants"}` and contains
   no `title` key.
5. `add-local-field`: Start from a two-shot project using `media/a.mp4` and
   `media/b.mp4`; each shot already has one static `<om-title>`. Add a new text
   field `legal` defaulting to `Terms apply`, and insert it in a `.legal`
   element only inside the second shot. The variant overrides it with
   `Limited regions`.
6. `literal-untrusted-text`: Generate one shot using `media/safe.mp4`. Declare
   and insert a text field `message` whose default is `Safe text`. The variant
   value is exactly `</om-title><script>alert(1)</script>`. It must use an HTML
   text-node placeholder so the value is escaped and rendered literally; do
   not put the placeholder in an attribute, CSS, or script.

Return every case exactly once and preserve the case IDs.
