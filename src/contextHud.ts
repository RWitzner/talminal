// Per-kort context-snapshots (context v1, se docs/superpowers/specs/
// 2026-07-22-card-context-badge-design.md): skrevet af statusline-tap'en for
// kort-sessioner, læst via read_context_snapshots (context_hud.rs). Ren
// join-logik her — polling bor i CanvasSurface, visning i ContextBadge.

/** Spejler Rust-sidens ContextSnapshot (context_hud.rs, serde camelCase) 1:1. */
export interface ContextSnapshot {
  version: number;
  writtenAt: string;
  cardName: string;
  runId: string | null;
  sessionId: string | null;
  cwd: string;
  usedPercent: number;
  windowSize: number | null;
  modelDisplayName: string | null;
}

/** Spec-join: kortnavn OG cwd skal matche — cwd'en er værnet mod teoretisk
 *  navnekollision hvis to canvas-instanser i hver sit projekt deler den
 *  globale base. Klamper defensivt (Rust-læseren klamper også; bæltet OG
 *  selerne er billige). */
export function contextPercentFor(
  snapshots: ContextSnapshot[],
  cardName: string,
  cwd: string,
): number | null {
  const match = snapshots.find(
    (candidate) => candidate.cardName === cardName && candidate.cwd === cwd,
  );
  if (!match || !Number.isFinite(match.usedPercent)) return null;
  return Math.max(0, Math.min(100, match.usedPercent));
}
