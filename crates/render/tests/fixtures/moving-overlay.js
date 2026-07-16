// Shared visual primitive for comparing browser and split-layer composition.

export function drawMovingOverlay(context, index, width, height) {
  const overlayWidth = Math.min(520, width - 64);
  const travel = Math.max(0, width - overlayWidth - 64);
  const x = 32 + (travel * (index % 60)) / 59;
  const y = height - 180;
  context.fillStyle = "rgb(8 15 32 / 72%)";
  context.fillRect(x, y, overlayWidth, 116);
  context.fillStyle = "white";
  context.font = "700 42px sans-serif";
  context.fillText("Onmark", x + 28, y + 70);
}
