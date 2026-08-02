import { describe, expect, it } from "vitest";
import { setOcclusionReason, _resetForTest } from "./occlusion";

describe("occlusion-gate", () => {
  it("invoker kun paa tom<->ikke-tom-transitioner", () => {
    const calls: boolean[] = [];
    _resetForTest((occluded) => calls.push(occluded));
    setOcclusionReason("spawn-dialog", true);
    setOcclusionReason("hud-panel", true); // allerede occluded - ingen ny invoke
    setOcclusionReason("spawn-dialog", false); // stadig occluded via hud-panel
    setOcclusionReason("hud-panel", false); // nu tom -> vis
    expect(calls).toEqual([true, false]);
  });
});
