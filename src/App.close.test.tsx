/**
 * @vitest-environment happy-dom
 */

// App'ens LEDNINGSFOERING omkring luk-bekraeftelsen (Task 11, frontend).
//
// Udskilt fra CloseWorkspaceDialog.test.tsx i fix-runde 1 (fund 3): dialogens
// egen kontrakt og appens ledningsfoering er to ting, og saa laenge de laa i
// samme fil var der ingen test der overhovedet asserterede paa fokus i den vej
// appen faktisk koerer.
//
// Filen daekker de fire veje ind i dialogen og de fire ud af den:
//   ind : `close-confirm-requested` (Alt+F4/taskbar), rail-✕ paa et andet
//         workspace med koerende kort, rail-✕ uden koerende kort, rail-✕ paa
//         det AKTIVE workspace (fund 4).
//   ud  : `confirm_close(true|false)`, `request_close_workspace`, intet.
// Plus kollisionen mellem de to kilder (fund 2) og occlusion-gaten (minor 5).
//
// Backend-halvdelen findes ikke endnu; Tauri afviser derfor begge commands, og
// det SKAL degradere paent — sidste test pinner det.

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
  dialogProps: [] as { restoreFocusTo: HTMLElement | null }[],
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

// Occlusion-gaten mockes saa vi kan assertere PAA kaldene — samme moenster som
// App.settings.test.tsx' "final-review Finding 1"-daekning (spec §4a).
vi.mock("./browser/occlusion", () => ({
  setOcclusionReason: mocks.setOcclusionReason,
}));

// Den ÆGTE dialog renderes (DOM-asserts nedenfor er reelle), men vi noterer
// props undervejs. Det er den eneste maade at pinne `restoreFocusTo`-vejen paa
// i happy-dom: Chromium blurrer en knap der bliver `disabled`, happy-dom goer
// ikke, saa en ren fokus-assert kan ikke skelne appens vej fra dialogens
// fallback (fund 3).
vi.mock("./CloseWorkspaceDialog", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("./CloseWorkspaceDialog")>();
  return {
    ...actual,
    CloseWorkspaceDialog: (
      props: React.ComponentProps<typeof actual.CloseWorkspaceDialog>,
    ) => {
      mocks.dialogProps.push({ restoreFocusTo: props.restoreFocusTo });
      return <actual.CloseWorkspaceDialog {...props} />;
    },
  };
});

// Samme afskaerming som App.cardsChanged.test.tsx: uden den starter voice- og
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

