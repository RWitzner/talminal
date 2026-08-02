import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CanvasController } from "../CanvasSurface";
import { subscribeCast, type CastEvent } from "./cast";
import {
  createVoiceDispatcher,
  type DispatchHudEvent,
  type TauriInvoke,
} from "./dispatch";

const cards = [
  {
    number: 1,
    name: "card-1",
    cwd: "C:\\projekter\\demo",
    profile: "claude",
    running: true,
    exited: null,
    restore_action: null,
  },
  {
    number: 2,
    name: "card-2",
    cwd: "C:\\projekter\\webshop",
    profile: "claude",
    running: true,
    exited: null,
    restore_action: null,
  },
  {
    number: 3,
    name: "card-3",
    cwd: "C:\\projekter\\brain-site",
    profile: "claude",
    running: true,
    exited: null,
    restore_action: null,
  },
  {
    number: 5,
    name: "card-5",
    cwd: "C:\\projekter\\demo\\canvas",
    profile: "claude",
    running: true,
    exited: null,
    restore_action: null,
  },
];

const browserCardFixture = {
  kind: "browser" as const,
  number: 6,
  name: "card-6",
  cwd: "",
  profile: "",
  running: true,
  exited: null,
  restore_action: null,
  opened_by: null,
  url: "https://github.com",
  title: "GitHub",
};

