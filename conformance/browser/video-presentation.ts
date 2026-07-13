// Gate-one presentation composing author-owned layout with the browser runtime.
// Presentation owns elements and layout; runtime owns timing and readiness.

import { createDomPresentationBindings } from "@onmark/authoring";
import {
  PresentationRuntimeAdapter,
  installRuntimeHost,
  materializedVideoSource,
} from "@onmark/runtime";

import "./video-presentation.css";

const READINESS_TIMEOUT_MILLISECONDS = 5_000;

const adapter = new PresentationRuntimeAdapter(
  createDomPresentationBindings({
    document,
    videoSource: materializedVideoSource,
  }),
  READINESS_TIMEOUT_MILLISECONDS,
);

installRuntimeHost(adapter);
