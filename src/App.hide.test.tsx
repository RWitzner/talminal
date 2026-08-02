/**
 * @vitest-environment happy-dom
 */

// "Fjern fra listen" — ledningsfoeringen fra rail'ens knap til backendens
// afvisning og videre gennem bekraeftelsen.
//
// REGRESSIONEN filen lukker: backendens `set_workspace_hidden` fejler LUKKET paa
// et koerende workspace (`Err("kraever_bekraeftelse:<n>")`, workspaces/commands.rs)
// for at tvinge lukningen gennem den bekraeftede vej. App.tsx sendte fejlen
// videre som en intetsigende notits, og knappen var derfor doed for ethvert
// koerende workspace — en vej der VIRKEDE foer (destruktivt, men den virkede).
//
// SLUTREVIEW B1 udvider filen: bekraeftelsen svarer nu backenden med
// `confirmed: true` paa SAMME command, og backenden skriver `.hidden` selv.
// Foer rettelsen armerede App'en kun React-state (`hidePending`) og ventede paa
// at rail'ens liste meldte posten nede — en betingelse der for ens EGET
// workspace pr. konstruktion ALDRIG kan indtraeffe foer processen doer. Ejeren
// kunne altsaa trykke "Fjern", besvare en destruktiv bekraeftelse, miste sine
// agent-sessioner — og posten stod der stadig ved naeste opstart.
//
// Filen daekker: ét-kliks-vejen (uaendret), afvisningen -> dialogen med
// backendens tal, bekraeft -> ét kald der baade lukker og skjuler,
// annullér -> intet, det AKTIVE workspace (instance_alive er sand for en selv),
// og fallback naar praefikset ikke kan laeses.
//
// Stilen er husets: happy-dom-docblock, createRoot + act(), ingen
// @testing-library. Forlaeg: UsageHud.render.test.tsx og App.close.test.tsx.

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  minimize: vi.fn(async () => {}),
  toggleMaximize: vi.fn(async () => {}),
  closeWindow: vi.fn(async () => {}),
  setOcclusionReason: vi.fn(),
  /** slug -> fejl som et UBEKRAEFTET `set_workspace_hidden` skal afvise med. */
  hideRejects: new Map<string, unknown>(),
  /** slug -> fejl som det BEKRAEFTEDE kald skal afvise med (luk-og-skjul fejler). */
  confirmedRejects: new Map<string, unknown>(),
  /** Fejler `request_close_workspace`? */
  closeRejects: null as unknown,
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    minimize: mocks.minimize,
    toggleMaximize: mocks.toggleMaximize,
    close: mocks.closeWindow,
  }),
}));
vi.mock("./browser/occlusion", () => ({
  setOcclusionReason: mocks.setOcclusionReason,
}));

// Samme afskaerming som App.close.test.tsx: uden den starter voice- og
// canvas-lagene i happy-dom.
vi.mock("./Settings", () => ({ default: () => null, Settings: () => null }));
vi.mock("./CanvasSurface", () => ({ CanvasSurface: () => null }));
vi.mock("./voice/sound", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./voice/sound")>()),
  createSoundPlayer: () => ({ play: () => {}, close: async () => {} }),
}));
vi.mock("./voice/dispatch", () => ({
  createVoiceDispatcher: () => ({ dispatch: async () => ({ ok: true }) }),
}));
import App from "./App";
import type { WorkspaceSummary } from "./workspaces";

