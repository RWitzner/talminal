// REN funktion: lokal kort-tilstand -> kantfarve. Ingen runtime-imports,
// ingen sideeffekter — testes isoleret med vitest (colors.test.ts).
//
// Task 4 (MVP-pathen): presence-laget er ude — farven er en funktion af
// LOKAL `running`/`exited` alene (spawn-flow + card-exit-event +
// get_card_state-hydrering i Card.tsx). Ingen controller-heartbeat, ingen
// stale-regel, ingen blaa pause-farve, ingen gul attention-farve.

import type { CardColor } from "./types";

/** Lokal kort-tilstand som Card.tsx kender den (spejler CardState.running /
 *  CardState.exited — exit-koden fra pty-child'en, null = aldrig exited). */
export type CardRunState = {
  running: boolean;
  exited: number | null;
};

/**
 * Raekkefoelgen ER precedensen: running > exited > ikke-startet.
 * - green:   kortet koerer (en frisk respawn vinder over gammel exit-kode)
 * - red:     exited med kode != 0 (crash/kill)
 * - neutral: exited med kode 0 (rent afsluttet)
 * - gray:    ikke startet / ingen historik
 */
export function cardColor(state: CardRunState): CardColor {
  if (state.running) return "green";
  if (state.exited !== null) return state.exited === 0 ? "neutral" : "red";
  return "gray";
}
