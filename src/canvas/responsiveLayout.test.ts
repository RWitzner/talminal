import { describe, expect, it } from "vitest";
import {
  computeResponsiveGridSpacing,
  computeResponsiveTileLayout,
  type ResponsiveTileLayout,
} from "./responsiveLayout";

function expectCompleteNonOverlappingGrid(
  layout: ResponsiveTileLayout,
  count: number,
): void {
  expect(layout.tiles).toHaveLength(count);
  const occupancy = Array.from(
    { length: layout.rows },
    () => Array<number>(layout.columns).fill(0),
  );

  layout.tiles.forEach((tile, tileIndex) => {
    expect(tile.column).toBeGreaterThanOrEqual(1);
    expect(tile.row).toBeGreaterThanOrEqual(1);
    expect(tile.columnSpan).toBeGreaterThanOrEqual(1);
    expect(tile.rowSpan).toBeGreaterThanOrEqual(1);
    expect(tile.column + tile.columnSpan - 1).toBeLessThanOrEqual(layout.columns);
    expect(tile.row + tile.rowSpan - 1).toBeLessThanOrEqual(layout.rows);

    for (let row = tile.row - 1; row < tile.row - 1 + tile.rowSpan; row += 1) {
      for (
        let column = tile.column - 1;
        column < tile.column - 1 + tile.columnSpan;
        column += 1
      ) {
        expect(occupancy[row][column]).toBe(0);
        occupancy[row][column] = tileIndex + 1;
      }
    }
  });

  for (const row of occupancy) {
    for (const owner of row) expect(owner).toBeGreaterThan(0);
  }
}

describe("computeResponsiveTileLayout", () => {
  it("returnerer et tomt layout for et tomt canvas", () => {
    expect(computeResponsiveTileLayout(0, { width: 1400, height: 900 })).toEqual({
      columns: 0,
      rows: 0,
      tiles: [],
    });
  });

  it("lader ét kort fylde hele gridet", () => {
    const layout = computeResponsiveTileLayout(1, {
      width: 1400,
      height: 900,
    });
    expect(layout).toEqual({
      columns: 1,
      rows: 1,
      tiles: [{ column: 1, row: 1, columnSpan: 1, rowSpan: 1 }],
    });
  });

  it("lægger to kort ved siden af hinanden i landscape og over hinanden i portrait", () => {
    expect(
      computeResponsiveTileLayout(2, { width: 1400, height: 900 }),
    ).toMatchObject({ columns: 2, rows: 1 });
    expect(
      computeResponsiveTileLayout(2, { width: 640, height: 900 }),
    ).toMatchObject({ columns: 1, rows: 2 });
  });

  it("lægger tre kort som venstre hero og to stablede kort til højre", () => {
    expect(
      computeResponsiveTileLayout(3, { width: 1400, height: 900 }),
    ).toEqual({
      columns: 2,
      rows: 2,
      tiles: [
        { column: 1, row: 1, columnSpan: 1, rowSpan: 2 },
        { column: 2, row: 1, columnSpan: 1, rowSpan: 1 },
        { column: 2, row: 2, columnSpan: 1, rowSpan: 1 },
      ],
    });
  });

  it("lægger fire kort i et 2x2-grid", () => {
    expect(
      computeResponsiveTileLayout(4, { width: 1400, height: 900 }),
    ).toEqual({
      columns: 2,
      rows: 2,
      tiles: [
        { column: 1, row: 1, columnSpan: 1, rowSpan: 1 },
        { column: 2, row: 1, columnSpan: 1, rowSpan: 1 },
        { column: 1, row: 2, columnSpan: 1, rowSpan: 1 },
        { column: 2, row: 2, columnSpan: 1, rowSpan: 1 },
      ],
    });
  });

  for (const size of [
    { width: 1920, height: 1080 },
    { width: 900, height: 600 },
    { width: 640, height: 900 },
    { width: 120, height: 80 },
  ]) {
    it(`dækker hele gridet uden overlap for 5-20 kort ved ${size.width}x${size.height}`, () => {
      for (let count = 5; count <= 20; count += 1) {
        expectCompleteNonOverlappingGrid(
          computeResponsiveTileLayout(count, size),
          count,
        );
      }
    });
  }

  it("ændrer den balancerede topologi, når canvasets aspect ratio ændres", () => {
    const landscape = computeResponsiveTileLayout(8, {
      width: 1800,
      height: 700,
    });
    const portrait = computeResponsiveTileLayout(8, {
      width: 600,
      height: 1000,
    });

    expect({ columns: landscape.columns, rows: landscape.rows }).not.toEqual({
      columns: portrait.columns,
      rows: portrait.rows,
    });
  });

  it("reducerer gap og padding, så gutters aldrig kan skubbe gridet uden for et lille canvas", () => {
    for (const size of [
      { width: 120, height: 80 },
      { width: 40, height: 30 },
      { width: 5, height: 3 },
    ]) {
      for (let count = 1; count <= 20; count += 1) {
        const layout = computeResponsiveTileLayout(count, size);
        const spacing = computeResponsiveGridSpacing(layout, size);
        const horizontalGutters =
          2 * spacing.padding + Math.max(0, layout.columns - 1) * spacing.gap;
        const verticalGutters =
          2 * spacing.padding + Math.max(0, layout.rows - 1) * spacing.gap;

        expect(spacing.gap).toBeGreaterThanOrEqual(0);
        expect(spacing.padding).toBeGreaterThanOrEqual(0);
        expect(horizontalGutters).toBeLessThanOrEqual(size.width);
        expect(verticalGutters).toBeLessThanOrEqual(size.height);
      }
    }
  });

  it("er deterministisk og afviser ugyldige counts", () => {
    const size = { width: 1377, height: 811 };
    expect(computeResponsiveTileLayout(10, size)).toEqual(
      computeResponsiveTileLayout(10, size),
    );
    expect(() => computeResponsiveTileLayout(-1, size)).toThrow(
      /non-negative integer/u,
    );
    expect(() => computeResponsiveTileLayout(1.5, size)).toThrow(
      /non-negative integer/u,
    );
  });
});
