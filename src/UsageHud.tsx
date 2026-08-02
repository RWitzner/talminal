import { useEffect, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FROSTED_BACKDROP } from "./canvas/liquidGlass";
import type { CardInfo } from "./types";

/** Spejler Rust-sidens UsageSnapshot (usage_hud.rs, serde camelCase) 1:1 —
 *  som igen spejler usage.json-kontrakten v1 fra statusline-tap'en. */
export interface UsageSnapshot {
  version: number;
  writtenAt: string;
  fiveHourPercent: number;
  fiveHourResetsAt: string | null;
  weeklyPercent: number | null;
  weeklyResetsAt: string | null;
  sessionId?: string | null;
}

export interface UsageBar {
  label: string;
  percent: number;
  resetLabel: string | null;
}

export type UsageView =
  | { kind: "data"; stale: boolean; bars: UsageBar[] }
  | { kind: "empty" };

export const POLL_INTERVAL_MS = 30_000;
const STALE_AFTER_MS = 15 * 60 * 1000;
const DEAD_AFTER_MS = 60 * 60 * 1000;

/** Nedtælling på dansk: "42m", "3t42m", "6d13t". null når resettet er passeret/ukendt. */
export function formatReset(resetIso: string | null, nowMs: number): string | null {
  if (!resetIso) return null;
  const resetMs = Date.parse(resetIso);
  if (Number.isNaN(resetMs) || resetMs <= nowMs) return null;
  const totalMinutes = Math.floor((resetMs - nowMs) / 60_000);
  const totalHours = Math.floor(totalMinutes / 60);
  const days = Math.floor(totalHours / 24);
  if (days > 0) return `${days}d${totalHours % 24}t`;
  if (totalHours === 0) return `${totalMinutes}m`;
  return `${totalHours}t${totalMinutes % 60}m`;
}

export function deriveUsageView(snapshot: UsageSnapshot | null, nowMs: number): UsageView {
  if (!snapshot) return { kind: "empty" };
  const age = nowMs - Date.parse(snapshot.writtenAt);
  if (Number.isNaN(age) || age > DEAD_AFTER_MS) return { kind: "empty" };
  const bars: UsageBar[] = [
    {
      label: "5t",
      percent: snapshot.fiveHourPercent,
      resetLabel: formatReset(snapshot.fiveHourResetsAt, nowMs),
    },
  ];
  if (snapshot.weeklyPercent != null) {
    bars.push({
      label: "uge",
      percent: snapshot.weeklyPercent,
      resetLabel: formatReset(snapshot.weeklyResetsAt, nowMs),
    });
  }
  return { kind: "data", stale: age > STALE_AFTER_MS, bars };
}

/** Samme tærskler (70/90) som den eksterne statusline-HUD, i canvas-paletten.
 *  Deles med ContextBadge (spec: genbrug, ikke duplikering). */
export function barColor(percent: number): string {
  if (percent >= 90) return "#e06058";
  if (percent >= 70) return "#e0b25c";
  return "#4dd6b7";
}

