export const SPAWN_CARD_WIDTH = 960;
export const SPAWN_CARD_HEIGHT = 640;
const GRID_GAP = 48;

export interface CardGeom {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Position {
  x: number;
  y: number;
}

function intersects(position: Position, geometry: CardGeom): boolean {
  return !(
    position.x + SPAWN_CARD_WIDTH <= geometry.x ||
    geometry.x + geometry.w <= position.x ||
    position.y + SPAWN_CARD_HEIGHT <= geometry.y ||
    geometry.y + geometry.h <= position.y
  );
}

function* gridOffsets(): Generator<{ column: number; row: number }> {
  yield { column: 0, row: 0 };
  for (let ring = 1; ; ring += 1) {
    const offsets: Array<{ column: number; row: number }> = [];
    for (let row = -ring; row <= ring; row += 1) {
      for (let column = -ring; column <= ring; column += 1) {
        if (Math.max(Math.abs(column), Math.abs(row)) === ring) {
          offsets.push({ column, row });
        }
      }
    }
    offsets.sort(
      (left, right) =>
        Math.hypot(left.column, left.row) - Math.hypot(right.column, right.row) ||
        left.row - right.row ||
        left.column - right.column,
    );
    yield* offsets;
  }
}

/**
 * Deterministic world-space spawn positions around the anchor — the world
 * CENTER of the visible view (CanvasController.getSpawnAnchor), NOT the
 * viewport corner. The first card is centered on the anchor.
 * Existing cards and positions selected earlier in the same batch are occupied.
 */
export function computeSpawnPositions(
  existing: readonly CardGeom[],
  count: number,
  anchor: Position,
): Position[] {
  if (!Number.isInteger(count) || count < 0 || count > 10) {
    throw new Error("Spawn count must be an integer between 1 and 10");
  }
  if (count === 0) return [];

  const occupied = existing.map((geometry) => ({ ...geometry }));
  const positions: Position[] = [];
  const stepX = SPAWN_CARD_WIDTH + GRID_GAP;
  const stepY = SPAWN_CARD_HEIGHT + GRID_GAP;

  for (const offset of gridOffsets()) {
    const position = {
      x: anchor.x - SPAWN_CARD_WIDTH / 2 + offset.column * stepX,
      y: anchor.y - SPAWN_CARD_HEIGHT / 2 + offset.row * stepY,
    };
    if (occupied.some((geometry) => intersects(position, geometry))) continue;

    positions.push(position);
    occupied.push({
      ...position,
      w: SPAWN_CARD_WIDTH,
      h: SPAWN_CARD_HEIGHT,
    });
    if (positions.length === count) return positions;
  }

  return positions;
}
