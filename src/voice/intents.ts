// Ren voice-intent-kerne: schema-typer og target-resolution (schema v4 — 5 kommandoer; kæder er arrays af intents, typen er uændret).

export type VoiceIntent =
  | { kind: "send_prompt"; card: number | null; text: string }
  | { kind: "new_card"; count?: number; agent?: "claude" | "codex" }
  | { kind: "close_cards"; cards: number[] | null; all?: boolean }
  | { kind: "restart_card"; card: number | null }
  | { kind: "open_browser"; url_hint: "github" | "google" | null };

export type ResolveResult =
  | { ok: true; card: number }
  | { ok: false; reason: "ambiguous_focus" | "no_such_card" | "no_target" };

export type ResolveTargetsResult =
  | { ok: true; cards: number[] }
  | { ok: false; reason: "no_such_card" | "no_target" | "no_cards" };

export function resolveTarget(
  intent: VoiceIntent,
  focusedCard: number | null,
  cards: Array<{ number: number }>,
): ResolveResult {
  let candidate: number | null;

  switch (intent.kind) {
    case "send_prompt":
      if (intent.card != null) candidate = intent.card;
      else if (focusedCard != null) candidate = focusedCard;
      else return { ok: false, reason: "ambiguous_focus" };
      break;
    case "restart_card":
      candidate = intent.card;
      if (candidate == null) return { ok: false, reason: "no_target" };
      break;
    case "close_cards":
    case "new_card":
    case "open_browser":
      return { ok: false, reason: "no_target" };
  }

  return cards.some((card) => card.number === candidate)
    ? { ok: true, card: candidate }
    : { ok: false, reason: "no_such_card" };
}

export function resolveTargets(
  intent: VoiceIntent,
  cards: Array<{ number: number }>,
): ResolveTargetsResult {
  if (intent.kind !== "close_cards") {
    return { ok: false, reason: "no_target" };
  }
  if (intent.all === true) {
    // "Alle" ekspanderes ALTID fra live-state — medsendte numre ignoreres,
    // resolveren opdigter aldrig delmængder.
    if (cards.length === 0) return { ok: false, reason: "no_cards" };
    return {
      ok: true,
      cards: cards.map((card) => card.number).sort((left, right) => left - right),
    };
  }
  if (!Array.isArray(intent.cards)) {
    return { ok: false, reason: "no_target" };
  }
  const targets = intent.cards.filter(
    (card, index, all) =>
      Number.isInteger(card) && card > 0 && all.indexOf(card) === index,
  );
  if (targets.length === 0) return { ok: false, reason: "no_target" };
  if (targets.some((target) => !cards.some((card) => card.number === target))) {
    return { ok: false, reason: "no_such_card" };
  }
  return { ok: true, cards: targets };
}