export function UsageHud({ cards }: { cards: CardInfo[] | null }) {
  const visible = (cards ?? []).some((card) => card.kind === "terminal" && card.running);
  const [snapshot, setSnapshot] = useState<UsageSnapshot | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());

  // Poll kun mens synlig — usynligt HUD koster nul. Samme tick driver både
  // snapshot-hentning og nedtællings-re-render (nowMs).
  useEffect(() => {
    if (!visible) return;
    let disposed = false;
    const poll = async () => {
      try {
        const next = await invoke<UsageSnapshot | null>("read_usage_snapshot");
        if (!disposed) setSnapshot(next);
      } catch {
        if (!disposed) setSnapshot(null);
      }
      if (!disposed) setNowMs(Date.now());
    };
    void poll();
    const interval = setInterval(() => void poll(), POLL_INTERVAL_MS);
    return () => {
      disposed = true;
      clearInterval(interval);
    };
  }, [visible]);

  const view = deriveUsageView(snapshot, nowMs);

  return (
    <div
      data-usage-hud
      aria-hidden={!visible}
      aria-label="Claude-forbrug"
      style={{ ...styles.dock, opacity: visible ? 1 : 0 }}
    >
      {view.kind === "data" ? (
        view.bars.map((bar) => (
          <div key={bar.label} data-usage-bar={bar.label} style={styles.row}>
            <span style={styles.label}>{bar.label}</span>
            <span style={styles.track} aria-hidden="true">
              <span
                style={{
                  ...styles.fill,
                  width: `${bar.percent}%`,
                  background: barColor(bar.percent),
                  ...(view.stale ? styles.fillStale : {}),
                }}
              />
            </span>
            <span style={{ ...styles.percent, ...(view.stale ? styles.percentStale : {}) }}>
              {Math.round(bar.percent)}%{view.stale ? "*" : ""}
            </span>
            {bar.resetLabel && <span style={styles.reset}>({bar.resetLabel})</span>}
          </div>
        ))
      ) : (
        <div data-usage-bar="tom" style={styles.row}>
          <span style={styles.label}>5t</span>
          <span style={styles.percent}>—</span>
        </div>
      )}
    </div>
  );
}

const styles: Record<string, CSSProperties> = {
  // Bor i orb-dockens bundbånd (ORB_DOCK_CLEARANCE=88, zIndex 40 — Orb.tsx),
  // venstre side: "til venstre for voice-visualen" (ejer-retning 2026-07-21).
  // Chip-højden (2 rækker ≈ 49px inkl. padding) holder sig indenfor båndet
  // ved bottom 24. pointerEvents:none = ren aflæsning, ingen interaktion →
  // ingen occlusion-registrering (samme begrundelse som voice-HUD'ens
  // kompakte chip).
  // `absolute`, ikke `fixed`: HUD'en monteres inde i App'ens <main>, som efter
  // AppShell-opdelingen er canvas-zonens position:relative-boks (App.tsx:981).
  // Med fixed ville chippen ligge i vinduets bundvenstre hjørne — dvs. oven på
  // rail'en. Ingen læser dens koordinater udefra (pointerEvents:none, ingen
  // occlusion-registrering, ingen getBoundingClientRect), så flytningen har
  // ingen anden virkning end den tilsigtede.
  dock: {
    position: "absolute",
    left: 18,
    bottom: 24,
    zIndex: 40,
    pointerEvents: "none",
    display: "flex",
    flexDirection: "column",
    gap: 5,
    padding: "8px 12px",
    border: "1px solid rgba(222, 241, 255, 0.2)",
    borderRadius: 12,
    background:
      "radial-gradient(90% 120% at 0% 0%, rgba(113, 180, 225, 0.16), transparent 58%), linear-gradient(145deg, rgba(24, 46, 70, 0.46), rgba(2, 9, 20, 0.4))",
    boxShadow:
      "0 10px 26px rgba(0, 7, 24, 0.4), inset 0 1px 0 rgba(242, 250, 255, 0.22)",
    color: "#d7dde7",
    fontFamily: '"Segoe UI", system-ui, sans-serif',
    fontSize: 11,
    backdropFilter: FROSTED_BACKDROP,
    WebkitBackdropFilter: FROSTED_BACKDROP,
    transition: "opacity 0.4s ease",
  },
  row: { display: "flex", alignItems: "center", gap: 7 },
  label: {
    flex: "0 0 24px",
    color: "#738094",
    fontSize: 9,
    letterSpacing: 1.1,
    textTransform: "uppercase",
  },
  track: {
    width: 72,
    height: 4,
    borderRadius: 999,
    background: "rgba(222, 241, 255, 0.14)",
    overflow: "hidden",
    display: "inline-block",
  },
  fill: { display: "block", height: "100%", borderRadius: 999 },
  fillStale: { opacity: 0.45 },
  percent: { color: "#eef2f8", fontWeight: 600, minWidth: 34, textAlign: "right" },
  percentStale: { color: "#8e99aa" },
  reset: { color: "#7f8a9b", fontSize: 10 },
};

export default UsageHud;
