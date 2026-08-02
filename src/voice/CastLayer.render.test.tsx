/**
 * @vitest-environment happy-dom
 */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emitCast } from "./cast";
import CastLayer from "./CastLayer";

describe("CastLayer", () => {
  let host: HTMLDivElement;
  let root: Root;
  let animateTargets: Element[];
  let reducedMotion: boolean;
  let originalAnimate: typeof Element.prototype.animate | undefined;

  beforeEach(() => {
    (
      globalThis as typeof globalThis & {
        IS_REACT_ACT_ENVIRONMENT: boolean;
      }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    vi.useFakeTimers({
      toFake: [
        "setTimeout",
        "clearTimeout",
        "requestAnimationFrame",
        "cancelAnimationFrame",
        "performance",
      ],
    });
    animateTargets = [];
    reducedMotion = false;
    // happy-dom har ingen stabil WAAPI — stub animate og registrér målene.
    originalAnimate = Element.prototype.animate;
    Element.prototype.animate = function (this: Element) {
      animateTargets.push(this);
      return {
        onfinish: null,
        cancel() {},
        finished: Promise.resolve(),
      } as unknown as Animation;
    } as typeof Element.prototype.animate;
    vi.stubGlobal("matchMedia", (media: string) => ({
      matches: reducedMotion,
      media,
      addEventListener() {},
      removeEventListener() {},
    }));
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    if (originalAnimate) {
      Element.prototype.animate = originalAnimate;
    } else {
      delete (Element.prototype as { animate?: unknown }).animate;
    }
    document.body.replaceChildren();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  function render(): Promise<void> {
    return act(async () => {
      root.render(<CastLayer />);
    });
  }

  function addOrb(): HTMLElement {
    const orb = document.createElement("div");
    orb.setAttribute("data-voice-orb", "");
    document.body.append(orb);
    return orb;
  }

  function addCard(number: number): HTMLElement {
    const card = document.createElement("div");
    card.setAttribute("data-card-frame", "");
    card.setAttribute("data-card-number", String(number));
    document.body.append(card);
    return card;
  }

  function paths(): number {
    return host.querySelectorAll("path").length;
  }

  function mockRect(
    el: Element,
    rect: { left: number; top: number; width: number; height: number },
  ) {
    el.getBoundingClientRect = () =>
      ({
        left: rect.left,
        top: rect.top,
        width: rect.width,
        height: rect.height,
        right: rect.left + rect.width,
        bottom: rect.top + rect.height,
        x: rect.left,
        y: rect.top,
      }) as DOMRect;
  }

  /** Endepunktet (p2) af foerste beam-path: "M x y Q cx cy x2 y2". */
  function beamEndpoint(): { x: number; y: number } {
    const d = host.querySelector("path")?.getAttribute("d") ?? "";
    const parts = d.split(" ");
    return {
      x: Number(parts.at(-2)),
      y: Number(parts.at(-1)),
    };
  }

  it("tegner straale-paths straks og nedslags-gloed ved tegne-slut", async () => {
    addOrb();
    const card = addCard(2);
    await render();
    await act(async () => {
      emitCast({ card: 2, landing: "card" });
    });
    expect(paths()).toBe(2);
    expect(animateTargets).not.toContain(card);
    await act(async () => {
      vi.advanceTimersByTime(200);
    });
    expect(animateTargets).toContain(card);
    // fade + oprydning: alle fx-elementer er vaek igen
    await act(async () => {
      vi.advanceTimersByTime(1_000);
    });
    expect(paths()).toBe(0);
  });

  it("venter paa spawn-mount og affyrer naar kortet dukker op", async () => {
    addOrb();
    await render();
    await act(async () => {
      emitCast({ card: 4, landing: "card" });
    });
    expect(paths()).toBe(0);
    addCard(4);
    await act(async () => {
      vi.advanceTimersByTime(100);
    });
    expect(paths()).toBe(2);
  });

  it("dropper tavst naar maalkortet aldrig mounter", async () => {
    addOrb();
    await render();
    await act(async () => {
      emitCast({ card: 9, landing: "prompt" });
    });
    await act(async () => {
      vi.advanceTimersByTime(2_000);
    });
    expect(paths()).toBe(0);
    expect(animateTargets).toEqual([]);
  });

  it("reduced motion: ingen straale, kun nedslags-gloed", async () => {
    reducedMotion = true;
    addOrb();
    const card = addCard(3);
    await render();
    await act(async () => {
      emitCast({ card: 3, landing: "prompt" });
    });
    expect(paths()).toBe(0);
    expect(animateTargets).toEqual([card]);
  });

  it("prompt-landing sigter paa xterm-tekstfeltet naar det findes i kortet", async () => {
    addOrb();
    const card = addCard(5);
    mockRect(card, { left: 100, top: 100, width: 300, height: 200 });
    const body = document.createElement("div");
    body.setAttribute("data-card-body", "");
    mockRect(body, { left: 100, top: 130, width: 300, height: 170 });
    const field = document.createElement("textarea");
    field.className = "xterm-helper-textarea";
    mockRect(field, { left: 150, top: 260, width: 2, height: 2 });
    body.append(field);
    card.append(body);
    await render();
    await act(async () => {
      emitCast({ card: 5, landing: "prompt" });
    });
    expect(beamEndpoint()).toEqual({ x: 151, y: 261 });
  });

  it("prompt-landing falder tilbage til bunden af kortkroppen uden tekstfelt", async () => {
    addOrb();
    const card = addCard(5);
    mockRect(card, { left: 100, top: 100, width: 300, height: 200 });
    const body = document.createElement("div");
    body.setAttribute("data-card-body", "");
    mockRect(body, { left: 100, top: 120, width: 300, height: 180 });
    card.append(body);
    await render();
    await act(async () => {
      emitCast({ card: 5, landing: "prompt" });
    });
    // bund minus min(48, 15 % af hoejden 180) = 300 - 27
    expect(beamEndpoint()).toEqual({ x: 250, y: 273 });
  });

  it("prompt-landing ignorerer et tekstfelt UDEN FOR kortkroppen", async () => {
    addOrb();
    const card = addCard(5);
    mockRect(card, { left: 100, top: 100, width: 300, height: 200 });
    const body = document.createElement("div");
    body.setAttribute("data-card-body", "");
    mockRect(body, { left: 100, top: 120, width: 300, height: 180 });
    const field = document.createElement("textarea");
    field.className = "xterm-helper-textarea";
    mockRect(field, { left: 0, top: 0, width: 2, height: 2 });
    body.append(field);
    card.append(body);
    await render();
    await act(async () => {
      emitCast({ card: 5, landing: "prompt" });
    });
    expect(beamEndpoint()).toEqual({ x: 250, y: 273 });
  });

  it("card-landing sigter paa kortets centrum", async () => {
    addOrb();
    const card = addCard(5);
    mockRect(card, { left: 100, top: 100, width: 300, height: 200 });
    await render();
    await act(async () => {
      emitCast({ card: 5, landing: "card" });
    });
    expect(beamEndpoint()).toEqual({ x: 250, y: 200 });
  });

  it("forskyder casts der ankommer i samme burst", async () => {
    addOrb();
    addCard(1);
    addCard(2);
    await render();
    await act(async () => {
      emitCast({ card: 1, landing: "prompt" });
      emitCast({ card: 2, landing: "prompt" });
    });
    expect(paths()).toBe(2);
    await act(async () => {
      vi.advanceTimersByTime(60);
    });
    expect(paths()).toBe(4);
  });

  it("unmount rydder op og efterlader ingen levende timere", async () => {
    addOrb();
    await render();
    await act(async () => {
      // kortet findes ikke — rAF-venten er i gang
      emitCast({ card: 8, landing: "card" });
    });
    await act(async () => root.unmount());
    expect(() => {
      emitCast({ card: 8, landing: "card" });
      vi.advanceTimersByTime(2_000);
    }).not.toThrow();
    expect(document.querySelectorAll("path").length).toBe(0);
  });
});
