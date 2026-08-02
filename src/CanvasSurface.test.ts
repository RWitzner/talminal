import { describe, expect, it, vi } from "vitest";
import { loadSpawnCwdPrefill, resolveSpawnPrefill } from "./CanvasSurface";

describe("CanvasSurface dialog prefill", () => {
  it("dialog_prefiller_projekt_rod", async () => {
    const getProject = vi.fn(async () => ({
      root: "C:\\projekter\\demo\\canvas",
      name: "canvas",
    }));
    await expect(loadSpawnCwdPrefill(getProject)).resolves.toBe(
      "C:\\projekter\\demo\\canvas",
    );
    expect(getProject).toHaveBeenCalledTimes(1);
  });

  it("prefill_udfylder_kun_uroert_felt", () => {
    expect(resolveSpawnPrefill("", "C:\\proj", true)).toBe("C:\\proj");
  });

  it("prefill_overskriver_aldrig_brugerens_input", () => {
    expect(resolveSpawnPrefill("C:\\mit\\eget\\valg", "C:\\proj", true)).toBe(
      "C:\\mit\\eget\\valg",
    );
  });

  it("prefill_ignorerer_stale_resolve_fra_tidligere_aabning", () => {
    expect(resolveSpawnPrefill("", "C:\\gammel\\rod", false)).toBe("");
  });
});
