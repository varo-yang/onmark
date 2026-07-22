// Gate-seven presentation whose pixels remain independent of primary video.
// Native execution supplies the admitted video beneath this transparent layer.

import { createDomPresentationBindings } from "@onmark/authoring";
import {
  installRuntimeHost,
  materializedVideoSource,
  PresentationRuntimeAdapter,
} from "@onmark/runtime";

import "./layered-presentation.css";

const READINESS_TIMEOUT_MILLISECONDS = 5_000;

installRuntimeHost(
  new PresentationRuntimeAdapter(
    createDomPresentationBindings({
      document,
      videoSource: materializedVideoSource,
    }),
    READINESS_TIMEOUT_MILLISECONDS,
  ),
);
