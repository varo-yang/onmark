// Gate-one presentation proving the production browser-video adapter.
// Presentation owns elements and layout; runtime owns timing and readiness.

import {
  PresentationRuntimeAdapter,
  installRuntimeHost,
  materializedVideoSource,
  type RuntimeOverlay,
} from "@onmark/runtime";

import "./video-presentation.css";

const READINESS_TIMEOUT_MILLISECONDS = 5_000;

const adapter = new PresentationRuntimeAdapter(
  {
    bindVideo(placement, index) {
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
        dispose(): void {
          element.remove();
        },
      };
    },
    bindOverlay(placement, index) {
      const element = document.createElement("div");
      element.className = `onmark-overlay ${overlayClass(placement)}`;
      element.dataset["placement"] = String(index);
      element.textContent = placement.text;
      element.hidden = true;
      document.body.append(element);

      return {
        setVisible(visible): void {
          element.hidden = !visible;
        },
        dispose(): void {
          element.remove();
        },
      };
    },
  },
  READINESS_TIMEOUT_MILLISECONDS,
);

installRuntimeHost(adapter);

function overlayClass(overlay: RuntimeOverlay): string {
  switch (overlay.kind) {
    case "title":
      return "onmark-title";
    case "callToAction":
      return "onmark-call-to-action";
  }
}
