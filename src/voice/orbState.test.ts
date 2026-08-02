import { describe, expect, it } from "vitest";
import type { HudSessionState } from "./Hud";
import {
  autonomousOrbLevel,
  ORB_ERROR_FLASH_LEVEL,
  ORB_ERROR_FLASH_MS,
  ORB_LEVEL_FALLBACK_AFTER_MS,
  ORB_SPEAKING_AUTONOMOUS_DAMP,
  orbLevelTarget,
  orbModeForSession,
  orbVisualAttr,
  triggerErrorFlash,
  type OrbMode,
} from "./orbState";

describe("orbModeForSession", () => {
  const table: Array<[HudSessionState, OrbMode]> = [
    ["idle", "idle"],
    ["asleep", "idle"],
    ["sleeping", "idle"],
    ["listening", "listening"],
    ["awake", "listening"],
    ["processing", "working"],
    ["waking", "working"],
    ["draining", "working"],
    ["speaking", "speaking"],
  ];

  it.each(table)("%s -> %s", (session, mode) => {
    expect(orbModeForSession(session)).toBe(mode);
  });
});

describe("orbVisualAttr (flash > session)", () => {
  it("uden blus følger attributten sessionen", () => {
    expect(orbVisualAttr("speaking", { flashUntil: null }, 1_000)).toBe(
      "speaking",
    );
  });

  it("et aktivt blus vinder over ethvert sessionskift", () => {
    const flash = triggerErrorFlash({ flashUntil: null }, 1_000);
    expect(flash.flashUntil).toBe(1_000 + ORB_ERROR_FLASH_MS);
    // Sessionen skifter midt i blusset — blusset kører sin fulde varighed.
    expect(orbVisualAttr("listening", flash, 1_100)).toBe("flash-error");
    expect(orbVisualAttr("idle", flash, 1_000 + ORB_ERROR_FLASH_MS - 1)).toBe(
      "flash-error",
    );
  });

  it("blusset udløber ved flashUntil og attributten falder til sessionen", () => {
    const flash = triggerErrorFlash({ flashUntil: null }, 1_000);
    expect(orbVisualAttr("idle", flash, 1_000 + ORB_ERROR_FLASH_MS)).toBe(
      "idle",
    );
  });

  it("ny fejl under aktivt blus genstarter timeren", () => {
    const first = triggerErrorFlash({ flashUntil: null }, 1_000);
    const second = triggerErrorFlash(first, 1_500);
    expect(second.flashUntil).toBe(1_500 + ORB_ERROR_FLASH_MS);
    expect(
      orbVisualAttr("idle", second, 1_000 + ORB_ERROR_FLASH_MS + 10),
    ).toBe("flash-error");
  });
});

describe("autonomousOrbLevel", () => {
  it("holder sig i companion-kurvens bånd [0.10, 0.34]", () => {
    for (let ms = 0; ms < 10_000; ms += 97) {
      const level = autonomousOrbLevel(ms);
      expect(level).toBeGreaterThanOrEqual(0.1);
      expect(level).toBeLessThanOrEqual(0.34);
    }
  });
});

describe("orbLevelTarget", () => {
  const mic = { level: 0.8, at: 5_000 };

  it("flash-error bruger det faste blus-niveau", () => {
    expect(
      orbLevelTarget({ attr: "flash-error", now: 5_000, mic, outputLevel: 0 }),
    ).toBe(ORB_ERROR_FLASH_LEVEL);
  });

  it("listening følger mic-niveauet mens samples er friske", () => {
    expect(
      orbLevelTarget({
        attr: "listening",
        now: 5_000 + ORB_LEVEL_FALLBACK_AFTER_MS,
        mic,
        outputLevel: 0,
      }),
    ).toBe(0.8);
  });

  it("listening falder til den autonome puls når samples udebliver", () => {
    const now = 5_000 + ORB_LEVEL_FALLBACK_AFTER_MS + 1;
    expect(
      orbLevelTarget({ attr: "listening", now, mic, outputLevel: 0 }),
    ).toBe(autonomousOrbLevel(now));
  });

  it("speaking følger output-niveauet når det er over den dæmpede bund", () => {
    expect(
      orbLevelTarget({ attr: "speaking", now: 5_000, mic, outputLevel: 0.9 }),
    ).toBe(0.9);
  });

  it("speaking ånder videre i stille passager (dæmpet autonom bund)", () => {
    const now = 5_000;
    expect(
      orbLevelTarget({ attr: "speaking", now, mic, outputLevel: 0 }),
    ).toBe(autonomousOrbLevel(now) * ORB_SPEAKING_AUTONOMOUS_DAMP);
  });

  it("working og idle deler den autonome kurve", () => {
    const now = 7_000;
    expect(orbLevelTarget({ attr: "working", now, mic, outputLevel: 0 })).toBe(
      autonomousOrbLevel(now),
    );
    expect(orbLevelTarget({ attr: "idle", now, mic, outputLevel: 0 })).toBe(
      autonomousOrbLevel(now),
    );
  });
});
