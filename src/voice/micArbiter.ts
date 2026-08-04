// Hvem holder mikrofonen? Der er to genveje (PTT og diktering) og én mikrofon.
//
// Gaten kan IKKE vaere "er den anden session idle". Pipelinen staar i
// `processing` og `speaking` mens svarklippet afspilles — flere sekunder efter
// tasten er sluppet og mikrofonen forlaengst lukket — saa en tilstands-baseret
// gate ville sluge diktér-tryk i det vindue.
//
// Ejerskabet foelger HOLDET i stedet: mikrofonen er min saa laenge jeg holder
// min tast nede. Det giver den rigtige opfoersel gratis, fordi beslutningen
// huskes fra press til release: fik trykket ikke mikrofonen, finder slippet
// heller ikke sin ejer, og den anden sessions tur roeres slet ikke.

export type MicOwner = "ptt" | "dictation";

export interface MicArbiter {
  /** `true` hvis kalderen fik mikrofonen og maa starte sin tur. */
  acquire(who: MicOwner): boolean;
  /** `true` hvis kalderen ejede mikrofonen og maa afslutte sin tur. */
  release(who: MicOwner): boolean;
  owner(): MicOwner | null;
}

export function createMicArbiter(): MicArbiter {
  let current: MicOwner | null = null;
  return {
    acquire(who) {
      if (current !== null) return false;
      current = who;
      return true;
    },
    release(who) {
      if (current !== who) return false;
      current = null;
      return true;
    },
    owner() {
      return current;
    },
  };
}