describe("App — luk-bekraeftelsens ledningsfoering", () => {
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

  const LISTE: WorkspaceSummary[] = [
    workspace({}),
    workspace({
      slug: "b-2",
      name: "Beta",
      cards: 0,
      running_cards: 0,
      state: "stopped",
    }),
    // DETTE vindue. Rail'en tegner ✕ paa alle rader, ogsaa den aktive.
    workspace({ slug: "c-3", name: "Gamma", is_active: true, running_cards: 3 }),
  ];

  beforeEach(() => {
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    mocks.listeners.clear();
    mocks.closeWindow.mockClear();
    mocks.setOcclusionReason.mockClear();
    mocks.dialogProps.length = 0;
    mocks.listen.mockReset().mockImplementation(
      async (event: string, handler: (event: { payload: unknown }) => void) => {
        mocks.listeners.set(event, handler);
        return () => mocks.listeners.delete(event);
      },
    );
    mocks.invoke
      .mockReset()
      .mockImplementation(
        async (command: string, args?: Record<string, unknown>) => {
          if (command === "list_cards" || command === "get_workspace") {
            throw new Error("test: ingen workspace");
          }
          if (command === "list_workspaces") return LISTE;
          // BACKENDENS gate, spejlet: `request_close_workspace` afviser med
          // `kraever_bekraeftelse:<n>` naar posten har koerende sessioner og
          // brugeren ikke har svaret endnu (workspaces/commands.rs::
          // bekraeftelse_mangler). Frontenden afgoer det IKKE selv laengere —
          // rail'ens liste er op til ét sekund gammel, og et kort der startede i
          // det sekund ville ellers blive lukket uden et spoergsmaal.
          if (command === "request_close_workspace" && args?.confirmed !== true) {
            const post = LISTE.find((w) => w.slug === args?.slug);
            if (post !== undefined && post.running_cards > 0) {
              throw new Error(`kraever_bekraeftelse:${post.running_cards}`);
            }
          }
          return undefined;
        },
      );
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
  const titlebarClose = () =>
    document.querySelector<HTMLButtonElement>(
      'button[aria-label="Afslut Talminal"]',
    )!;
  const railClose = (slug: string) =>
    document.querySelector<HTMLButtonElement>(
      `[data-workspace-close="${slug}"]`,
    )!;

  function calls(command: string): unknown[][] {
    return mocks.invoke.mock.calls.filter((call) => call[0] === command);
  }

  function occlusionCalls(): unknown[][] {
    return mocks.setOcclusionReason.mock.calls.filter(
      (call) => call[0] === "close-dialog",
    );
  }

  async function emitCloseConfirm(
    running: number,
    workspaces = 2,
  ): Promise<void> {
    await act(async () => {
      mocks.listeners.get("close-confirm-requested")?.({
        payload: { workspaces, running_cards: running },
      });
    });
  }

  it("lytter paa close-confirm-requested ved mount", async () => {
    await render();
    expect(mocks.listeners.has("close-confirm-requested")).toBe(true);
  });

  it("eventet aabner dialogen med backendens tal og deaktiverer titelbjaelkens ✕", async () => {
    await render();
    expect(dialog()).toBeNull();
    await emitCloseConfirm(4);
    expect(dialog()!.textContent).toContain("Afslut Talminal?");
    expect(dialog()!.textContent).toContain("2 åbne workspaces");
    expect(dialog()!.textContent).toContain("4 kørende kort");
    expect(titlebarClose().disabled).toBe(true);
  });

  it("Annullér svarer confirm_close(false) og lukker dialogen", async () => {
    await render();
    await emitCloseConfirm(2);
    await act(async () => cancel().click());
    expect(calls("confirm_close")).toEqual([["confirm_close", { ok: false }]]);
    expect(dialog()).toBeNull();
    expect(titlebarClose().disabled).toBe(false);
  });

  it("Bekraeft svarer confirm_close(true)", async () => {
    await render();
    await emitCloseConfirm(2);
    await act(async () => confirm().click());
    expect(calls("confirm_close")).toEqual([["confirm_close", { ok: true }]]);
    expect(dialog()).toBeNull();
  });

  it("rail-✕ paa et workspace med koerende kort spoerger FOERST, lukker bagefter", async () => {
    await render();
    await act(async () => railClose("a-1").click());

    // Foerste kald er PROBEN — den spoerger backenden om der er noget at
    // bekraefte og roerer ingenting. Der er ikke lukket noget endnu.
    expect(calls("request_close_workspace")).toEqual([
      ["request_close_workspace", { slug: "a-1", confirmed: false }],
    ]);
    expect(dialog()!.textContent).toContain("Alpha");
    // Tallet kommer fra backendens afvisning, ikke fra rail'ens liste.
    expect(dialog()!.textContent).toContain("2 kort");

    await act(async () => confirm().click());
    expect(calls("request_close_workspace").at(-1)).toEqual([
      "request_close_workspace",
      { slug: "a-1", confirmed: true },
    ]);
    // Det er et ANDET vindue: dets confirm_close er ikke vores at svare paa.
    expect(calls("confirm_close")).toHaveLength(0);
  });

  it("rail-✕ uden koerende kort spoerger ikke", async () => {
    await render();
    await act(async () => railClose("b-2").click());
    expect(dialog()).toBeNull();
    expect(calls("request_close_workspace")).toEqual([
      ["request_close_workspace", { slug: "b-2", confirmed: false }],
    ]);
  });

  // P1-1 fra Codex-reviewet, som REGRESSION: rail'ens liste siger 0 koerende
  // kort for b-2 — men listen kommer fra `workspaces-changed` paa
  // badge-kadencen (1 s), og et kort kan vaere startet siden. Traf frontenden
  // selv beslutningen (som foer), blev workspacet lukket med en levende
  // agent-session uden at nogen spurgte. Nu er det backendens svar der
  // afgoer det, og dialogen viser BACKENDENS tal.
  it("et kort der startede efter sidste liste-opdatering giver stadig en dialog", async () => {
    mocks.invoke.mockImplementation(
      async (command: string, args?: Record<string, unknown>) => {
        if (command === "list_cards" || command === "get_workspace") {
          throw new Error("test: ingen workspace");
        }
        if (command === "list_workspaces") return LISTE;
        if (command === "request_close_workspace" && args?.confirmed !== true) {
          throw new Error("kraever_bekraeftelse:1");
        }
        return undefined;
      },
    );
    await render();
    await act(async () => railClose("b-2").click());

    expect(dialog()!.textContent).toContain("Beta");
    expect(dialog()!.textContent).toContain("1 kort");
    expect(
      calls("request_close_workspace").filter(
        (call) => (call[1] as { confirmed: boolean }).confirmed,
      ),
    ).toHaveLength(0);
  });

  it("Annullér paa et andet workspace udfoerer INGENTING", async () => {
    await render();
    await act(async () => railClose("a-1").click());
    await act(async () => cancel().click());
    // Proben er afvist og har ikke roert noget; der maa ikke komme et
    // bekraeftet kald efter et "nej".
    expect(
      calls("request_close_workspace").filter(
        (call) => (call[1] as { confirmed: boolean }).confirmed,
      ),
    ).toHaveLength(0);
    expect(calls("confirm_close")).toHaveLength(0);
    expect(dialog()).toBeNull();
  });

  // FUND 4 (rev 2, efter Codex-reviewets P1-1): vejen er nu ÉN for alle rader.
  //
  // Foer greb frontenden fat i `is_active` og kaldte `getCurrentWindow().close()`
  // for "mit eget" workspace. Den markering kommer fra `workspaces-changed` paa
  // badge-kadencen og staar i op til ét sekund efter et skift paa den FORRIGE
  // post — saa ✕ paa den post lukkede det vindue brugeren sad i. Identitet maa
  // ikke udledes af en tilstand der halter. Alle rader gaar nu gennem
  // control-kanalen, og der spoerges stadig kun én gang: modtagerens funnel
  // springer sin egen bekraeftelse over (`begin_peer_close`).
  it("rail-✕ paa den aktive post lukker ikke vinduet bag om backenden", async () => {
    await render();
    await act(async () => railClose("c-3").click());

    expect(mocks.closeWindow).toHaveBeenCalledTimes(0);
    // Samme to-trins-form som alle andre rader — og spoergsmaalet er rail'ens,
    // ikke funnelens.
    expect(calls("request_close_workspace")).toEqual([
      ["request_close_workspace", { slug: "c-3", confirmed: false }],
    ]);
    expect(dialog()!.textContent).toContain("Gamma");
    expect(dialog()!.textContent).toContain("3 kort");

    await act(async () => confirm().click());
    expect(calls("request_close_workspace").at(-1)).toEqual([
      "request_close_workspace",
      { slug: "c-3", confirmed: true },
    ]);
  });

  // FUND 2: to kilder, ét felt. Eventet overskrev en aaben rail-dialog, og fordi
  // samme indre dialog stod paa samme plads reconcilede React det som en
  // PROP-aendring: fare-knappen skiftede betydning under en markoer der allerede
  // var paa vej ned. Nu koees anmodningen bag den aabne.
  it("et close-confirm-requested muterer IKKE en aaben rail-dialog — det koees", async () => {
    await render();
    await act(async () => railClose("a-1").click());
    const fareknapFoer = confirm();
    expect(dialog()!.textContent).toContain("Luk Alpha?");

    await emitCloseConfirm(4);

    // Uaendret spoergsmaal, uaendret knap — samme DOM-node, samme betydning.
    expect(dialog()!.textContent).toContain("Luk Alpha?");
    expect(dialog()!.textContent).not.toContain("Afslut Talminal?");
    expect(confirm()).toBe(fareknapFoer);

    // Brugerens klik gaar til det han faktisk saa …
    await act(async () => confirm().click());
    expect(calls("request_close_workspace").at(-1)).toEqual([
      "request_close_workspace",
      { slug: "a-1", confirmed: true },
    ]);

    // … og den koeede anmodning falder IKKE tavst paa gulvet: den bliver til en
    // NY dialog (ny node, fokus taget forfra) som funnelen faar svar paa.
    expect(dialog()!.textContent).toContain("Afslut Talminal?");
    expect(dialog()!.textContent).toContain("4 kørende kort");
    expect(confirm()).not.toBe(fareknapFoer);
    expect(document.activeElement).toBe(cancel());

    await act(async () => cancel().click());
    expect(calls("confirm_close")).toEqual([["confirm_close", { ok: false }]]);
    expect(dialog()).toBeNull();
  });

  it("funnelen spoerger kun én gang: to events giver én dialog og ét svar", async () => {
    await render();
    await emitCloseConfirm(2);
    await emitCloseConfirm(2);
    await act(async () => cancel().click());
    expect(calls("confirm_close")).toEqual([["confirm_close", { ok: false }]]);
    expect(dialog()).toBeNull();
  });

  // FUND 3: den vej appen faktisk koerer. App'en fanger fokus i LYTTEREN — foer
  // React har rendret — fordi titelbjaelkens ✕ bliver `disabled` i samme commit
  // som dialogen monteres. Fjernes proppen fra App.tsx, er det baade en
  // typefejl og en roed test her.
  it("App sender det element der havde fokus da anmodningen kom", async () => {
    await render();
    titlebarClose().focus();
    const lukKnap = titlebarClose();
    await emitCloseConfirm(1);

    const sidste = mocks.dialogProps.at(-1)!;
    expect(sidste.restoreFocusTo).toBe(lukKnap);
    expect(sidste.restoreFocusTo).not.toBeNull();
  });

  it("rail-vejen: fokus er tilbage paa rail-✕ naar dialogen er lukket", async () => {
    await render();
    railClose("a-1").focus();
    await act(async () => railClose("a-1").click());
    expect(mocks.dialogProps.at(-1)!.restoreFocusTo).toBe(railClose("a-1"));
    expect(document.activeElement).toBe(cancel());

    await act(async () => cancel().click());
    expect(document.activeElement).toBe(railClose("a-1"));
  });

  // MINOR 5: browser-kortenes WebView2-boern maler OVER DOM'et. Uden gaten
  // ville et aabent browser-kort daekke bekraeftelsen — praecis den defekt
  // review-gaten fangede for settings-panelet (App.settings.test.tsx).
  it("registrerer occlusion mens dialogen staar aaben, og rydder den bagefter", async () => {
    await render();
    expect(occlusionCalls()).toEqual([["close-dialog", false]]);

    await emitCloseConfirm(2);
    expect(occlusionCalls().at(-1)).toEqual(["close-dialog", true]);

    await act(async () => cancel().click());
    expect(occlusionCalls().at(-1)).toEqual(["close-dialog", false]);
  });

  it("backenden findes ikke endnu: en afvist confirm_close faelder ikke appen", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_cards" || command === "get_workspace") {
        throw new Error("test: ingen workspace");
      }
      if (command === "list_workspaces") return LISTE;
      if (command === "confirm_close" || command === "request_close_workspace") {
        throw new Error("unknown command");
      }
      return undefined;
    });
    await render();
    await emitCloseConfirm(1);
    await act(async () => confirm().click());
    expect(calls("confirm_close")).toHaveLength(1);
    expect(dialog()).toBeNull();
    // Fladen staar endnu — rail'en er stadig monteret.
    expect(document.querySelector("[data-workspace-rail]")).not.toBeNull();
  });
});
