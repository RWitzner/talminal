import { describe, expect, it } from "vitest";
import {
  computeSpawnPositions,
  type CardGeom,
  type Position,
} from "./autoLayout";

const CARD_W = 960;
const CARD_H = 640;
const anchor = { x: 2_000, y: 1_000 };

function overlaps(a: Position, b: Position): boolean {
  return !(
    a.x + CARD_W <= b.x ||
    b.x + CARD_W <= a.x ||
    a.y + CARD_H <= b.y ||
    b.y + CARD_H <= a.y
  );
}

function overlapsExisting(position: Position, existing: CardGeom): boolean {
  return !(
    position.x + CARD_W <= existing.x ||
    existing.x + existing.w <= position.x ||
    position.y + CARD_H <= existing.y ||
    existing.y + existing.h <= position.y
  );
}

describe("computeSpawnPositions", () => {
  it("placerer ét kort centreret på spawn-ankeret på tomt canvas", () => {
    expect(computeSpawnPositions([], 1, anchor)).toEqual([
      { x: anchor.x - CARD_W / 2, y: anchor.y - CARD_H / 2 },
    ]);
  });

  for (const count of [3, 10]) {
    it(`giver ${count} indbyrdes ikke-overlappende grid-positioner`, () => {
      const positions = computeSpawnPositions([], count, anchor);
      expect(positions).toHaveLength(count);
      for (let left = 0; left < positions.length; left += 1) {
        for (let right = left + 1; right < positions.length; right += 1) {
          expect(overlaps(positions[left], positions[right])).toBe(false);
        }
      }
    });
  }

  it("springer kandidater over der kolliderer med eksisterende geometri", () => {
    const existing: CardGeom[] = [
      {
        x: anchor.x - CARD_W / 2,
        y: anchor.y - CARD_H / 2,
        w: CARD_W,
        h: CARD_H,
      },
      { x: 2_988, y: 680, w: 420, h: 300 },
    ];

    const positions = computeSpawnPositions(existing, 3, anchor);

    expect(positions).toHaveLength(3);
    for (const position of positions) {
      expect(existing.some((card) => overlapsExisting(position, card))).toBe(false);
    }
  });

  it("er deterministisk og muterer ikke input", () => {
    const existing: CardGeom[] = [{ x: 0, y: 0, w: 300, h: 200 }];
    const before = structuredClone(existing);

    const first = computeSpawnPositions(existing, 10, anchor);
    const second = computeSpawnPositions(existing, 10, anchor);

    expect(second).toEqual(first);
    expect(existing).toEqual(before);
  });

  it("returnerer tomt for count 0 og afviser count over schemaets maksimum", () => {
    expect(computeSpawnPositions([], 0, anchor)).toEqual([]);
    expect(() => computeSpawnPositions([], 11, anchor)).toThrow(/1.*10/u);
  });
});
