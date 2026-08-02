import type { HudSessionState } from "./Hud";

/**
 * Voice-orbens tilstandskerne (spec 2026-07-20 §4) — port af Redapting-
 * companionens lytte-orb, uden pet-tilstanden: orben er permanent synlig,
 * idle er et dæmpet udtryk, ikke en skjult orb.
 *
 * Flash-laget er et separat PRIORITETSLAG oven på session-mappingen
 * (companion-lærestregen: en status-regel og en fejl-hale-regel med samme
 * trigger-øjeblik skal have eksplicit prioritet — her: flash > session).
 * Ingen timere i kernen; udløb afgøres ved sammenligning med `now`.
 */
export type OrbMode = "idle" | "listening" | "working" | "speaking";

export const ORB_ERROR_FLASH_MS = 900;
export const ORB_ERROR_FLASH_LEVEL = 0.6;
/** Lerp-faktor og freshness-tærskel er låst fra companion-mockup-designet. */
export const ORB_LEVEL_LERP = 0.16;
export const ORB_LEVEL_FALLBACK_AFTER_MS = 400;
/** Stille passager i svaret skal stadig ånde: speaking-target er
 * max(outputNiveau, autonom · denne dæmpning). */
export const ORB_SPEAKING_AUTONOMOUS_DAMP = 0.6;

const MODE_BY_SESSION: Record<HudSessionState, OrbMode> = {
  idle: "idle",
  asleep: "idle",
  sleeping: "idle",
  listening: "listening",
  awake: "listening",
  processing: "working",
  waking: "working",
  draining: "working",
  speaking: "speaking",
};

export function orbModeForSession(session: HudSessionState): OrbMode {
  return MODE_BY_SESSION[session];
}

export type OrbFlashState = { flashUntil: number | null };

/** Ny fejl under et aktivt blus genstarter timeren (flashUntil rykkes frem). */
export function triggerErrorFlash(
  _state: OrbFlashState,
  now: number,
): OrbFlashState {
  return { flashUntil: now + ORB_ERROR_FLASH_MS };
}

/** data-mode-attributten på orb-roden (CSS-kontrakten). Blusset kører sin
 * fulde varighed hen over sessionskift. */
export function orbVisualAttr(
  session: HudSessionState,
  flash: OrbFlashState,
  now: number,
): string {
  if (flash.flashUntil !== null && now < flash.flashUntil) {
    return "flash-error";
  }
  return orbModeForSession(session);
}

/** Autonom "vejrtrækning" — verbatim companion-kurven. */
export function autonomousOrbLevel(nowMs: number): number {
  return 0.22 + 0.12 * Math.sin(nowMs / 700);
}

/**
 * Målniveau pr. frame. Orben må aldrig fryse: listening falder tilbage til
 * den autonome puls når mic-samples udebliver, speaking bunder i en dæmpet
 * autonom kurve, og idle deler kurven (dæmpningen er ren CSS).
 */
export function orbLevelTarget(input: {
  attr: string;
  now: number;
  mic: { level: number; at: number };
  outputLevel: number;
}): number {
  if (input.attr === "flash-error") {
    return ORB_ERROR_FLASH_LEVEL;
  }
  if (input.attr === "listening") {
    if (input.now - input.mic.at <= ORB_LEVEL_FALLBACK_AFTER_MS) {
      return input.mic.level;
    }
    return autonomousOrbLevel(input.now);
  }
  if (input.attr === "speaking") {
    return Math.max(
      input.outputLevel,
      autonomousOrbLevel(input.now) * ORB_SPEAKING_AUTONOMOUS_DAMP,
    );
  }
  return autonomousOrbLevel(input.now);
}
