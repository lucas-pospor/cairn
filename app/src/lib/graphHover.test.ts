// The graph's hover box (graphHover.ts): drawn in the colours it is given, not
// sigma's white with a black shadow.
//
// Run: cd app && npx vitest run src/lib/graphHover.test.ts

import { describe, expect, it } from "vitest";
import type { Settings } from "sigma/settings";
import { drawNodeHover } from "./graphHover";

/** A canvas context that records the colour of each fill, stroke and text, and any shadow drawn with it. */
function recorder() {
  const drawn: string[] = [];
  const shadow = () => (ctx.shadowBlur ? ` shadow ${ctx.shadowColor}` : "");
  const ctx = {
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 0,
    font: "",
    shadowBlur: 0,
    shadowColor: "",
    measureText: (s: string) => ({ width: s.length * 7 }),
    beginPath() {},
    moveTo() {},
    lineTo() {},
    arc() {},
    closePath() {},
    fill() {
      drawn.push(`fill ${ctx.fillStyle}${shadow()}`);
    },
    stroke() {
      drawn.push(`stroke ${ctx.strokeStyle}${shadow()}`);
    },
    fillText(text: string) {
      drawn.push(`text ${ctx.fillStyle}${shadow()} ${text}`);
    },
  };
  return { ctx: ctx as unknown as CanvasRenderingContext2D, drawn };
}

const settings = { labelSize: 12, labelWeight: "normal", labelFont: "sans-serif", labelColor: { color: "#dfe3e0" } } as unknown as Settings;
const colours = { box: "#23272a", outline: "#6f787e" };

describe("the graph's hover box", () => {
  it("is drawn in the theme's colours, with the label in the label colour", () => {
    const { ctx, drawn } = recorder();
    drawNodeHover(ctx, { x: 10, y: 10, size: 4, label: "Beans", color: "#888" }, settings, colours);
    // No shadow under any of them (sigma's box had a black one).
    expect(drawn).toEqual(["fill #23272a", "stroke #6f787e", "text #dfe3e0 Beans"]);
  });

  it("draws the box around a node with no label, and no text", () => {
    const { ctx, drawn } = recorder();
    drawNodeHover(ctx, { x: 10, y: 10, size: 4, label: null, color: "#888" }, settings, colours);
    expect(drawn).toEqual(["fill #23272a", "stroke #6f787e"]);
  });
});
