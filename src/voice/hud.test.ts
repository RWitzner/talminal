import { describe, expect, it } from "vitest";
import { deriveHudView, type HudState } from "./Hud";

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

describe("deriveHudView", () => {
  it("viser en klar vågen/sovende-indikator", () => {
    expect(deriveHudView(state({ session: "asleep" }), 0).sessionLabel).toBe(
      "Sovende",
    );
    expect(deriveHudView(state({ session: "listening" }), 0).sessionLabel).toBe(
      "Lytter",
    );
    expect(deriveHudView(state({ session: "processing" }), 0).sessionLabel).toBe(
      "Fortolker",
    );
    expect(deriveHudView(state({ session: "speaking" }), 0).sessionLabel).toBe(
      "Svarer",
    );
    expect(deriveHudView(state({ session: "idle" }), 0).sessionLabel).toBe(
      "Klar — hold tasten og tal",
    );
  });

  it("viser draining-tilstand som afslutter svar…", () => {
    const view = deriveHudView(state({ session: "draining" }), 0);
    expect(view.sessionLabel).toBe("afslutter svar…");
    expect(view.statusText).toBe("afslutter svar…");
  });

  it("viser fortolket tool, alle mål og resolver-resultat uden at gætte", () => {
    const view = deriveHudView(
      state({
        transcript: "Luk kort to og kort tre",
        tool: { name: "close_cards", arguments: { cards: [2, 3] } },
        resolver: { ok: true, cards: [2, 3] },
      }),
      0,
    );

    expect(view.transcript).toBe("Luk kort to og kort tre");
    expect(view.toolLabel).toBe("close_cards");
    expect(view.targetCards).toEqual([2, 3]);
    expect(view.resolverLabel).toBe("Mål: kort 2, 3");
  });

  it("prioriterer konkrete fejltekster over neutral status", () => {
    expect(
      deriveHudView(
        state({ error: "Kommandoen mangler eksplicitte kortnumre" }),
        0,
      ).statusText,
    ).toBe("Kommandoen mangler eksplicitte kortnumre");
  });

  it("deriveHudView bærer kæde-linjerne igennem med status", () => {
    const view = deriveHudView(
      {
        session: "idle",
        transcript: "åbn en terminal og en browser",
        responseText: "Udført",
        tool: null,
        resolver: null,
        error: null,
        chain: [
          { name: "new_card", ok: true, label: "1 kort oprettet" },
          { name: "open_browser", ok: false, label: "Noget gik galt" },
        ],
      },
      0,
    );
    expect(view.chain).toHaveLength(2);
    expect(view.chain?.[1]).toMatchObject({ ok: false });
  });

  it("deriveHudView uden kæde giver chain: null", () => {
    const view = deriveHudView(
      {
        session: "idle",
        transcript: "",
        responseText: "",
        tool: null,
        resolver: null,
        error: null,
        chain: null,
      },
      0,
    );
    expect(view.chain).toBeNull();
  });
});
