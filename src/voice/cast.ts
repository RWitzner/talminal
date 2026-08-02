// Cast-bus (spec 2026-07-22): dispatch emitter, CastLayer abonnerer.
// Ren pynt-kanal — maa ALDRIG kaste ind i dispatch-vejen, derfor er hvert
// abonnent-kald try/catch-pakket. Ingen React-imports, ingen DOM.

/**
 * landing styrer hvor straalen slaar ned (ejer-beslutning 2026-07-22 aften):
 * - "prompt": terminalens tekstfelt (send_prompt — prompten lander I feltet)
 * - "card":   kortets centrum (new_card/open_browser — selve kortet er maalet)
 */
export type CastLanding = "prompt" | "card";

export type CastEvent = { card: number; landing: CastLanding };

type CastListener = (event: CastEvent) => void;

const listeners = new Set<CastListener>();

export function subscribeCast(listener: CastListener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function emitCast(event: CastEvent): void {
  for (const listener of [...listeners]) {
    try {
      listener(event);
    } catch {
      // Pynt maa aldrig vaelte noget — fejl i en abonnent sluges tavst.
    }
  }
}

/**
 * Burst-forskydning: naar flere casts affyres samlet (multi-spawn emitter
 * efter onWorkspaceMutation, saa alle events kommer i samme tick), forskyder
 * CastLayer dem CAST_STAGGER_MS ad gangen, saa N straaler laeses som en vifte
 * og ikke ét blink. Stilhed laengere end CAST_BURST_RESET_MS starter ny burst.
 */
export type CastBurstState = { count: number; lastAt: number };

export const CAST_STAGGER_MS = 60;
export const CAST_BURST_RESET_MS = 150;
export const CAST_BURST_INITIAL: CastBurstState = {
  count: 0,
  lastAt: Number.NEGATIVE_INFINITY,
};

export function nextCastDelay(
  state: CastBurstState,
  now: number,
): { state: CastBurstState; delay: number } {
  const count = now - state.lastAt > CAST_BURST_RESET_MS ? 0 : state.count;
  return {
    state: { count: count + 1, lastAt: now },
    delay: count * CAST_STAGGER_MS,
  };
}