describe("createVoiceDispatcher", () => {
  let invoke: ReturnType<typeof vi.fn<TauriInvoke>>;
  let canvas: CanvasController;
  let hudEvents: DispatchHudEvent[];

  beforeEach(() => {
    invoke = vi.fn<TauriInvoke>(async (command, args) => {
      if (command === "list_cards") return cards;
      if (command === "create_card") {
        return {
          number: 6,
          name: "card-6",
          cwd: "C:\\projekter\\demo",
          profile: "claude",
          running: false,
          exited: null,
          restore_action: null,
        };
      }
      if (command === "close_cards") {
        return {
          closed: Array.isArray(args?.names) ? args.names : [],
          errors: [],
          sequence_reset: false,
        };
      }
      return undefined;
    });
    canvas = {
      focusCard: vi.fn(),
      prepareFreshSpawn: vi.fn(),
      getFocusedCard: vi.fn(() => 2),
    };
    hudEvents = [];
  });

  function dispatcher() {
    return createVoiceDispatcher({
      invoke,
      canvas,
      getProject: async () => ({
        root: "C:\\projekter\\demo",
        name: "demo",
      }),
      onHudEvent: (event) => hudEvents.push(event),
    });
  }

  // Reviewfund (T7 review): objectContaining({ profile: anything() }) kan
  // ALDRIG skelne "noeglen mangler" fra "noeglen er til stede med vaerdien
  // undefined" — Anything.asymmetricMatch returnerer false for undefined
  // uanset, saa .not.objectContaining(...) bestaar i begge tilfaelde. Det er
  // netop den regression (ubetinget spread af en undefined agent) dette
  // choke-punkt skal fange, saa vi inspicerer det faktiske mock-kald og
  // bruger toHaveProperty, der skelner reelt (samme idiom som
  // pipeline.test.ts' agent-telemetri-tests).
  function createCardArgs(): Record<string, unknown> | undefined {
    return invoke.mock.calls.find(([command]) => command === "create_card")
      ?.[1] as Record<string, unknown> | undefined;
  }

  function listCardsWithBrowser() {
    invoke.mockImplementation(async (command) => {
      if (command === "list_cards") return [...cards, browserCardFixture];
      return undefined;
    });
  }

  it("sender prompt til det fokuserede kort med Rust-side submit-koreografi", async () => {
    await dispatcher().dispatch({
      kind: "send_prompt",
      card: null,
      text: "Kør testene",
    });

    expect(invoke).toHaveBeenNthCalledWith(1, "list_cards");
    expect(invoke).toHaveBeenNthCalledWith(2, "submit_prompt", {
      name: "card-2",
      text: "Kør testene",
    });
  });

  it("sender prompt til et eksplicit kort frem for fokus", async () => {
    await dispatcher().dispatch({
      kind: "send_prompt",
      card: 3,
      text: "Skriv README",
    });

    expect(invoke).toHaveBeenLastCalledWith("submit_prompt", {
      name: "card-3",
      text: "Skriv README",
    });
  });

  it("send_prompt mod browser-kort afvises med browser_card uden invoke", async () => {
    // browser-fixturen (kind: "browser") er kort nr. 6 i listCards-mocken
    listCardsWithBrowser();
    const result = await dispatcher().dispatch({
      kind: "send_prompt",
      card: 6,
      text: "Kør testene",
    });
    expect(result).toMatchObject({ ok: false, code: "browser_card" });
    expect(invoke).not.toHaveBeenCalledWith("submit_prompt", expect.anything());
    expect(hudEvents.at(-1)).toMatchObject({
      kind: "error",
      message: "Kort 6 er en browser",
    });
  });

  it("restart_card mod browser-kort afvises med browser_card uden invoke", async () => {
    listCardsWithBrowser();
    const result = await dispatcher().dispatch({ kind: "restart_card", card: 6 });
    expect(result).toMatchObject({ ok: false, code: "browser_card" });
    expect(invoke).not.toHaveBeenCalledWith("kill_card", expect.anything());
    expect(invoke).not.toHaveBeenCalledWith("respawn_card", expect.anything());
    expect(invoke).not.toHaveBeenCalledWith("spawn_card", expect.anything());
    expect(canvas.prepareFreshSpawn).not.toHaveBeenCalled();
  });

  it("new_card opretter altid i projekt-roden", async () => {
    await dispatcher().dispatch({ kind: "new_card", count: 2 });
    expect(invoke).toHaveBeenCalledWith("create_card", {
      cwd: "C:\\projekter\\demo", // getProject-mockens root
      command: null,
    });
  });

  it("restart bruger frisk spawn i rækkefølgen list, kill, spawn, refresh", async () => {
    const order: string[] = [];
    canvas.prepareFreshSpawn = vi.fn(() => order.push("prepare"));
    invoke.mockImplementation(async (command) => {
      order.push(command);
      if (command === "list_cards") return cards;
      return undefined;
    });
    const onWorkspaceMutation = vi.fn(async () => {
      order.push("refresh");
    });
    const result = await createVoiceDispatcher({
      invoke,
      canvas,
      getProject: async () => ({
        root: "C:\\projekter\\demo",
        name: "demo",
      }),
      onHudEvent: (event) => hudEvents.push(event),
      onWorkspaceMutation,
    }).dispatch({ kind: "restart_card", card: 1 });

    expect(invoke.mock.calls).toEqual([
      ["list_cards"],
      ["kill_card", { name: "card-1" }],
      ["spawn_card", { name: "card-1" }],
    ]);
    expect(invoke).not.toHaveBeenCalledWith("respawn_card", expect.anything());
    expect(onWorkspaceMutation).toHaveBeenCalledTimes(1);
    expect(order).toEqual([
      "list_cards",
      "kill_card",
      "prepare",
      "spawn_card",
      "refresh",
    ]);
    expect(canvas.prepareFreshSpawn).toHaveBeenCalledWith(1);
    expect(result).toMatchObject({ ok: true, kind: "restart_card", card: 1 });
  });

  it("restart af et stoppet kort springer kill over og starter frisk", async () => {
    const stopped = { ...cards[0], running: false, exited: 0 };
    const order: string[] = [];
    canvas.prepareFreshSpawn = vi.fn(() => order.push("prepare"));
    invoke.mockImplementation(async (command) => {
      order.push(command);
      if (command === "list_cards") return [stopped];
      return undefined;
    });
    const onWorkspaceMutation = vi.fn(async () => {
      order.push("refresh");
    });
    const result = await createVoiceDispatcher({
      invoke,
      canvas,
      getProject: async () => ({
        root: "C:\\projekter\\demo",
        name: "demo",
      }),
      onHudEvent: (event) => hudEvents.push(event),
      onWorkspaceMutation,
    }).dispatch({ kind: "restart_card", card: 1 });

    expect(invoke.mock.calls).toEqual([
      ["list_cards"],
      ["spawn_card", { name: "card-1" }],
    ]);
    expect(order).toEqual(["list_cards", "prepare", "spawn_card", "refresh"]);
    expect(result).toMatchObject({ ok: true, kind: "restart_card", card: 1 });
  });

  it("refresher også hvis den friske spawn fejler", async () => {
    const onWorkspaceMutation = vi.fn(async () => undefined);
    invoke.mockImplementation(async (command) => {
      if (command === "list_cards") return cards;
      if (command === "spawn_card") throw new Error("spawn fejlede");
      return undefined;
    });

    await expect(
      createVoiceDispatcher({
        invoke,
        canvas,
        getProject: async () => ({
          root: "C:\\projekter\\demo",
          name: "demo",
        }),
        onHudEvent: (event) => hudEvents.push(event),
        onWorkspaceMutation,
      }).dispatch({ kind: "restart_card", card: 1 }),
    ).rejects.toThrow("spawn fejlede");
    expect(onWorkspaceMutation).toHaveBeenCalledTimes(1);
  });

  it("create_med_count_1_bruger_projekt_rod", async () => {
    await dispatcher().dispatch({ kind: "new_card", count: 1 });
    expect(invoke).toHaveBeenCalledWith("create_card", {
      cwd: "C:\\projekter\\demo",
      command: null,
    });
  });

  it("opretter count kort sekventielt og lader CanvasSurface eje det responsive layout", async () => {
    const created = [6, 7, 8].map((number) => ({
      number,
      name: `card-${number}`,
      cwd: "C:\\projekter\\demo",
      profile: "claude",
      running: false,
      exited: null,
      restore_action: null,
    }));
    let createIndex = 0;
    invoke.mockImplementation(async (command) => {
      if (command === "list_cards") return cards;
      if (command === "create_card") return created[createIndex++];
      return undefined;
    });

    const result = await dispatcher().dispatch({ kind: "new_card", count: 3 });

    expect(invoke.mock.calls.filter(([command]) => command === "create_card")).toHaveLength(3);
    expect(
      invoke.mock.calls.filter(([command]) => command === "update_card_geometry"),
    ).toHaveLength(0);
    expect(result).toMatchObject({
      ok: true,
      kind: "new_card",
      cards: [6, 7, 8],
    });
  });

  it("new_card sender ikke profile med uden eksplicit agent — choke-pointet (default_agent) afgør", async () => {
    await dispatcher().dispatch({ kind: "new_card", count: 1 });
    expect(createCardArgs()).not.toHaveProperty("profile");
  });

  it("new_card sender eksplicit agent som profile", async () => {
    await dispatcher().dispatch({ kind: "new_card", count: 1, agent: "codex" });
    expect(invoke).toHaveBeenCalledWith(
      "create_card",
      expect.objectContaining({ profile: "codex" }),
    );
  });

  it("HUD-beskeden navngiver agenten ved eksplicit valg", async () => {
    await dispatcher().dispatch({ kind: "new_card", count: 2, agent: "codex" });
    expect(
      hudEvents.some((event) => event.message.includes("2 codex-kort oprettet")),
    ).toBe(true);
  });

  it("behandler en ukendt agent-streng som fravaer + advarsel (spec N2)", async () => {
    await dispatcher().dispatch({
      kind: "new_card",
      count: 1,
      agent: "cursor" as never,
    });
    expect(createCardArgs()).not.toHaveProperty("profile");
    expect(hudEvents.some((event) => event.message.includes("ukendt agent"))).toBe(
      true,
    );
  });

  it("refresher gridet efter en delvist fejlet batch-oprettelse", async () => {
    const onWorkspaceMutation = vi.fn();
    let createCount = 0;
    invoke.mockImplementation(async (command) => {
      if (command === "list_cards") return cards;
      if (command === "create_card") {
        createCount += 1;
        if (createCount === 2) throw new Error("create fejlede");
        return {
          number: 6,
          name: "card-6",
          cwd: "C:\\projekter\\demo",
          profile: "claude",
          running: false,
          exited: null,
          restore_action: null,
        };
      }
      return undefined;
    });

    await expect(
      createVoiceDispatcher({
        invoke,
        canvas,
        getProject: async () => ({
          root: "C:\\projekter\\demo",
          name: "demo",
        }),
        onHudEvent: (event) => hudEvents.push(event),
        onWorkspaceMutation,
      }).dispatch({ kind: "new_card", count: 3 }),
    ).rejects.toThrow("create fejlede");
    expect(onWorkspaceMutation).toHaveBeenCalledTimes(1);
  });

  it("close_cards_udfoeres_straks_uden_confirmation", async () => {
    const result = await dispatcher().dispatch({ kind: "close_cards", cards: [3, 1] });
    expect(invoke).toHaveBeenCalledWith("close_cards", {
      names: ["card-3", "card-1"],
    });
    expect(result).toMatchObject({ ok: true, kind: "close_cards", cards: [3, 1] });
    expect(hudEvents).toEqual([
      { kind: "info", message: "Lukker 2 kort…" },
      { kind: "info", message: "Kort 3, 1 blev lukket" },
    ]);
  });

  it("refresher gridet efter en delvist fejlet multi-close", async () => {
    const onWorkspaceMutation = vi.fn(async () => undefined);
    const onCardsClosing = vi.fn();
    invoke.mockImplementation(async (command, args) => {
      if (command === "list_cards") return cards;
      if (command === "close_cards") {
        return {
          closed: args?.names ?? [],
          errors: [{ name: "card-1", message: "teardown fejlede" }],
          sequence_reset: false,
        };
      }
      return undefined;
    });

    await expect(
      createVoiceDispatcher({
        invoke,
        canvas,
        getProject: async () => ({
          root: "C:\\projekter\\demo",
          name: "demo",
        }),
        onHudEvent: (event) => hudEvents.push(event),
        onCardsClosing,
        onWorkspaceMutation,
      }).dispatch({ kind: "close_cards", cards: [3, 1] }),
    ).rejects.toThrow("card-1: teardown fejlede");
    expect(onCardsClosing).toHaveBeenCalledWith([3, 1]);
    expect(onWorkspaceMutation).toHaveBeenCalledTimes(1);
  });

  it("venter på refresh før close_cards afsluttes", async () => {
    let finishRefresh!: () => void;
    const refreshGate = new Promise<void>((resolve) => {
      finishRefresh = resolve;
    });
    const onWorkspaceMutation = vi.fn(() => refreshGate);
    let settled = false;
    const closing = createVoiceDispatcher({
      invoke,
      canvas,
      getProject: async () => ({
        root: "C:\\projekter\\demo",
        name: "demo",
      }),
      onHudEvent: (event) => hudEvents.push(event),
      onWorkspaceMutation,
    })
      .dispatch({ kind: "close_cards", cards: [3, 1] })
      .then(() => {
        settled = true;
      });

    await Promise.resolve();
    await Promise.resolve();
    expect(settled).toBe(false);
    finishRefresh();
    await closing;
    expect(settled).toBe(true);
  });

  it("maal_loes_close_afvises_stadig", async () => {
    const result = await dispatcher().dispatch({ kind: "close_cards", cards: [] });

    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).not.toHaveBeenCalledWith("close_cards", expect.anything());
    expect(hudEvents.at(-1)).toEqual({
      kind: "error",
      message: "Kommandoen mangler eksplicitte kortnumre",
    });
    expect(result).toMatchObject({ ok: false, code: "no_target" });
  });

  it("luk_alle_lukker_samtlige_aabne_kort", async () => {
    const onWorkspaceMutation = vi.fn();
    const result = await createVoiceDispatcher({
      invoke,
      canvas,
      getProject: async () => ({
        root: "C:\\projekter\\demo",
        name: "demo",
      }),
      onHudEvent: (event) => hudEvents.push(event),
      onWorkspaceMutation,
    }).dispatch({
      kind: "close_cards",
      cards: [],
      all: true,
    });
    expect(invoke).toHaveBeenCalledWith("close_cards", {
      names: ["card-1", "card-2", "card-3", "card-5"],
    });
    expect(result).toMatchObject({
      ok: true,
      kind: "close_cards",
      cards: [1, 2, 3, 5],
    });
    expect(hudEvents.map((event) => event.kind)).toEqual(["info", "info"]);
    // UI'et skal genindlæse kortlisten efter close — ellers bliver lukkede
    // kort stående som spøgelser i frontenden.
    expect(onWorkspaceMutation).toHaveBeenCalled();
  });

  it("luk_alle_paa_tom_canvas_giver_hud_fejl_uden_handling", async () => {
    invoke.mockImplementation(async (command) => {
      if (command === "list_cards") return [];
      return undefined;
    });
    const result = await dispatcher().dispatch({
      kind: "close_cards",
      cards: [],
      all: true,
    });
    expect(invoke).not.toHaveBeenCalledWith("close_cards", expect.anything());
    expect(hudEvents.at(-1)).toEqual({
      kind: "error",
      message: "Ingen åbne kort at lukke",
    });
    expect(result).toMatchObject({ ok: false, code: "no_cards" });
  });

  it("ukendt kortnummer giver HUD-fejl og ingen handling", async () => {
    await dispatcher().dispatch({ kind: "restart_card", card: 4 });

    expect(invoke).not.toHaveBeenCalledWith("kill_card", expect.anything());
    expect(invoke).not.toHaveBeenCalledWith("respawn_card", expect.anything());
    expect(invoke).not.toHaveBeenCalledWith("spawn_card", expect.anything());
    expect(hudEvents.at(-1)).toEqual({
      kind: "error",
      message: "Kortet findes ikke",
    });
  });

  it("open_browser_invoker_create_browser_card_og_svarer_med_kortnummer", async () => {
    const onWorkspaceMutation = vi.fn();
    invoke.mockImplementation(async (command) => {
      if (command === "create_browser_card") return browserCardFixture;
      return undefined;
    });
    const result = await createVoiceDispatcher({
      invoke,
      canvas,
      getProject: async () => ({
        root: "C:\\projekter\\demo",
        name: "demo",
      }),
      onHudEvent: (event) => hudEvents.push(event),
      onWorkspaceMutation,
    }).dispatch({ kind: "open_browser", url_hint: null });

    expect(invoke).toHaveBeenCalledWith("create_browser_card", {
      url: null,
      openedBy: null,
    });
    expect(result).toMatchObject({
      ok: true,
      kind: "open_browser",
      card: 6,
      message: "Browser åbnet som kort 6",
    });
    expect(onWorkspaceMutation).toHaveBeenCalledTimes(1);
  });

  it("open_browser_med_url_hint_bruger_site_hints", async () => {
    invoke.mockImplementation(async (command) => {
      if (command === "create_browser_card") return browserCardFixture;
      return undefined;
    });
    await dispatcher().dispatch({ kind: "open_browser", url_hint: "github" });
    expect(invoke).toHaveBeenCalledWith("create_browser_card", {
      url: "https://github.com",
      openedBy: null,
    });

    await dispatcher().dispatch({ kind: "open_browser", url_hint: "google" });
    expect(invoke).toHaveBeenCalledWith("create_browser_card", {
      url: "https://www.google.com",
      openedBy: null,
    });
  });

  it("router-reject viser det rå transcript og udfører intet", async () => {
    await dispatcher().dispatch(
      null,
      "Øhm… det der med Vercel deploy… eller nej, hm, vent.",
    );

    expect(invoke).not.toHaveBeenCalled();
    expect(hudEvents.at(-1)).toEqual({
      kind: "error",
      message: "Kommandoen kunne ikke fortolkes",
      rawTranscript: "Øhm… det der med Vercel deploy… eller nej, hm, vent.",
    });
  });

  describe("cast-emits (spec 2026-07-22)", () => {
    let castEvents: CastEvent[];
    let unsubscribeCast: () => void;

    beforeEach(() => {
      castEvents = [];
      unsubscribeCast = subscribeCast((event) => castEvents.push(event));
    });

    afterEach(() => {
      unsubscribeCast();
    });

    it("send_prompt emitter en cast mod det oploeste maalkort", async () => {
      await dispatcher().dispatch({ kind: "send_prompt", card: 3, text: "hej" });
      expect(castEvents).toEqual([{ card: 3, landing: "prompt" }]);
    });

    it("send_prompt uden eksplicit kort emitter mod fokus-kortet", async () => {
      await dispatcher().dispatch({
        kind: "send_prompt",
        card: null,
        text: "hej",
      });
      expect(castEvents).toEqual([{ card: 2, landing: "prompt" }]);
    });

    it("send_prompt mod browser-kort emitter ikke", async () => {
      listCardsWithBrowser();
      await dispatcher().dispatch({ kind: "send_prompt", card: 6, text: "hej" });
      expect(castEvents).toEqual([]);
    });

    it("send_prompt emitter ikke naar submit_prompt fejler", async () => {
      invoke.mockImplementation(async (command) => {
        if (command === "list_cards") return cards;
        if (command === "submit_prompt") throw new Error("pty vaek");
        return undefined;
      });
      await expect(
        dispatcher().dispatch({ kind: "send_prompt", card: 3, text: "hej" }),
      ).rejects.toThrow();
      expect(castEvents).toEqual([]);
    });

    it("new_card emitter én cast pr. oprettet kort", async () => {
      await dispatcher().dispatch({ kind: "new_card", count: 2 });
      expect(castEvents).toEqual([
        { card: 6, landing: "card" },
        { card: 6, landing: "card" },
      ]);
    });

    it("new_card emitter foerst EFTER workspace-mutationen (kortet skal kunne findes i DOM)", async () => {
      const castsSeenByMutation: number[] = [];
      const withMutation = createVoiceDispatcher({
        invoke,
        canvas,
        getProject: async () => ({
          root: "C:\\projekter\\demo",
          name: "demo",
        }),
        onHudEvent: (event) => hudEvents.push(event),
        onWorkspaceMutation: () => {
          castsSeenByMutation.push(castEvents.length);
        },
      });
      await withMutation.dispatch({ kind: "new_card", count: 1 });
      expect(castsSeenByMutation).toEqual([0]);
      expect(castEvents).toEqual([{ card: 6, landing: "card" }]);
    });

    it("delvist fejlet batch-oprettelse emitter ikke", async () => {
      let createCalls = 0;
      invoke.mockImplementation(async (command) => {
        if (command === "list_cards") return cards;
        if (command === "create_card") {
          createCalls += 1;
          if (createCalls === 2) throw new Error("spawn fejlede");
          return {
            number: 6,
            name: "card-6",
            cwd: "C:\\projekter\\demo",
            profile: "claude",
            running: false,
            exited: null,
            restore_action: null,
          };
        }
        return undefined;
      });
      await expect(
        dispatcher().dispatch({ kind: "new_card", count: 2 }),
      ).rejects.toThrow();
      expect(castEvents).toEqual([]);
    });

    it("open_browser emitter cast mod browser-kortet", async () => {
      invoke.mockImplementation(async (command) => {
        if (command === "create_browser_card") return browserCardFixture;
        return undefined;
      });
      await dispatcher().dispatch({ kind: "open_browser", url_hint: null });
      expect(castEvents).toEqual([{ card: 6, landing: "card" }]);
    });

    it("close_cards og restart_card emitter ikke", async () => {
      await dispatcher().dispatch({ kind: "close_cards", cards: [1] });
      await dispatcher().dispatch({ kind: "restart_card", card: 1 });
      expect(castEvents).toEqual([]);
    });

    it("router-reject emitter ikke", async () => {
      await dispatcher().dispatch(null, "hvad laver kort 3");
      expect(castEvents).toEqual([]);
    });
  });
});
