// RENE funktioner: geometri for cast-straalen (spec 2026-07-22). Ingen DOM,
// ingen sideeffekter — testes isoleret med vitest, som colors.ts.

export type CastPoint = { x: number; y: number };

/** Normal-offset for bezier-kontrolpunktet som andel af rutelaengden. */
export const CAST_CURVE_K = 0.18;

/**
 * Kvadratisk bezier-kontrolpunkt: midtpunktet forskudt vinkelret paa ruten,
 * altid mod skaerm-opad (negativ y), saa straalen buer over kortene i stedet
 * for ind i orb-dokken. p0 == p2 degenererer til punktet selv.
 */
export function castControlPoint(
  p0: CastPoint,
  p2: CastPoint,
  k = CAST_CURVE_K,
): CastPoint {
  const mx = (p0.x + p2.x) / 2;
  const my = (p0.y + p2.y) / 2;
  const dx = p2.x - p0.x;
  const dy = p2.y - p0.y;
  const length = Math.hypot(dx, dy);
  if (length === 0) return { x: p0.x, y: p0.y };
  let nx = -dy / length;
  let ny = dx / length;
  if (ny > 0) {
    nx = -nx;
    ny = -ny;
  }
  return { x: mx + nx * length * k, y: my + ny * length * k };
}

/** SVG-path-streng for straalen: M p0 Q cp p2. */
export function castPathD(p0: CastPoint, cp: CastPoint, p2: CastPoint): string {
  return `M ${p0.x} ${p0.y} Q ${cp.x} ${cp.y} ${p2.x} ${p2.y}`;
}

/**
 * Tilnaermet kurvelaengde for den kvadratiske bezier — gennemsnit af korden
 * og kontrolpolygonen. Rigelig praecision til dash-sweepet, og deterministisk
 * uden SVG-DOM (happy-dom har ikke getTotalLength).
 */
export function castApproxLength(
  p0: CastPoint,
  cp: CastPoint,
  p2: CastPoint,
): number {
  const chord = Math.hypot(p2.x - p0.x, p2.y - p0.y);
  const polygon =
    Math.hypot(cp.x - p0.x, cp.y - p0.y) + Math.hypot(p2.x - cp.x, p2.y - cp.y);
  return (chord + polygon) / 2;
}