describe("App — Fjern fra listen gaar gennem bekraeftelsen", () => {
  let host: HTMLDivElement;
  let root: Root;

  function workspace(overrides: Partial<WorkspaceSummary>): WorkspaceSummary {
    return {
      slug: "a-1",
      name: "Alpha",
      path_hint: null,
      root: null,
      state: "running",
      cards: 2,
      running_cards: 2,
      attention: false,
      attention_kind: "none",
      is_active: false,
      hidden: false,
      defect: false,
      ...overrides,
    };
  }

  const ALPHA = workspace({});
  const BETA = workspace({
    slug: "b-2",
    name: "Beta",
    state: "stopped",
    cards: 0,
    running_cards: 0,
  });
  /** DETTE vindue. `instance_alive` er sand for en selv, saa backenden afviser
   *  ogsaa her — bekraeftelsen skal derfor ogsaa gaelde den aktive post. */
  const GAMMA = workspace({
    slug: "c-3",
    name: "Gamma",
    is_active: true,
    cards: 3,
    running_cards: 3,
  });
  const DELTA = workspace({ slug: "d-4", name: "Delta", hidden: true, state: "stopped" });

  const LISTE: WorkspaceSummary[] = [ALPHA, BETA, GAMMA, DELTA];

  beforeEach(() => {
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    mocks.listeners.clear();
    mocks.closeWindow.mockClear();
    mocks.setOcclusionReason.mockClear();
    mocks.hideRejects.clear();
    mocks.confirmedRejects.clear();
    mocks.closeRejects = null;
    mocks.listen.mockReset().mockImplementation(
      async (event: string, handler: (event: { payload: unknown }) => void) => {
        mocks.listeners.set(event, handler);
        return () => mocks.listeners.delete(event);
      },
    );
    mocks.invoke
      .mockReset()
      .mockImplementation(async (command: string, args?: Record<string, unknown>) => {
        if (command === "list_cards" || command === "get_workspace") {
          throw new Error("test: ingen workspace");
        }
        if (command === "list_workspaces") return LISTE;
        // Backendens gate: den afviser KUN det ubekraeftede kald. Med
        // `confirmed: true` lukker og skjuler den selv — det er hele B1-fixet,
        // og modellen her skal spejle det, ellers tester filen en backend der
        // ikke findes.
        if (command === "set_workspace_hidden" && args?.hidden === true) {
          const slug = String(args?.slug);
          if (args?.confirmed === true) {
            const afvisning = mocks.confirmedRejects.get(slug);
            if (afvisning !== undefined) throw afvisning;
          } else {
            const afvisning = mocks.hideRejects.get(slug);
            if (afvisning !== undefined) throw afvisning;
          }
        }
        if (command === "request_close_workspace" && mocks.closeRejects !== null) {
          throw mocks.closeRejects;
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

  const dialog = () =>
    document.querySelector<HTMLElement>("[data-close-dialog]");
  const confirm = () =>
    document.querySelector<HTMLButtonElement>("[data-close-confirm]")!;
  const cancel = () =>
    document.querySelector<HTMLButtonElement>("[data-close-cancel]")!;
  const hideButton = (slug: string) =>
    document.querySelector<HTMLButtonElement>(`[data-workspace-hide="${slug}"]`)!;

  function calls(command: string): unknown[][] {
    return mocks.invoke.mock.calls.filter((call) => call[0] === command);
  }

  function hideCalls(): unknown[][] {
    return calls("set_workspace_hidden");
  }

  /** Backendens `workspaces-changed`: rail'ens liste er backendens sandhed, og
   *  det er DEN der fortaeller at et workspace er nede. */
  function emitListe(liste: WorkspaceSummary[]): Promise<void> {
    return act(async () => {
      mocks.listeners.get("workspaces-changed")?.({ payload: liste });
    });
  }

  function click(button: HTMLButtonElement): Promise<void> {
    return act(async () => button.click());
  }

  function dotState(): string | null | undefined {
    return document.querySelector("[data-hud-dot]")?.getAttribute("data-state");
  }

  it("et workspace UDEN koerende kort skjules stadig i ét klik", async () => {
    await render();
    await click(hideButton("b-2"));

    expect(dialog()).toBeNull();
    expect(hideCalls()).toEqual([
      ["set_workspace_hidden", { slug: "b-2", hidden: true, confirmed: false }],
    ]);
    expect(calls("request_close_workspace")).toHaveLength(0);
    expect(dotState()).not.toBe("error");
  });

  it("Hent frem gaar direkte igennem — der er intet at bekraefte", async () => {
    await render();
    // Skjulte poster tegnes foerst naar "Vis skjulte" er slaaet til.
    await click(
      document.querySelector<HTMLButtonElement>("[data-workspace-show-hidden]")!,
    );
    await click(hideButton("d-4"));

    expect(dialog()).toBeNull();
    expect(hideCalls()).toEqual([
      ["set_workspace_hidden", { slug: "d-4", hidden: false, confirmed: false }],
    ]);
  });

  // Tallet kommer fra PRAEFIKSET, ikke fra rail'ens liste: listen siger 2, mens
  // backenden — der taeller paa skrivetidspunktet — siger 5.
  it("afvisningen aabner bekraeftelsen med backendens tal, ikke listens", async () => {
    mocks.hideRejects.set("a-1", "kraever_bekraeftelse:5");
    await render();
    await click(hideButton("a-1"));

    expect(dialog()!.textContent).toContain("Alpha");
    expect(dialog()!.textContent).toContain("5 kort");
    expect(dialog()!.textContent).not.toContain("2 kort");
    // Ingen intetsigende notits — spoergsmaalet ER svaret.
    expect(dotState()).not.toBe("error");
    // Og der er endnu ikke rykket noget.
    expect(calls("request_close_workspace")).toHaveLength(0);
    expect(hideCalls()).toHaveLength(1);
  });

  // SLUTREVIEW B1. Bekraeftelsen gaar TILBAGE til backenden paa samme command
  // med `confirmed: true`. Backenden er den ENESTE der kan skrive `.hidden` paa
  // en maade der overlever processens doed: den skriver sidecaren synkront i
  // selve kaldet, foer lukningen naar at blive udfoert.
  it("bekraeftelsen sender skjulningen tilbage til BACKENDEN, ikke til React-state", async () => {
    mocks.hideRejects.set("a-1", "kraever_bekraeftelse:2");
    await render();
    await click(hideButton("a-1"));
    await click(confirm());

    expect(dialog()).toBeNull();
    expect(hideCalls()).toEqual([
      ["set_workspace_hidden", { slug: "a-1", hidden: true, confirmed: false }],
      ["set_workspace_hidden", { slug: "a-1", hidden: true, confirmed: true }],
    ]);
    // Ikke en raa `request_close_workspace`: den lukker uden at skjule, og
    // skjulningen ville saa skulle overleve i frontend-state.
    expect(calls("request_close_workspace")).toHaveLength(0);
    expect(dotState()).not.toBe("error");
  });

  // Den STRUKTURELLE regression: handlingen maa ikke afhaenge af at rail'ens
  // liste senere melder posten nede. Foer fixet skete skjulningen foerst naar
  // `state === "stopped"` landede — her sker der derfor INTET nyt naar listen
  // kommer, fordi arbejdet allerede er gjort.
  it("skjulningen afventer ikke et senere listeevent", async () => {
    mocks.hideRejects.set("a-1", "kraever_bekraeftelse:2");
    await render();
    await click(hideButton("a-1"));
    await click(confirm());
    expect(hideCalls()).toHaveLength(2);

    // Processen lever stadig et par tick …
    await emitListe(LISTE);
    expect(hideCalls()).toHaveLength(2);
    // … og da den endelig melder sig nede, er der intet tilbage at gøre.
    await emitListe([{ ...ALPHA, state: "stopped" }, BETA, GAMMA, DELTA]);
    expect(hideCalls()).toHaveLength(2);
  });

  it("annullering skjuler intet og lukker intet — heller ikke bagefter", async () => {
    mocks.hideRejects.set("a-1", "kraever_bekraeftelse:2");
    await render();
    await click(hideButton("a-1"));
    await click(cancel());

    expect(dialog()).toBeNull();
    expect(calls("request_close_workspace")).toHaveLength(0);
    expect(hideCalls()).toHaveLength(1);

    // Lukker workspacet af en HELT anden grund bagefter, maa den annullerede
    // skjulning ikke vaagne op igen.
    mocks.hideRejects.clear();
    await emitListe([{ ...ALPHA, state: "stopped" }, BETA, GAMMA, DELTA]);
    expect(hideCalls()).toHaveLength(1);
  });

  // SLUTREVIEW B1 — DEN VIGTIGSTE CASE. `instance_alive` er sand for en selv,
  // saa backenden afviser ogsaa naar brugeren fjerner det workspace han SELV
  // sidder i. Og netop dér kan en armeret frontend-skjulning aldrig udfoeres:
  // posten kan pr. konstruktion ikke melde sig `stopped` foer processen er doed,
  // og saa er React-staten vaek med den. Bekraeftelsen skal derfor tilbage til
  // backenden, som skriver `.hidden` FOER lukningen kan naa at gennemfoeres.
  it("det AKTIVE workspace skjules af backenden i selve bekraeftelses-kaldet", async () => {
    mocks.hideRejects.set("c-3", "kraever_bekraeftelse:3");
    await render();
    await click(hideButton("c-3"));

    expect(dialog()!.textContent).toContain("Gamma");
    expect(dialog()!.textContent).toContain("3 kort");

    await click(confirm());
    expect(hideCalls()).toEqual([
      ["set_workspace_hidden", { slug: "c-3", hidden: true, confirmed: false }],
      ["set_workspace_hidden", { slug: "c-3", hidden: true, confirmed: true }],
    ]);
    // Ikke vinduets egen luk-vej: den ville stille samme spoergsmaal én gang til
    // gennem CloseRequested-funnelen (T11, fund 4).
    expect(mocks.closeWindow).not.toHaveBeenCalled();
    // Og posten forsvinder ikke tavst uden at nogen har skrevet noget: der er
    // ingen armeret ventetilstand tilbage, saa et senere listeevent maa ikke
    // udloese en ny runde.
    await emitListe([ALPHA, BETA, { ...GAMMA, state: "stopped" }, DELTA]);
    expect(hideCalls()).toHaveLength(2);
  });

  it("fejler backendens luk-og-skjul, faar brugeren en notits", async () => {
    mocks.hideRejects.set("a-1", "kraever_bekraeftelse:2");
    mocks.confirmedRejects.set("a-1", new Error("adgang naegtet (os error 5)"));
    await render();
    await click(hideButton("a-1"));
    await click(confirm());

    expect(hideCalls()).toHaveLength(2);
    expect(dotState()).toBe("error");
    await click(document.querySelector<HTMLButtonElement>("button[data-hud-chip]")!);
    expect(document.body.textContent).toContain("Kunne ikke opdatere listen");
    expect(document.body.textContent).toContain("adgang naegtet (os error 5)");
  });

  // Aendrer fejlformatet sig en dag, skal vejen falde tilbage til den
  // eksisterende notits — ikke kaste, og ikke gaette paa et tal.
  it("en fejl der ikke er en bekraeftelses-anmodning giver notitsen, ikke en dialog", async () => {
    mocks.hideRejects.set("a-1", "adgang naegtet (os error 5)");
    await render();
    await click(hideButton("a-1"));

    expect(dialog()).toBeNull();
    expect(calls("request_close_workspace")).toHaveLength(0);
    expect(dotState()).toBe("error");

    // Notitsen baerer stadig backendens ord — HUD'ens panel foldes ud for at se den.
    await click(document.querySelector<HTMLButtonElement>("button[data-hud-chip]")!);
    expect(document.body.textContent).toContain("Kunne ikke opdatere listen");
    expect(document.body.textContent).toContain("adgang naegtet (os error 5)");
  });

  it("et uforstaaeligt praefiks faelder ikke appen", async () => {
    mocks.hideRejects.set("a-1", "kraever_bekraeftelse:mange");
    await render();
    await click(hideButton("a-1"));

    expect(dialog()).toBeNull();
    expect(dotState()).toBe("error");
    expect(document.querySelector("[data-workspace-rail]")).not.toBeNull();
  });
});
