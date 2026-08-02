/**
 * @vitest-environment happy-dom
 */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const occlusionMocks = vi.hoisted(() => ({
  setOcclusionReason: vi.fn(),
}));

// Task 7's occlusion-gate — mocket her saa vi kan assertere PAA kaldene
// (spec 4a, brief-spec c) uden at trække Tauri's invoke() ind i testen.
vi.mock("../browser/occlusion", () => ({
  setOcclusionReason: occlusionMocks.setOcclusionReason,
}));

import Hud, { type HudState } from "./Hud";

function state(overrides: Partial<HudState> = {}): HudState {
  return {
    session: "asleep",
    transcript: "",
    responseText: "",
    tool: null,
    resolver: null,
    error: null,
    chain: null,
    ...overrides,
  };
}

describe("Hud — bund-dock-chip + on-demand panel (spec 4a; bundhøjre pr. ejer-beslutning 2026-07-22)", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (
      globalThis as typeof globalThis & {
        IS_REACT_ACT_ENVIRONMENT: boolean;
      }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    occlusionMocks.setOcclusionReason.mockClear();
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  function render(hud: HudState): Promise<void> {
    return act(async () => {
      root.render(<Hud state={hud} />);
    });
  }

  function chipButton(): HTMLButtonElement {
    const button = host.querySelector("button[data-hud-chip]");
    if (!button) throw new Error("hud chip button ikke fundet");
    return button as HTMLButtonElement;
  }

  function clickChip(): Promise<void> {
    return act(async () => {
      chipButton().click();
    });
  }

  it("(a) kompakt chip er altid mounted og ejer ingen egen fixed-position (bund-docken i App.tsx placerer den)", async () => {
    await render(state());
    const region = host.querySelector('[aria-live="polite"]') as HTMLElement | null;
    expect(region).not.toBeNull();
    expect(region!.style.position).toBe("relative");
    expect(region!.style.top).toBe("");
    expect(region!.style.bottom).toBe("");
    expect(chipButton()).not.toBeNull();
  });

  it("(b) klik paa chippen toggler panelet", async () => {
    await render(state({ session: "speaking", transcript: "luk kort 3" }));
    expect(host.querySelector("[data-hud-panel]")).toBeNull();

    await clickChip();
    expect(host.querySelector("[data-hud-panel]")).not.toBeNull();
    expect(host.textContent).toContain("luk kort 3");

    await clickChip();
    expect(host.querySelector("[data-hud-panel]")).toBeNull();
  });

  it("(c) udfoldning registrerer occlusion-reason, sammenfoldning rydder den", async () => {
    await render(state());
    expect(occlusionMocks.setOcclusionReason).not.toHaveBeenCalledWith(
      "hud-panel",
      true,
    );

    await clickChip();
    expect(occlusionMocks.setOcclusionReason).toHaveBeenLastCalledWith(
      "hud-panel",
      true,
    );

    await clickChip();
    expect(occlusionMocks.setOcclusionReason).toHaveBeenLastCalledWith(
      "hud-panel",
      false,
    );
  });

  it("(c) unmount mens panelet er udfoldet rydder occlusion-reason", async () => {
    await render(state());
    await clickChip();
    expect(occlusionMocks.setOcclusionReason).toHaveBeenLastCalledWith(
      "hud-panel",
      true,
    );
    await act(async () => root.unmount());
    expect(occlusionMocks.setOcclusionReason).toHaveBeenLastCalledWith(
      "hud-panel",
      false,
    );
  });

  it("(d) aktive voice-tilstande er synlige i CHIPPEN (statusprik + aria-label/title — ikon-knap uden tekst) uden aabent panel", async () => {
    await render(state({ session: "listening" }));
    expect(chipButton().getAttribute("aria-label")).toContain("Lytter");
    expect(chipButton().getAttribute("title")).toBe("Lytter");
    expect(host.querySelector("[data-hud-panel]")).toBeNull();

    await render(state({ session: "processing" }));
    expect(chipButton().getAttribute("aria-label")).toContain("Fortolker");

    await render(state({ session: "speaking" }));
    expect(chipButton().getAttribute("aria-label")).toContain("Svarer");

    const dot = host.querySelector("[data-hud-dot]");
    expect(dot).not.toBeNull();
    expect(dot!.getAttribute("data-state")).toBe("active");
  });

  it("(d) en staaende fejl fremhaever chippen (fejlfarve) uden at aabne panelet automatisk", async () => {
    await render(state({ session: "idle", error: "STT fejlede" }));
    expect(host.querySelector("[data-hud-panel]")).toBeNull();
    expect(host.textContent).not.toContain("STT fejlede");

    const dot = host.querySelector("[data-hud-dot]");
    expect(dot!.getAttribute("data-state")).toBe("error");

    // Brugeren folder selv ud naar han vil se detaljen:
    await clickChip();
    expect(host.textContent).toContain("STT fejlede");
  });

  it("panelet gengiver stadig turindhold (transcript/tool/resolver) naar det er udfoldet manuelt", async () => {
    await render(
      state({
        session: "speaking",
        transcript: "luk kort 3",
        responseText: "Kort 3 blev lukket",
        tool: { name: "close_card", arguments: { card: 3 } },
        resolver: { ok: true, card: 3 },
      }),
    );
    await clickChip();
    expect(host.textContent).toContain("luk kort 3");
    expect(host.textContent).toContain("Kort 3 blev lukket");
    expect(host.textContent).toContain("close_card");
    expect(host.textContent).toContain("Mål: kort 3");
  });

  it("aria-live-regionen forbliver samme mountede DOM-knude paa tvaers af ture", async () => {
    await render(state());
    const region1 = host.querySelector('[aria-live="polite"]');
    await render(state({ session: "speaking", transcript: "x" }));
    const region2 = host.querySelector('[aria-live="polite"]');
    expect(region1).not.toBeNull();
    expect(region1).toBe(region2);
  });
});
