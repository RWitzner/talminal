/**
 * @vitest-environment happy-dom
 */

// Dogfood-fund 2026-07-25 (operatoer-roegtesten for agent-til-agent):
// `card_pair` lykkedes og returnerede baade partner og chat-kort — men
// INGEN af dem dukkede op paa canvas. De laa i backenden hele tiden.
//
// Aarsagen laa her i App'en: DENGANG blev kortlisten hentet ved mount og
// derefter KUN paa browser-card-updated/browser-card-dead. Kort som en AGENT
// skabte havde altsaa ingen vej ind i fladen og blev foerst synlige da ejeren
// tilfaeldigvis selv oprettede et kort — hvis create_card-vej selv kalder
// refresh().
//
// SAADAN ER DET IKKE LAENGERE. Rust emitter "cards-changed", App'en lytter paa
// det (og paa "workspace-revealed"), og foerste hentning ligger EFTER
// registreringen af dem alle (M23 — den raekkefoelge pinnes i
// App.listenerOrder.test.tsx). Polling findes til gengaeld stadig ikke, saa
// lytteren er fortsat den eneste vej ind for agent-skabte kort. Denne fil
// pinner netop den: at lytteren findes, og at et emitteret cards-changed
// henter listen igen.
//
// Samme harness som App.spawnFailed.test.tsx: cards/workspace holdes BEVIDST
// paa null (list_cards/get_workspace afvises), saa hverken CanvasSurface eller
// den store voice-effekt starter. Kun lytter-effekten koerer, og det er praecis
// den der testes.

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

vi.mock("./Settings", () => ({
  default: () => null,
  Settings: () => null,
}));

vi.mock("./CanvasSurface", () => ({
  CanvasSurface: () => null,
}));

vi.mock("./voice/sound", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./voice/sound")>()),
  createSoundPlayer: () => ({
    play: () => {},
    close: async () => {},
  }),
}));

vi.mock("./voice/dispatch", () => ({
  createVoiceDispatcher: () => ({ dispatch: async () => ({ ok: true }) }),
}));

import App from "./App";

describe("App — cards-changed henter kortlisten igen (dogfood-fund)", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    mocks.listeners.clear();
    mocks.listen.mockReset().mockImplementation(
      async (event: string, handler: (event: { payload: unknown }) => void) => {
        mocks.listeners.set(event, handler);
        return () => mocks.listeners.delete(event);
      },
    );
    mocks.invoke.mockReset().mockImplementation(async (command: string) => {
      if (command === "list_cards" || command === "get_workspace") {
        throw new Error("test: ingen workspace");
      }
      return undefined;
    });
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  function render(): Promise<void> {
    return act(async () => {
      root.render(<App />);
    });
  }

  function listCardsCalls(): number {
    return mocks.invoke.mock.calls.filter((call) => call[0] === "list_cards")
      .length;
  }

  it("registrerer en cards-changed-lytter ved mount", async () => {
    await render();
    expect(mocks.listeners.has("cards-changed")).toBe(true);
  });

  it("et emitteret cards-changed henter kortlisten igen", async () => {
    await render();
    const before = listCardsCalls();

    await act(async () => {
      mocks.listeners.get("cards-changed")?.({ payload: {} });
    });

    expect(listCardsCalls()).toBeGreaterThan(before);
  });
});
