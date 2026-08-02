/**
 * @vitest-environment happy-dom
 */

// T9 (N4): en fejlet best-effort-spawn (fx codex ikke paa PATH, dvs. exe-
// oploesningen eller selve PTY-spawnet fejler — en slettet cwd taeller IKKE,
// den falder tavst tilbage til %USERPROFILE% i portable-pty)
// maa ikke vaere lydloes for operatoeren. Rust emitter "card-spawn-failed"
// (main.rs' create_card_impl), og App'en skal vise Rust-fejlteksten UAENDRET
// i HUD'ens fejlkanal + bumpe errorTick — samme moenster som
// worker-browser-tools-degraded-listeneren (App.tsx, mount-effekten der
// registrerer browser-kort-events).
//
// App renderes normalt IKKE i sin helhed i happy-dom (voice-effekterne, se
// kommentaren ved SettingsPanel i App.tsx) — her holdes workspace/cards
// BEVIDST paa null (list_cards/get_workspace afvises), saa hverken
// CanvasSurface eller den store voice-effekt (gated af voiceReady =
// workspace !== null) nogensinde starter. Kun mount-effekten med
// event-listenerne (inkl. den nye) koerer, hvilket er praecis det denne
// test daekker.

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  // Voice-soemmen: App'ens EGEN callback fanges ved konstruktion, saa testen
  // kan fyre praecis de events dispatcheren ville fyre.
  onHudEvent: { current: null as ((event: unknown) => void) | null },
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

// Settings-panelets indhold er irrelevant for denne soem og laver egen
// async invoke-IO — stub det hermetisk (samme moenster som
// App.settings.test.tsx), saa mountet forbliver stoejfrit.
vi.mock("./Settings", () => ({
  default: () => null,
  Settings: () => null,
}));

// Kort-gitteret (xterm-instantiering pr. kort) er irrelevant her; det mountes
// saa snart cards+workspace er ikke-null, hvilket voice-beskrivelsen nedenfor
// kraever.
vi.mock("./CanvasSurface", () => ({
  CanvasSurface: () => null,
}));

// Lyd-afspilleren stubbes: HUD'ens fejl-arm kalder sounds.play("error"),
// som konstruerer en AudioContext — den findes ikke i happy-dom, saa
// sound.ts' egen catch loggede en ReferenceError-stak i vitest-stdout ved
// hver koersel. Stoejen ville maskere en fremtidig aegte playback-regression
// (slut-review fix 5). Kun createSoundPlayer erstattes; withSoundFeedback og
// de rene render-funktioner beholdes fra det virkelige modul.
vi.mock("./voice/sound", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./voice/sound")>()),
  createSoundPlayer: () => ({
    play: () => {},
    close: async () => {},
  }),
}));

// Dispatcheren erstattes af sit eget interface: vi vil have fat i App'ens
// handleHudEvent — den funktion hvis info-arm er selve fejlen i T9-reviewet.
vi.mock("./voice/dispatch", () => ({
  createVoiceDispatcher: (deps: { onHudEvent(event: unknown): void }) => {
    mocks.onHudEvent.current = deps.onHudEvent;
    return { dispatch: async () => ({ ok: true }) };
  },
}));

import App from "./App";

describe("App — card-spawn-failed synliggoeres i HUD'ens fejlkanal (N4, T9)", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    mocks.listeners.clear();
    mocks.listen.mockReset().mockImplementation(
      async (
        event: string,
        handler: (event: { payload: unknown }) => void,
      ) => {
        mocks.listeners.set(event, handler);
        return () => mocks.listeners.delete(event);
      },
    );
    // list_cards/get_workspace afvises BEVIDST: cards/workspace forbliver
    // null, saa hverken CanvasSurface eller voiceReady-effekten starter
    // (se filens toppkommentar).
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

  function chipButton(): HTMLButtonElement {
    const button = host.querySelector("button[data-hud-chip]");
    if (!button) throw new Error("hud chip button ikke fundet");
    return button as HTMLButtonElement;
  }

  function emit(event: string, payload: unknown): Promise<void> {
    return act(async () => {
      mocks.listeners.get(event)?.({ payload });
    });
  }

  it("registrerer en card-spawn-failed-lytter ved mount", async () => {
    await render();
    expect(mocks.listeners.has("card-spawn-failed")).toBe(true);
  });

  it("emitteret card-spawn-failed viser Rust-fejlteksten uaendret + fremhaever HUD-chippen", async () => {
    await render();

    await emit("card-spawn-failed", {
      name: "card-3",
      number: 3,
      error: "codex.exe blev ikke fundet paa PATH (er codex installeret?)",
    });

    // Chippen er altid mounted og fremhaever fejl med det samme (Hud.tsx,
    // samme kontrakt som worker-browser-tools-degraded) — panelet folder
    // IKKE automatisk ud (ejer-beslutning 2026-07-22).
    const dot = host.querySelector("[data-hud-dot]");
    expect(dot?.getAttribute("data-state")).toBe("error");
    expect(host.querySelector("[data-hud-panel]")).toBeNull();

    // Brugeren folder selv ud for at se detaljen: fejlteksten fra
    // resolve_spawn_program skal staa der UAENDRET (ingen omskrivning).
    await act(async () => {
      chipButton().click();
    });
    expect(host.textContent).toContain(
      "codex.exe blev ikke fundet paa PATH (er codex installeret?)",
    );
  });

  it("rammer ogsaa et Claude-kort (ingen agent-gate — generel forbedring)", async () => {
    await render();

    await emit("card-spawn-failed", {
      name: "card-7",
      number: 7,
      error: "card is closing: card-7",
    });

    await act(async () => {
      chipButton().click();
    });
    expect(host.textContent).toContain("card is closing: card-7");
  });
});

