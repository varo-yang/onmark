// Public facade for the browser runtime package.
// Consumers never reach into generated codecs or session internals directly.

export {
  ProtocolDecodeError,
  decodeBrowserRequest,
  decodeBrowserResponse,
} from "./generated/codec.js";
export type { BrowserRequest } from "./generated/browser-request.js";
export type { BrowserResponse } from "./generated/browser-response.js";
export type { BrowserPlan } from "./generated/browser-request.js";
export {
  RuntimeAdapterError,
  RuntimeSession,
  type RuntimeAdapter,
  type RuntimeAdapterFailureKind,
} from "./session.js";
