// Gate-one presentation composing author-owned layout with the browser runtime.
// Presentation owns elements and layout; runtime owns timing and readiness.

import {
  createDomPresentationBindings,
  createFontResource,
  createImageResource,
} from "@onmark/authoring";
import {
  PresentationRuntimeAdapter,
  installRuntimeHost,
  materializedVideoSource,
  type FrameEffect,
  type PresentationResource,
} from "@onmark/runtime";

import bodyFontSource from "./resources/basic-regular.ttf";
import posterSource from "./resources/poster.svg";
import "./video-presentation.css";

const READINESS_TIMEOUT_MILLISECONDS = 5_000;
const FRAME_ACCENT_PROPERTY = "--onmark-frame-accent";

const adapter = new PresentationRuntimeAdapter(
  createDomPresentationBindings({
    document,
    frameEffects: bindFrameEffects,
    resources: bindResources,
    videoSource: materializedVideoSource,
  }),
  READINESS_TIMEOUT_MILLISECONDS,
);

installRuntimeHost(adapter);

function bindFrameEffects(): readonly FrameEffect[] {
  const style = document.documentElement.style;
  return [
    {
      apply(frame): void {
        const accent = frame.index % 2 === 0 ? "#59d8ff" : "#ffcf59";
        style.setProperty(FRAME_ACCENT_PROPERTY, accent);
      },
      dispose(): void {
        style.removeProperty(FRAME_ACCENT_PROPERTY);
      },
    },
  ];
}

function bindResources(): readonly PresentationResource[] {
  const face = new FontFace(
    "Onmark Conformance",
    `url("${bodyFontSource}") format("truetype")`,
  );
  const font = createFontResource({
    face,
    fonts: document.fonts,
    id: "conformance-body",
  });
  const poster = createImageResource({
    document,
    id: "conformance-poster",
    source: posterSource,
  });
  poster.element.alt = "";
  poster.element.className = "onmark-poster";
  document.body.append(poster.element);

  return [poster, font];
}