// ---------------------------------------------------------------------------
// T9-review (fix-runde 1): backend-fejlens levetid i HUD'en.
//
// Reviewerens fund: create_card returnerer Ok(info) OGSAA naar best-effort-
// spawn'et fejlede, saa voice-vejens new_card fyrer sit rutine-info-event
// ("N kort oprettet i ...") LIGE efter Rust har emittet card-spawn-failed.
// handleHudEvent's info-arm satte error: null — altsaa blev den nye HUD-fejl
// visket ud millisekunder efter den kom, og operatoeren sad tilbage med et
// blus og en success-tekst. Her mountes App'en MED workspace, saa den store
// voice-effekt (gated af voiceReady) reelt koerer og de to kanaler moedes.
// ---------------------------------------------------------------------------

const WORKSPACE = {
  schema_version: 1,
  next_card_number: 1,
  viewport: { x: 0, y: 0, zoom: 1 },
  settings: {
    ptt_hotkey: "CmdOrCtrl+Shift+Space",
    exit_type_mode_hotkey: "CmdOrCtrl+Shift+E",
    voice_engine: "pipeline",
    wallpaper: "",
    default_agent: "claude",
  },
  cards: [],
};

const SPAWN_ERROR = "codex.exe blev ikke fundet paa PATH (er codex installeret?)";

describe("App — backend-fejl overlever voice-vejens rutine-success (T9-review)", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    mocks.listeners.clear();
    mocks.onHudEvent.current = null;
    mocks.listen.mockReset().mockImplementation(
      async (
        event: string,
        handler: (event: { payload: unknown }) => void,
      ) => {
        mocks.listeners.set(event, handler);
        return () => mocks.listeners.delete(event);
      },
    );
    // Her SKAL workspace vaere ikke-null: det er gaten for voice-effekten.
    mocks.invoke.mockReset().mockImplementation(async (command: string) => {
      if (command === "list_cards") return [];
      if (command === "get_workspace") return WORKSPACE;
      if (command === "get_project")
        return { root: "C:\\dev\\demo", name: "demo" };
      if (command === "get_cards_status")
        return { path: "cards.toml", missing: false, error: null };
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

  async function render(): Promise<void> {
    await act(async () => {
      root.render(<App />);
    });
    if (mocks.onHudEvent.current === null) {
      throw new Error("voice-dispatcheren blev aldrig konstrueret (voiceReady?)");
    }
  }

  function emit(event: string, payload: unknown): Promise<void> {
    return act(async () => {
      mocks.listeners.get(event)?.({ payload });
    });
  }

  function hudEvent(event: unknown): Promise<void> {
    return act(async () => {
      mocks.onHudEvent.current?.(event);
    });
  }

  function dotState(): string | null | undefined {
    return host.querySelector("[data-hud-dot]")?.getAttribute("data-state");
  }

  /** Panelet aabnes ALDRIG automatisk (ejer-beslutning) — brugeren folder selv ud. */
  function openPanel(): Promise<void> {
    return act(async () => {
      const button = host.querySelector("button[data-hud-chip]");
      if (!button) throw new Error("hud chip button ikke fundet");
      (button as HTMLButtonElement).click();
    });
  }

  it("card-spawn-failed efterfulgt af voice-success: fejlteksten staar stadig", async () => {
    await render();

    // Den rapporterede raekkefoelge, 1:1: Rust emitter FOER kommandoen
    // returnerer, saa info-eventet lander sidst.
    await emit("card-spawn-failed", {
      name: "card-4",
      number: 4,
      error: SPAWN_ERROR,
    });
    await hudEvent({
      kind: "info",
      message: "1 codex-kort oprettet i C:\\dev\\demo",
    });

    expect(dotState()).toBe("error");
    await openPanel();
    expect(host.textContent).toContain(SPAWN_ERROR);
  });

  it("samme beskyttelse for worker-browser-tools-degraded (soeskende-lytter)", async () => {
    await render();

    await emit("worker-browser-tools-degraded", {
      card: "card-2",
      reason: "mcp-config manglede",
    });
    await hudEvent({ kind: "info", message: "1 kort oprettet i C:\\dev" });

    expect(dotState()).toBe("error");
    await openPanel();
    expect(host.textContent).toContain("browser-tools mangler");
  });

  it("samme beskyttelse for browser-keeper-hijacked (soeskende-lytter)", async () => {
    await render();

    await emit("browser-keeper-hijacked", {
      scope: "worker",
      owner: "card-5",
    });
    await hudEvent({ kind: "status", message: "kort 5 kigger", card: 5, data: null });

    expect(dotState()).toBe("error");
    await openPanel();
    expect(host.textContent).toContain("browser-styring udenom kortene");
  });

  it("en NY voice-fejl afloeser spawn-fejlen (fejl bliver ikke udoedelige)", async () => {
    await render();

    await emit("card-spawn-failed", {
      name: "card-4",
      number: 4,
      error: SPAWN_ERROR,
    });
    await hudEvent({ kind: "error", message: "Kortet findes ikke" });

    expect(dotState()).toBe("error");
    await openPanel();
    expect(host.textContent).toContain("Kortet findes ikke");
    expect(host.textContent).not.toContain(SPAWN_ERROR);
  });

});
