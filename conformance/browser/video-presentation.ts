// Gate-one presentation proving the production browser-video adapter.
// Presentation owns elements and layout; runtime owns timing and readiness.

import {
  VideoRuntimeAdapter,
  installRuntimeHost,
  materializedVideoSource,
} from "@onmark/runtime";

import "./video-presentation.css";

const READINESS_TIMEOUT_MILLISECONDS = 5_000;

const adapter = new VideoRuntimeAdapter((placement, index) => {
  const element = document.createElement("video");
  element.dataset["placement"] = String(index);
  element.muted = true;
  element.playsInline = true;
  element.hidden = true;
  document.body.append(element);

  return {
    element,
    source: materializedVideoSource(placement),
    setVisible(visible): void {
      element.hidden = !visible;
    },
  };
}, READINESS_TIMEOUT_MILLISECONDS);

installRuntimeHost(adapter);
