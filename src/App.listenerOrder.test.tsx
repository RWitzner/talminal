/**
 * @vitest-environment happy-dom
 */

// M23: kortlisten blev hentet FOER lytterne var registreret.
//
// `listen()` er en aegte asynkron round-trip til Rust (invoke('plugin:event|listen')),
// saa der laa et reelt vindue mellem mount-hentningen og registreringen. Alt
// hvad backenden emitterede dér — `cards-changed`, `browser-card-updated`,
// `browser-card-dead`, `workspace-revealed` — faldt paa gulvet uden lytter, og
// INGEN af dem gentages. Der er heller ingen periodisk poll (App'ens egen
// kommentar ved `refresh` siger det ordret), saa tabet var VARIGT: kortlisten
// stod forkert til naeste gang noget tilfaeldigvis aendrede sig.
//
// Praecis den klasse fejl kostede allerede én gang: card_pair's partner- og
// chat-kort laa usynlige, fordi fladen ikke pollede kortlisten. Denne fil pinner
// raekkefoelgen, saa vinduet ikke kan komme igen.
//
// Harnessen svarer med et RIGTIGT snapshot (forlaeg: anden describe i
// App.spawnFailed.test.tsx). Det er et bevidst valg: afviste den
// `list_cards`/`get_workspace` — som soesterfilerne goer for at holde
// CanvasSurface og voice-effekten ude — ville `refresh` altid ende i sin
// catch-arm, og testen kunne kun bevise at der blev KALDT igen. Pointen med
// M23 er at frisk DATA lander, saa den skal kunne lande.
//
// Det baerende greb er porten: den mockede `listen` resolver foerst naar testen
// aabner den, og imens ER vi i registreringsvinduet.

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  // Kortlisten som gitteret sidst blev rendret med. `null` betyder "aldrig
  // mountet", altsaa at App'en stadig staar i sin null-tilstand.
  canvasCards: { current: null as unknown[] | null },
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

