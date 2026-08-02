/**
 * @vitest-environment happy-dom
 */

// SLUTREVIEW B2: spec §6.2 er ordret — "`reveal()` udloeser et frontend-refresh
// af settings og kortliste foer eller som del af ack'en. Uden det viser et
// laengere skjult workspace gammel wallpaper og gamle hotkeys."
//
// Rust-siden HAVDE seamen: `TauriSurface::reveal` emitter `workspace-revealed`
// (workspaces/surface.rs). Men eventet havde NUL lyttere i hele repoet, saa
// halvdelen af kravet fandtes ikke.
//
// Konsekvensen er ikke kosmetisk. `settings.json` er GLOBAL
// (`project::global_base()`), saa en aendring i workspace A rammer disken
// oejeblikkeligt — men aldrig det skjulte workspace B's React-state. B's
// `workspace.settings.ptt_hotkey` bliver derfor staaende paa den GAMLE vaerdi og
// sendes videre til `registerWakeKey`/`configure_wake_hotkey`: den nye hotkey
// gør ingenting i B, mens den gamle stadig er optaget af B's proces. Uden en
// periodisk poll (App.tsx dokumenterer selv "kun events trigger refresh") retter
// det sig foerst ved en genstart af B.
//
// Samme harness som App.cardsChanged.test.tsx: cards/workspace holdes BEVIDST
// paa null (list_cards/get_workspace afvises), saa hverken CanvasSurface eller
// den store voice-effekt starter. Kun mount-effekten med event-listenerne
// koerer, og det er praecis den der testes.

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

describe("App — workspace-revealed henter settings og kortliste igen", () => {
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

  function callsTo(command: string): number {
    return mocks.invoke.mock.calls.filter((call) => call[0] === command).length;
  }

  it("registrerer en workspace-revealed-lytter ved mount", async () => {
    await render();
    expect(mocks.listeners.has("workspace-revealed")).toBe(true);
  });

  // `get_workspace` er kilden til `settings.wallpaper` og `settings.ptt_hotkey`
  // — de to vaerdier fundet handler om. Derfor taelles netop den, og ikke bare
  // kortlisten.
  it("et emitteret workspace-revealed henter settings igen", async () => {
    await render();
    const foer = callsTo("get_workspace");

    await act(async () => {
      mocks.listeners.get("workspace-revealed")?.({ payload: undefined });
    });

    expect(callsTo("get_workspace")).toBeGreaterThan(foer);
  });

  it("…og kortlisten med, saa et skjult workspace ikke vaagner med gamle kort", async () => {
    await render();
    const foer = callsTo("list_cards");

    await act(async () => {
      mocks.listeners.get("workspace-revealed")?.({ payload: undefined });
    });

    expect(callsTo("list_cards")).toBeGreaterThan(foer);
  });
});
