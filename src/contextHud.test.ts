import { describe, expect, it } from "vitest";
import { contextPercentFor, type ContextSnapshot } from "./contextHud";

function snapshot(overrides: Partial<ContextSnapshot> = {}): ContextSnapshot {
  return {
    version: 1,
    writtenAt: "2026-07-22T10:00:00.000Z",
    cardName: "kort-3",
    runId: "run-1",
    sessionId: "cc-1",
    cwd: "C:\\proj",
    usedPercent: 18,
    windowSize: 1_000_000,
    modelDisplayName: "Fable 5",
    ...overrides,
  };
}

describe("contextPercentFor", () => {
  it("matcher på kortnavn OG cwd (spec-join)", () => {
    const snapshots = [snapshot(), snapshot({ cardName: "kort-4", usedPercent: 55 })];
    expect(contextPercentFor(snapshots, "kort-3", "C:\\proj")).toBe(18);
    expect(contextPercentFor(snapshots, "kort-4", "C:\\proj")).toBe(55);
  });

  it("cwd-mismatch → null (navnekollisions-værnet)", () => {
    expect(contextPercentFor([snapshot()], "kort-3", "C:\\andet-projekt")).toBeNull();
  });

  it("intet match eller tom liste → null", () => {
    expect(contextPercentFor([], "kort-3", "C:\\proj")).toBeNull();
    expect(contextPercentFor([snapshot()], "kort-9", "C:\\proj")).toBeNull();
  });

  it("klamper defensivt og afviser ikke-endelige tal", () => {
    expect(
      contextPercentFor([snapshot({ usedPercent: 130 })], "kort-3", "C:\\proj"),
    ).toBe(100);
    expect(
      contextPercentFor(
        [snapshot({ usedPercent: Number.NaN })],
        "kort-3",
        "C:\\proj",
      ),
    ).toBeNull();
  });
});
