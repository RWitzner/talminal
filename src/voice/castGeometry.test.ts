import { describe, expect, it } from "vitest";
import {
  CAST_CURVE_K,
  castApproxLength,
  castControlPoint,
  castPathD,
  type CastPoint,
} from "./castGeometry";

describe("castControlPoint", () => {
  it("buer opad for en vandret rute (kontrolpunkt over midtpunktet)", () => {
    const p0: CastPoint = { x: 0, y: 100 };
    const p2: CastPoint = { x: 200, y: 100 };
    const cp = castControlPoint(p0, p2);
    expect(cp.x).toBeCloseTo(100);
    expect(cp.y).toBeLessThan(100);
  });

  it("ligger normal-forskudt k gange rutelaengden fra midtpunktet", () => {
    // Lodret opad-rute (orb nederst -> kort oeverst): normalen peger til siden.
    const p0: CastPoint = { x: 0, y: 0 };
    const p2: CastPoint = { x: 0, y: -200 };
    const cp = castControlPoint(p0, p2);
    const distanceFromMid = Math.hypot(cp.x - 0, cp.y - -100);
    expect(distanceFromMid).toBeCloseTo(200 * CAST_CURVE_K);
  });

  it("degenererer til punktet selv naar p0 == p2", () => {
    const p: CastPoint = { x: 40, y: 40 };
    expect(castControlPoint(p, p)).toEqual({ x: 40, y: 40 });
  });
});

describe("castPathD", () => {
  it("formaterer en kvadratisk bezier-path", () => {
    expect(castPathD({ x: 1, y: 2 }, { x: 3, y: 4 }, { x: 5, y: 6 })).toBe(
      "M 1 2 Q 3 4 5 6",
    );
  });
});

describe("castApproxLength", () => {
  it("er korde-laengden naar kontrolpunktet ligger paa linjen", () => {
    const p0: CastPoint = { x: 0, y: 0 };
    const p2: CastPoint = { x: 100, y: 0 };
    expect(castApproxLength(p0, { x: 50, y: 0 }, p2)).toBeCloseTo(100);
  });

  it("er laengere end korden naar ruten buer", () => {
    const p0: CastPoint = { x: 0, y: 0 };
    const p2: CastPoint = { x: 100, y: 0 };
    expect(castApproxLength(p0, { x: 50, y: 40 }, p2)).toBeGreaterThan(100);
  });
});
