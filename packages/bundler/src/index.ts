// Public facade for deterministic presentation bundling.

export {
  BundleError,
  bundleDomPresentation,
  bundlePresentation,
  type BundleArtifact,
  type BundleErrorKind,
  type BundleFile,
  type BundleManifest,
  type BundleOptions,
  type DomBundleOptions,
} from "./presentation.js";

export {
  BUNDLE_ASSET_DIRECTORY,
  BUNDLE_ENTRY_POINT,
  BUNDLE_MANIFEST_FILE,
  BUNDLE_TEMPORAL_CAPABILITIES,
  BUNDLE_VERSION,
} from "./generated/bundle-manifest.js";
