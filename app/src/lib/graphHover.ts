// The box behind the label of a hovered node in the graph, in the theme's
// colours. Sigma's own (drawDiscNodeHover) is always white with a black
// shadow, so the light label of a dark theme was barely readable on it. This
// draws the same shape in --bg-input with a --border-strong outline, and the
// label in --text, a pair the contrast test checks for every theme.

import type { Settings } from "sigma/settings";
import type { NodeDisplayData, PartialButFor } from "sigma/types";

export interface HoverColours {
  /** The box: --bg-input. */
  box: string;
  /** Its outline: --border-strong. */
  outline: string;
}

type Data = PartialButFor<NodeDisplayData, "x" | "y" | "size" | "label" | "color">;

const PADDING = 2;

export function drawNodeHover(context: CanvasRenderingContext2D, data: Data, settings: Settings, colours: HoverColours): void {
  const size = settings.labelSize;
  context.font = `${settings.labelWeight} ${size}px ${settings.labelFont}`;
  context.beginPath();
  if (typeof data.label === "string") {
    const boxWidth = Math.round(context.measureText(data.label).width + 5);
    const boxHeight = Math.round(size + 2 * PADDING);
    const radius = Math.max(data.size, size / 2) + PADDING;
    const angle = Math.asin(boxHeight / 2 / radius);
    const dx = Math.sqrt(Math.abs(radius ** 2 - (boxHeight / 2) ** 2));
    context.moveTo(data.x + dx, data.y + boxHeight / 2);
    context.lineTo(data.x + radius + boxWidth, data.y + boxHeight / 2);
    context.lineTo(data.x + radius + boxWidth, data.y - boxHeight / 2);
    context.lineTo(data.x + dx, data.y - boxHeight / 2);
    context.arc(data.x, data.y, radius, angle, -angle);
  } else {
    context.arc(data.x, data.y, data.size + PADDING, 0, Math.PI * 2);
  }
  context.closePath();
  context.fillStyle = colours.box;
  context.fill();
  context.lineWidth = 1;
  context.strokeStyle = colours.outline;
  context.stroke();
  if (!data.label) return;
  context.fillStyle = settings.labelColor.color ?? "#000";
  context.fillText(data.label, data.x + data.size + 3, data.y + size / 3);
}
