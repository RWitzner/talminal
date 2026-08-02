/**
 * @vitest-environment happy-dom
 */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { HudSessionState } from "./Hud";
import Orb from "./Orb";
import { ORB_ERROR_FLASH_MS } from "./orbState";

describe("Orb", () => {
  let host: HTMLDivElement;
  let root: Root;
  let rafQueue: FrameRequestCallback[];
  let fakeNow: number;
  let mic: { level: number; at: number };
  let outputLevel: number;

  beforeEach(() => {
    (
      globalThis as typeof globalThis & {
        IS_REACT_ACT_ENVIRONMENT: boolean;
      }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    rafQueue = [];
    fakeNow = 10_000;
    mic = { level: 0, at: 0 };
    outputLevel = 0;
    vi.stubGlobal(
      "requestAnimationFrame",
      (callback: FrameRequestCallback) => rafQueue.push(callback),
    );
    vi.stubGlobal("cancelAnimationFrame", () => undefined);
    vi.spyOn(performance, "now").mockImplementation(() => fakeNow);
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  function render(session: HudSessionState, errorTick = 0): Promise<void> {
    return act(async () => {
      root.render(
        <Orb
          session={session}
          errorTick={errorTick}
          sessionLabel="Klar — hold tasten og tal"
          getMicLevel={() => mic}
          getOutputLevel={() => outputLevel}
        />,
      );
    });
  }

  function flushFrame(): Promise<void> {
    return act(async () => {
      const callbacks = rafQueue;
      rafQueue = [];
      for (const callback of callbacks) callback(fakeNow);
    });
  }

  function shell(): HTMLElement {
    const element = host.querySelector<HTMLElement>(".voice-orb");
    if (!element) throw new Error("voice-orb mangler i DOM'en");
    return element;
  }

  it("renderer med session-afledt data-mode og aria-label", async () => {
    await render("idle");
    expect(shell().getAttribute("data-mode")).toBe("idle");
    expect(
      host.querySelector('[role="status"]')?.getAttribute("aria-label"),
    ).toBe("Voice: Klar — hold tasten og tal");
  });

  it("niveau-motoren følger sessionskift og skriver --lvl", async () => {
    await render("idle");
    await render("listening");
    mic = { level: 1, at: fakeNow };
    await flushFrame();
    expect(shell().getAttribute("data-mode")).toBe("listening");
    // Én lerp-frame mod frisk mic-niveau 1: 0 + (1 - 0) * 0.16.
    expect(shell().style.getPropertyValue("--lvl")).toBe("0.160");
  });

  it("speaking driver --lvl fra output-niveauet", async () => {
    await render("speaking");
    outputLevel = 0.9;
    await flushFrame();
    expect(shell().style.getPropertyValue("--lvl")).toBe("0.144");
  });

  it("errorTick-bump blusser rødt hen over sessionen og udløber igen", async () => {
    await render("listening");
    await render("listening", 1);
    await flushFrame();
    expect(shell().getAttribute("data-mode")).toBe("flash-error");

    // Sessionen består — blusset kører sin fulde varighed og falder så
    // tilbage til den session-afledte tilstand.
    fakeNow += ORB_ERROR_FLASH_MS - 1;
    await flushFrame();
    expect(shell().getAttribute("data-mode")).toBe("flash-error");

    fakeNow += 1;
    await flushFrame();
    expect(shell().getAttribute("data-mode")).toBe("listening");
  });

  it("samme errorTick blusser ikke ved mount", async () => {
    await render("idle", 3);
    await flushFrame();
    expect(shell().getAttribute("data-mode")).toBe("idle");
  });
});
