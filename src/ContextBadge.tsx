import type { CSSProperties } from "react";
import { barColor } from "./UsageHud";

/** Per-kort context-badge (spec: docs/superpowers/specs/
 *  2026-07-22-card-context-badge-design.md). Ren visning: procenten er
 *  allerede joinet/klampet i CanvasSurface (contextPercentFor) — null
 *  betyder "intet snapshot for kortet" og renderer ingenting. At badgen kun
 *  optræder på kort med context-data ER Claude-filtret: kun kort-CC-sessioner
 *  skriver context-filer (TALMINAL_SESSION_ID-gaten i tap'en). */
export function ContextBadge({ percent }: { percent: number | null }) {
  if (percent == null) return null;
  const rounded = Math.round(percent);
  return (
    <span
      data-context-badge
      style={{ ...styles.badge, color: barColor(percent) }}
      title={`Context: ${rounded}% af vinduet brugt`}
    >
      CTX {rounded}%
    </span>
  );
}

const styles: Record<string, CSSProperties> = {
  badge: {
    flex: "0 0 auto",
    fontSize: 10,
    fontWeight: 600,
    padding: "1px 6px",
    borderRadius: 999,
    border: "1px solid rgba(222, 241, 255, 0.18)",
    background: "rgba(2, 9, 20, 0.35)",
  },
};

export default ContextBadge;