// Selve kort-gitteret (xterm pr. kort) er irrelevant her — men OM det mountes
// er ikke: App'en rendrer det kun naar BAADE cards og workspace har forladt
// null. Stubben noterer derfor den kortliste den blev kaldt med og bliver
// dermed testens "snapshot'et landede"-soem.
vi.mock("./CanvasSurface", () => ({
  CanvasSurface: (props: { cards: unknown[] }) => {
    mocks.canvasCards.current = props.cards;
    return null;
  },
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

// Navnet er med vilje ikke en delstreng af stien: topbaren viser BEGGE, og
// assertionen skal kunne skelne dem.
const PROJECT = { root: "C:\\dev\\lytterorden", name: "ordens-projektet" };

describe("App — kortlisten hentes foerst naar lytterne staar (M23)", () => {
  let host: HTMLDivElement;
  let root: Root;
  let aabnPorten: () => void;

  beforeEach(() => {
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    mocks.listeners.clear();
    mocks.canvasCards.current = null;
    // Porten holder registreringen aaben lige saa laenge testen vil. I den
    // rigtige app er ventetiden IPC'ens egen; her er den styrbar.
    const port = new Promise<void>((open) => {
      aabnPorten = open;
    });
    mocks.listen.mockReset().mockImplementation(
      async (event: string, handler: (event: { payload: unknown }) => void) => {
        await port;
        mocks.listeners.set(event, handler);
        return () => mocks.listeners.delete(event);
      },
    );
    // Et komplet, gyldigt snapshot: kun saaledes kan testen se forskel paa "der
    // blev kaldt igen" og "der landede friske data" (se filens topkommentar).
    mocks.invoke.mockReset().mockImplementation(async (command: string) => {
      if (command === "list_cards") return [];
      if (command === "get_workspace") return WORKSPACE;
      if (command === "get_project") return PROJECT;
      if (command === "get_cards_status")
        return { path: "cards.toml", missing: false, error: null };
      return undefined;
    });
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    aabnPorten();
    await act(async () => root.unmount());
    host.remove();
  });

  function render(): Promise<void> {
    return act(async () => {
      root.render(<App />);
    });
  }

  // En makrotask-graense draener alle ventende mikrotasks paa én gang, og
  // kaeden port -> listen -> Promise.all -> refresh -> loadAppSnapshot er flere
  // led lang.
  function tik(): Promise<void> {
    return act(async () => {
      await new Promise<void>((done) => setTimeout(done, 0));
    });
  }

  function listCardsKald(): number {
    return mocks.invoke.mock.calls.filter((call) => call[0] === "list_cards")
      .length;
  }

  /** Topbaren viser `project.name` fra snapshot'et — og "Workspace" saa laenge
   *  intet snapshot er landet. Den er dermed rendret bevis, ikke et kald. */
  function topbarTekst(): string {
    const topbar = host.querySelector("[data-global-topbar]");
    if (!topbar) throw new Error("topbaren blev ikke rendret");
    return topbar.textContent ?? "";
  }

  it("henter intet snapshot mens registreringen stadig er i luften", async () => {
    await render();
    await tik();

    // Vinduet er aabent: der er endnu ingen der kan fange et cards-changed.
    expect(mocks.listeners.size).toBe(0);
    expect(listCardsKald()).toBe(0);
  });

  it("henter snapshot'et FOERST naar lytterne er registreret", async () => {
    await render();
    await tik();
    // Backenden emitter i vinduet. Der er ingen lytter, saa eventet er tabt for
    // altid — og uden en hentning bagefter ville kortlisten aldrig blive frisk.
    expect(mocks.listeners.get("cards-changed")).toBeUndefined();
    const foer = listCardsKald();
    // Diskriminatoren: laa hentningen stadig i mount-effekten, var snapshot'et
    // landet HER — gitteret mountet og projektnavnet i topbaren.
    expect(mocks.canvasCards.current).toBeNull();
    expect(topbarTekst()).toContain("Workspace");
    expect(topbarTekst()).not.toContain(PROJECT.name);

    aabnPorten();
    await tik();

    expect(mocks.listeners.has("cards-changed")).toBe(true);
    // Supplement: der blev kaldt igen ...
    expect(listCardsKald()).toBeGreaterThan(foer);
    // ... men det afgoerende er at DATA landede. App'en har forladt sin
    // null-tilstand: gitteret rendres kun naar baade cards og workspace er
    // sat, og topbaren viser snapshot'ets projekt i stedet for pladsholderen.
    expect(mocks.canvasCards.current).toEqual([]);
    expect(topbarTekst()).toContain(PROJECT.name);
  });

  // Bagsiden af den bindende raekkefoelge: naar registreringen er foerste
  // hentnings forudsaetning, koster ét afvist `listen()`-kald BAADE kortene og
  // — indtil dette fix — enhver besked om hvorfor. Dengang `refresh()` laa i sin
  // egen effekt, gav en doed IPC i det mindste `loadError`-notitsen; her
  // resolvede den async IIFE aldrig, og rejection'en forsvandt som en
  // uhaandteret promise.
  it("et afvist listen-kald ender som synlig fejl, ikke som tavshed", async () => {
    const konsol = vi.spyOn(console, "error").mockImplementation(() => {});
    mocks.listen
      .mockReset()
      .mockImplementation(
        async (
          event: string,
          handler: (event: { payload: unknown }) => void,
        ) => {
          if (event === "cards-changed") {
            throw new Error("plugin:event|listen afvist");
          }
          mocks.listeners.set(event, handler);
          return () => mocks.listeners.delete(event);
        },
      );
    try {
      await render();
      await tik();

      // Ingen kort — det er uundgaaeligt uden lyttere ...
      expect(mocks.canvasCards.current).toBeNull();
      // ... men fladen SIGER hvorfor, i baade notitsen og konsollen.
      expect(host.textContent).toContain(
        "kortliste-lytterne kunne ikke registreres",
      );
      expect(konsol).toHaveBeenCalledWith(
        "kortliste-lyttere fejlede:",
        expect.any(Error),
      );
    } finally {
      konsol.mockRestore();
    }
  });

  it("henter ikke hvis App'en forsvinder mens registreringen er i luften", async () => {
    await render();
    await act(async () => {
      root.render(<div />);
    });

    aabnPorten();
    await tik();

    expect(listCardsKald()).toBe(0);
  });
});
