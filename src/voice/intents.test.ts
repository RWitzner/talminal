import { describe, expect, it } from "vitest";
import {
  resolveTarget,
  resolveTargets,
  type VoiceIntent,
} from "./intents";

// Intent-kernen — rene moduler, ZERO app-afhængigheder.
// Låste regler (schema v4 — 5 kommandoer):
//   - send_prompt m. card: null → fokuseret kort KUN når præcis ét er fokuseret
//     (focusedCard != null), ellers ambiguous_focus
//   - ukendt kortnummer → no_such_card
//   - restart_card uden mål → no_target (ALDRIG implicit fokus-fallback)

const cards = [{ number: 1 }, { number: 2 }, { number: 3 }];

describe("resolveTarget: send_prompt (card nullable, fokus-fallback)", () => {
  it("card: null + præcis ét fokuseret kort → det fokuserede kort", () => {
    expect(
      resolveTarget({ kind: "send_prompt", card: null, text: "hej" }, 2, cards),
    ).toEqual({ ok: true, card: 2 });
  });

  it("card: null + intet fokus → ambiguous_focus", () => {
    expect(
      resolveTarget({ kind: "send_prompt", card: null, text: "hej" }, null, cards),
    ).toEqual({ ok: false, reason: "ambiguous_focus" });
  });

  it("card: null + stale fokus (kortet findes ikke længere) → no_such_card", () => {
    expect(
      resolveTarget({ kind: "send_prompt", card: null, text: "hej" }, 4, cards),
    ).toEqual({ ok: false, reason: "no_such_card" });
  });

  it("card: null + fokus men tom kortliste → no_such_card", () => {
    expect(
      resolveTarget({ kind: "send_prompt", card: null, text: "hej" }, 2, []),
    ).toEqual({ ok: false, reason: "no_such_card" });
  });

  it("eksplicit kortnummer vinder over fokus", () => {
    expect(
      resolveTarget({ kind: "send_prompt", card: 3, text: "hej" }, 1, cards),
    ).toEqual({ ok: true, card: 3 });
  });

  it("eksplicit ukendt kortnummer → no_such_card (selv med gyldigt fokus)", () => {
    expect(
      resolveTarget({ kind: "send_prompt", card: 9, text: "hej" }, 1, cards),
    ).toEqual({ ok: false, reason: "no_such_card" });
  });
});

describe("resolveTarget: restart_card (eneste rene kort-target udover send_prompt)", () => {
  it("restart_card: kendt kortnummer → ok", () => {
    expect(resolveTarget({ kind: "restart_card", card: 2 }, null, cards)).toEqual({
      ok: true,
      card: 2,
    });
  });

  it("restart_card: ukendt kortnummer → no_such_card", () => {
    expect(resolveTarget({ kind: "restart_card", card: 7 }, null, cards)).toEqual({
      ok: false,
      reason: "no_such_card",
    });
  });

  it("restart_card uden nummer giver no_target", () => {
    expect(
      resolveTarget({ kind: "restart_card", card: null }, null, [{ number: 1 }]),
    ).toEqual({ ok: false, reason: "no_target" });
  });

  it("restart_card: uden mål → no_target — ALDRIG fokus-fallback, selv med fokus", () => {
    expect(resolveTarget({ kind: "restart_card", card: null }, 2, cards)).toEqual({
      ok: false,
      reason: "no_target",
    });
  });

  it("restart_card: eksplicit nummer vinder over andet fokus", () => {
    expect(resolveTarget({ kind: "restart_card", card: 1 }, 3, cards)).toEqual({
      ok: true,
      card: 1,
    });
  });
});

describe("resolveTarget: mål-løse intents (new_card, open_browser, close_cards)", () => {
  it("new_card og open_browser er target-løse", () => {
    for (const intent of [
      { kind: "new_card", count: 2 },
      { kind: "open_browser", url_hint: null },
    ] as const) {
      expect(resolveTarget(intent, 2, [{ number: 2 }])).toEqual({
        ok: false,
        reason: "no_target",
      });
    }
  });

  it("close_cards og new_card uden count → no_target (uanset fokus)", () => {
    const intents: VoiceIntent[] = [
      { kind: "close_cards", cards: [1] },
      { kind: "new_card" },
    ];
    for (const intent of intents) {
      expect(resolveTarget(intent, 2, cards)).toEqual({
        ok: false,
        reason: "no_target",
      });
      expect(resolveTarget(intent, null, cards)).toEqual({
        ok: false,
        reason: "no_target",
      });
    }
  });
});

describe("resolveTargets: close_cards", () => {
  it("accepterer flere eksplicitte, kendte mål i samme rækkefølge", () => {
    expect(resolveTargets({ kind: "close_cards", cards: [3, 1] }, cards)).toEqual({
      ok: true,
      cards: [3, 1],
    });
  });

  it("tom eller null-liste giver no_target — antal må aldrig blive gættet", () => {
    expect(resolveTargets({ kind: "close_cards", cards: [] }, cards)).toEqual({
      ok: false,
      reason: "no_target",
    });
    expect(
      resolveTargets(
        { kind: "close_cards", cards: null } as unknown as VoiceIntent,
        cards,
      ),
    ).toEqual({ ok: false, reason: "no_target" });
  });

  it("ukendt mål blokerer hele batchen", () => {
    expect(resolveTargets({ kind: "close_cards", cards: [2, 4] }, cards)).toEqual({
      ok: false,
      reason: "no_such_card",
    });
  });

  it("dubletter normaliseres uden at ændre første rækkefølge", () => {
    expect(resolveTargets({ kind: "close_cards", cards: [2, 2, 3] }, cards)).toEqual({
      ok: true,
      cards: [2, 3],
    });
  });

  it("all-flaget ekspanderer til samtlige åbne kort i nummerorden", () => {
    expect(
      resolveTargets({ kind: "close_cards", cards: [], all: true }, cards),
    ).toEqual({ ok: true, cards: [1, 2, 3] });
  });

  it("all-flaget vinder over medsendte numre — resolveren opdigter aldrig delmængder", () => {
    expect(
      resolveTargets({ kind: "close_cards", cards: [9], all: true }, cards),
    ).toEqual({ ok: true, cards: [1, 2, 3] });
  });

  it("all-flaget på tom canvas giver no_cards, ikke handling", () => {
    expect(
      resolveTargets({ kind: "close_cards", cards: [], all: true }, []),
    ).toEqual({ ok: false, reason: "no_cards" });
  });
});
