/**
 * @vitest-environment happy-dom
 */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { CanvasLiquidGlass } from "./liquidGlass";

describe("canvas liquid-glass background", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (
      globalThis as typeof globalThis & {
        IS_REACT_ACT_ENVIRONMENT: boolean;
      }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  it("renderer refraction, smoke og highlights over hele wallpaperet", async () => {
    await act(async () => {
      root.render(<CanvasLiquidGlass wallpaperUrl="/wallpaper.png" />);
    });

    const glass = host.querySelector<HTMLElement>("[data-canvas-liquid-glass]");
    const refraction = host.querySelector<HTMLElement>(
      "[data-canvas-liquid-refraction]",
    );
    const wallpaper = host.querySelector<HTMLElement>(
      "[data-canvas-liquid-wallpaper]",
    );
    expect(glass).not.toBeNull();
    expect(host.querySelector("#canvas-liquid-refraction")).not.toBeNull();
    expect(host.querySelector("[data-canvas-liquid-smoke]")).not.toBeNull();
    expect(host.querySelector("[data-canvas-liquid-highlight]")).not.toBeNull();
    expect(wallpaper?.style.backgroundImage).toContain("/wallpaper.png");
    expect(refraction?.style.filter).toContain(
      'url("#canvas-liquid-refraction")',
    );
    expect(wallpaper?.style.filter).toContain("blur(7px)");
  });

  it("renderer liquid-only uden wallpaper-lag men med transparent materiale", async () => {
    await act(async () => {
      root.render(<CanvasLiquidGlass wallpaperUrl={null} />);
    });

    const glass = host.querySelector<HTMLElement>("[data-canvas-liquid-glass]");
    const refraction = host.querySelector<HTMLElement>(
      "[data-canvas-liquid-refraction]",
    );
    const material = host.querySelector<HTMLElement>(
      "[data-canvas-liquid-material]",
    );
    expect(glass?.dataset.canvasLiquidMode).toBe("liquid-only");
    expect(glass?.style.background).toBe("transparent");
    expect(host.querySelector("[data-canvas-liquid-wallpaper]")).toBeNull();
    expect(material?.style.background).toContain("linear-gradient");
    expect(material?.style.background).toContain("rgba");
    expect(host.querySelector("[data-canvas-liquid-smoke]")).not.toBeNull();
    expect(host.querySelector("[data-canvas-liquid-highlight]")).not.toBeNull();
    expect(refraction?.style.filter).toContain(
      'url("#canvas-liquid-refraction")',
    );
  });
});
