// Task 9: ren world/screen-matematik (viewport.ts).
//
// Konvention (dokumenteret i viewport.ts, spejler Rust-defaulten
// {x:0, y:0, zoom:1}): (v.x, v.y) er WORLD-punktet i skaerm-origo (0,0);
// zoom er skaerm-px pr. world-enhed. Invarianterne her er planens navngivne
// testcases: roundtrip (screenToWorld ∘ worldToScreen == id) og zoomAround-
// fixpunktet (skaermpunktet er invariant under zoom).

import { describe, expect, it } from "vitest";
import {
  screenToWorld,
  viewCenterWorld,
  worldToScreen,
  zoomAround,
  type Viewport,
} from "./viewport";

const VIEWPORTS: Viewport[] = [
  { x: 0, y: 0, zoom: 1 }, // Rust-defaulten (empty_file)
  { x: 120, y: -80, zoom: 1 },
  { x: -300.5, y: 250.25, zoom: 0.5 },
  { x: 1000, y: 2000, zoom: 2.75 },
  { x: 5.5, y: -3.25, zoom: 0.1 },
];

const POINTS = [
  { x: 0, y: 0 },
  { x: 117, y: -42 },
  { x: -1234.5, y: 987.25 },
  { x: 0.125, y: 0.375 },
];

describe("viewport roundtrip", () => {
  it("screenToWorld(worldToScreen(p)) == p for repraesentative viewports", () => {
    for (const v of VIEWPORTS) {
      for (const p of POINTS) {
        const back = screenToWorld(v, worldToScreen(v, p));
        expect(back.x).toBeCloseTo(p.x, 9);
        expect(back.y).toBeCloseTo(p.y, 9);
      }
    }
  });

  it("worldToScreen(screenToWorld(s)) == s (modsat retning)", () => {
    for (const v of VIEWPORTS) {
      for (const s of POINTS) {
        const back = worldToScreen(v, screenToWorld(v, s));
        expect(back.x).toBeCloseTo(s.x, 9);
        expect(back.y).toBeCloseTo(s.y, 9);
      }
    }
  });

  it("identitets-viewportet er en no-op", () => {
    const id: Viewport = { x: 0, y: 0, zoom: 1 };
    expect(worldToScreen(id, { x: 42, y: -7 })).toEqual({ x: 42, y: -7 });
    expect(screenToWorld(id, { x: 42, y: -7 })).toEqual({ x: 42, y: -7 });
  });
});

describe("zoomAround", () => {
  const SCREEN_POINTS = [
    { x: 0, y: 0 },
    { x: 700, y: 450 }, // vindues-midte-klassen
    { x: 13.5, y: 899 },
  ];
  const FACTORS = [2, 0.5, 1.1, 1 / 1.1, 3.7];

  it("skaermpunktet er fixpunkt: world-punktet under cursoren bliver staaende", () => {
    for (const v of VIEWPORTS) {
      for (const sp of SCREEN_POINTS) {
        for (const f of FACTORS) {
          const worldUnderCursor = screenToWorld(v, sp);
          const zoomed = zoomAround(v, sp, f);
          const after = worldToScreen(zoomed, worldUnderCursor);
          expect(after.x).toBeCloseTo(sp.x, 7);
          expect(after.y).toBeCloseTo(sp.y, 7);
        }
      }
    }
  });

  it("zoom multipliceres med faktoren", () => {
    for (const v of VIEWPORTS) {
      expect(zoomAround(v, { x: 100, y: 100 }, 2).zoom).toBeCloseTo(v.zoom * 2, 9);
      expect(zoomAround(v, { x: 100, y: 100 }, 0.25).zoom).toBeCloseTo(v.zoom * 0.25, 9);
    }
  });

  it("faktor 1 er en no-op", () => {
    for (const v of VIEWPORTS) {
      const z = zoomAround(v, { x: 321, y: 123 }, 1);
      expect(z.x).toBeCloseTo(v.x, 9);
      expect(z.y).toBeCloseTo(v.y, 9);
      expect(z.zoom).toBeCloseTo(v.zoom, 9);
    }
  });
});

describe("viewCenterWorld", () => {
  it("identitets-viewportet centrerer i halv vinduesstoerrelse", () => {
    expect(viewCenterWorld({ x: 0, y: 0, zoom: 1 }, { w: 1400, h: 900 })).toEqual({
      x: 700,
      y: 450,
    });
  });

  it("panned+zoomed viewport: centrum = hjoerne + halvt vindue i world-enheder", () => {
    expect(
      viewCenterWorld({ x: 2000, y: 1000, zoom: 2 }, { w: 1400, h: 900 }),
    ).toEqual({ x: 2350, y: 1225 });
  });

  it("invariant: centret projicerer tilbage til skaermens midtpunkt", () => {
    for (const v of VIEWPORTS) {
      const size = { w: 1280, h: 720 };
      const screen = worldToScreen(v, viewCenterWorld(v, size));
      expect(screen.x).toBeCloseTo(size.w / 2, 9);
      expect(screen.y).toBeCloseTo(size.h / 2, 9);
    }
  });
});
