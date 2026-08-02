import { useEffect, useState, type CSSProperties } from "react";
import { setOcclusionReason } from "../browser/occlusion";
import { FROSTED_BACKDROP } from "../canvas/liquidGlass";

export type HudSessionState =
  | "idle"
  | "asleep"
  | "waking"
  | "awake"
  | "listening"
  | "processing"
  | "speaking"
  | "draining"
  | "sleeping";

export interface HudToolCall {
  name: string;
  arguments: Record<string, unknown>;
}

export type HudResolver =
  | { ok: true; card?: number; cards?: number[] }
  | { ok: false; reason: string };

export interface HudChainLine {
  name: string;
  ok: boolean;
  label: string;
}

export interface HudState {
  session: HudSessionState;
  transcript: string;
  responseText: string;
  tool: HudToolCall | null;
  resolver: HudResolver | null;
  error: string | null;
  chain: HudChainLine[] | null;
}

export interface HudView {
  sessionLabel: string;
  transcript: string;
  responseText: string;
  toolLabel: string | null;
  toolArguments: string | null;
  targetCards: number[];
  resolverLabel: string | null;
  statusText: string;
  chain: HudChainLine[] | null;
}

const SESSION_LABELS: Record<HudSessionState, string> = {
  idle: "Klar — hold tasten og tal",
  asleep: "Sovende",
  waking: "Vækker",
  awake: "Lytter",
  listening: "Lytter",
  processing: "Fortolker",
  speaking: "Svarer",
  draining: "afslutter svar…",
  sleeping: "Sover",
};

function explicitTargets(tool: HudToolCall | null): number[] {
  if (!tool) return [];
  const cards = tool.arguments.cards;
  if (Array.isArray(cards)) {
    return cards.filter((card): card is number => Number.isInteger(card) && Number(card) > 0);
  }
  const card = tool.arguments.card;
  return Number.isInteger(card) && Number(card) > 0 ? [Number(card)] : [];
}

export function deriveHudView(state: HudState, _now: number): HudView {
  const targetCards = explicitTargets(state.tool);
  let resolverLabel: string | null = null;
  if (state.resolver?.ok) {
    const cards = state.resolver.cards ??
      (state.resolver.card === undefined ? targetCards : [state.resolver.card]);
    resolverLabel = cards.length > 0 ? `Mål: kort ${cards.join(", ")}` : "Intet mål";
  } else if (state.resolver) {
    resolverLabel = `Blokeret: ${state.resolver.reason}`;
  }

  const statusText =
    state.error || state.responseText || SESSION_LABELS[state.session];

  return {
    sessionLabel: SESSION_LABELS[state.session],
    transcript: state.transcript,
    responseText: state.responseText,
    toolLabel: state.tool?.name ?? null,
    toolArguments: state.tool ? JSON.stringify(state.tool.arguments) : null,
    targetCards,
    resolverLabel,
    statusText,
    chain: state.chain,
  };
}

/**
 * API-lyd-kontakten laa her indtil 2026-07-29. Den satte kun App-state, som
 * ingen laeste: dens eneste forbruger er `realtime.ts`' `setApiAudio`, og den
 * REALTIME-motor monteres ikke — stemme-effekten i App bygger ubetinget en
 * pipeline-session. Kontakten er fjernet, ikke skjult. Genoptages realtime-
 * sporet, skal ledningen trækkes forfra og til den motor der faktisk kører.
 */
export function Hud({ state }: { state: HudState }) {
  const view = deriveHudView(state, 0);
  const active = !["idle", "asleep", "sleeping"].includes(state.session);
  const hasError = state.error !== null;
  const [panelOpen, setPanelOpen] = useState(false);

  // Occlusion-registrering (Task 7's gate, spec §4a): kun det UDFOLDEDE
  // panel dækker browser-webviews visuelt — den kompakte chip er for lille
  // til at kræve occlusion. Sammenfoldning OG unmount rydder årsagen, ellers
  // ville en stale "hud-panel"-occlusion overleve en fjernet Hud.
  useEffect(() => {
    setOcclusionReason("hud-panel", panelOpen);
    return () => setOcclusionReason("hud-panel", false);
  }, [panelOpen]);

  // Ejer-beslutning (2026-07-22, afløser topzone-valget fra 2026-07-20):
  // kompakt status er ALTID synlig i bund-docken nederst til højre (App.tsx'
  // data-bottom-dock, ved siden af Indstillinger), panelet er on-demand og
  // folder OPAD — det åbnes ALDRIG automatisk, heller
  // ikke ved en stående fejl. En fejl fremhæver i stedet chippen (fejlfarve
  // på statusprikken + kort label), så brugeren selv folder ud når han vil se
  // detaljen. aria-live-regionen er den altid-mountede chip; en remount kan
  // ellers sluge den første skærmlæser-announcement.
  const chipState: "error" | "active" | "idle" = hasError
    ? "error"
    : active
      ? "active"
      : "idle";
  const chipLabel = hasError ? "Fejl" : view.sessionLabel;

  return (
    <aside style={styles.liveRegion} aria-live="polite" aria-label="Voice-status">
      {/* Ikon-knap (ejer-beslutning 2026-07-22): ingen tekst-label — kun
          mikrofon-ikonet med statusprikken som badge i hjørnet. Selve
          status-TEKSTEN lever i aria-label (skærmlæser + tooltip) og i det
          udfoldede panel. */}
      <button
        type="button"
        data-hud-chip
        aria-expanded={panelOpen}
        aria-label={`Voice-status: ${chipLabel}. ${panelOpen ? "Luk" : "Åbn"} panel.`}
        title={chipLabel}
        style={{ ...styles.chip, ...(hasError ? styles.chipError : {}) }}
        onClick={() => setPanelOpen((open) => !open)}
      >
        <svg aria-hidden="true" width="14" height="14" viewBox="0 0 20 20" fill="none">
          <rect
            x="7.25"
            y="2.25"
            width="5.5"
            height="9.5"
            rx="2.75"
            stroke="currentColor"
            strokeWidth="1.25"
          />
          <path
            d="M4.75 9.75a5.25 5.25 0 0 0 10.5 0"
            stroke="currentColor"
            strokeWidth="1.25"
            strokeLinecap="round"
          />
          <path
            d="M10 15v2.75"
            stroke="currentColor"
            strokeWidth="1.25"
            strokeLinecap="round"
          />
        </svg>
        <span
          data-hud-dot
          data-state={chipState}
          aria-hidden="true"
          style={{
            ...styles.chipDot,
            ...(chipState === "active" ? styles.chipDotActive : {}),
            ...(chipState === "error" ? styles.chipDotError : {}),
          }}
        />
      </button>

      {panelOpen && (
        <div data-hud-panel style={styles.panel}>
          <div style={styles.rail} aria-hidden="true">
            <span style={{ ...styles.pulse, ...(active ? styles.pulseActive : {}) }} />
          </div>
          <div style={styles.body}>
            <header style={styles.header}>
              <strong style={styles.title}>VOICE</strong>
              <span style={{ ...styles.state, ...(active ? styles.stateActive : {}) }}>
                {view.sessionLabel}
              </span>
            </header>

            <div style={styles.status}>{view.statusText}</div>
            {view.transcript && (
              <div style={styles.line}>
                <span style={styles.key}>HØRT</span>
                <span>{view.transcript}</span>
              </div>
            )}
            {view.toolLabel && (
              <div style={styles.line}>
                <span style={styles.key}>TOOL</span>
                <code style={styles.code}>{view.toolLabel}</code>
                <code style={styles.args}>{view.toolArguments}</code>
              </div>
            )}
            {(view.targetCards.length > 0 || view.resolverLabel) && (
              <div style={styles.targets}>
                {view.targetCards.map((card) => (
                  <span key={card} style={styles.cardChip}>#{card}</span>
                ))}
                {view.resolverLabel && (
                  <span style={styles.resolver}>{view.resolverLabel}</span>
                )}
              </div>
            )}
            {state.chain && state.chain.length > 0 && (
              <div style={styles.chainList}>
                {state.chain.map((line, index) => (
                  <div key={index} style={styles.line}>
                    <span
                      style={{
                        ...styles.chainMark,
                        ...(line.ok ? {} : styles.chainMarkFailed),
                      }}
                      aria-hidden="true"
                    >
                      {line.ok ? "✓" : "✕"}
                    </span>
                    <code style={styles.code}>{line.name}</code>
                    <span style={styles.args}>{line.label}</span>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      )}
    </aside>
  );
}

const styles: Record<string, CSSProperties> = {
  // Selve aria-live-regionen: altid mounted. Ejer INGEN fixed-position
  // længere — den bor i App.tsx' bund-dock (data-bottom-dock, nederst til
  // højre ved siden af Indstillinger) og fylder kun chippen i rækken.
  // position:relative er panelets anker, så det udfoldede panel kan lægge
  // sig OVER chippen (bottom-ankret) uden at flytte dockens knap-række.
  liveRegion: {
    position: "relative",
    display: "flex",
    alignItems: "flex-end",
  },
  // Kompakt ikon-chip: altid renderet, uanset aktivitet — eneste indgang
  // til panelet. pointerEvents:auto punkterer forældrens none, så klik
  // rammer knappen og ikke canvas'et bagved. position:relative er anker
  // for statusprik-badgen i hjørnet.
  chip: {
    pointerEvents: "auto",
    position: "relative",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    border: "1px solid rgba(222, 241, 255, 0.2)",
    borderRadius: 999,
    padding: 7,
    background:
      "radial-gradient(90% 120% at 0% 0%, rgba(113, 180, 225, 0.16), transparent 58%), linear-gradient(145deg, rgba(24, 46, 70, 0.46), rgba(2, 9, 20, 0.4))",
    boxShadow:
      "0 10px 26px rgba(0, 7, 24, 0.4), inset 0 1px 0 rgba(242, 250, 255, 0.22)",
    color: "#9eafc1",
    cursor: "pointer",
    backdropFilter: FROSTED_BACKDROP,
    WebkitBackdropFilter: FROSTED_BACKDROP,
  },
  chipError: {
    borderColor: "rgba(224, 96, 88, 0.55)",
  },
  // Statusprik som badge oven på ikon-knappens hjørne.
  chipDot: {
    position: "absolute",
    top: -1,
    right: -1,
    width: 7,
    height: 7,
    borderRadius: 999,
    background: "#46505e",
    boxShadow: "none",
  },
  chipDotActive: {
    background: "#4dd6b7",
    boxShadow: "0 0 10px #4dd6b7",
  },
  chipDotError: {
    background: "#e06058",
    boxShadow: "0 0 10px #e06058",
  },
  // Udfoldet panel: bottom-ankret over chippen (docken sidder i bundhjørnet,
  // så detaljen folder OPAD); right:0 flugter panelets højrekant med dockens.
  panel: {
    pointerEvents: "auto",
    position: "absolute",
    bottom: "calc(100% + 8px)",
    right: 0,
    // Zonen, ikke vinduet: HUD'en bor i canvas-zonen (App.tsx' <main>), som er
    // 100vw MINUS rail'en. Uden fradraget klippes panelet af zonens
    // venstrekant ved smalle vinduer. Fallback 0px = montering uden AppShell.
    width: "min(440px, calc(100vw - var(--rail-width, 0px) - 36px))",
    maxHeight: "calc(100vh - 120px)",
    display: "flex",
    overflowX: "hidden",
    overflowY: "auto",
    border: "1px solid rgba(222, 241, 255, 0.2)",
    borderRadius: 14,
    background:
      "radial-gradient(90% 120% at 0% 0%, rgba(113, 180, 225, 0.16), transparent 58%), linear-gradient(145deg, rgba(24, 46, 70, 0.46), rgba(2, 9, 20, 0.4))",
    boxShadow:
      "0 24px 58px rgba(0, 7, 24, 0.5), 0 5px 16px rgba(0, 4, 15, 0.3), inset 0 1px 0 rgba(242, 250, 255, 0.26), inset 1px 0 0 rgba(201, 231, 255, 0.08)",
    color: "#d7dde7",
    fontFamily: '"Segoe UI", system-ui, sans-serif',
    backdropFilter: FROSTED_BACKDROP,
    WebkitBackdropFilter: FROSTED_BACKDROP,
  },
  rail: {
    width: 6,
    background: "rgba(12, 27, 43, 0.38)",
    display: "grid",
    placeItems: "center",
  },
  pulse: {
    width: 2,
    height: "54%",
    borderRadius: 999,
    background: "#46505e",
    boxShadow: "none",
  },
  pulseActive: {
    background: "#4dd6b7",
    boxShadow: "0 0 12px #4dd6b7",
  },
  body: { flex: 1, minWidth: 0, padding: "11px 13px 12px" },
  header: { display: "flex", alignItems: "center", gap: 9, marginBottom: 8 },
  title: { fontSize: 10, letterSpacing: 1.8, color: "#738094" },
  state: {
    padding: "2px 7px",
    border: "1px solid #3b4553",
    borderRadius: 999,
    color: "#8e99aa",
    fontSize: 11,
  },
  stateActive: { borderColor: "#277d6e", color: "#69e0c4" },
  status: { marginBottom: 8, color: "#eef2f8", fontSize: 13, fontWeight: 600 },
  line: { display: "flex", alignItems: "baseline", gap: 8, marginTop: 5, fontSize: 12 },
  key: { flex: "0 0 34px", color: "#697587", fontSize: 9, letterSpacing: 1.1 },
  code: { color: "#7dd3fc", fontFamily: '"Cascadia Mono", monospace' },
  args: {
    minWidth: 0,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    color: "#8793a5",
    fontFamily: '"Cascadia Mono", monospace',
  },
  targets: { display: "flex", alignItems: "center", gap: 6, marginTop: 8 },
  chainList: { marginTop: 8, display: "flex", flexDirection: "column", gap: 2 },
  chainMark: { flex: "0 0 14px", color: "#4dd6b7", fontSize: 11 },
  chainMarkFailed: { color: "#e06058" },
  cardChip: {
    padding: "2px 7px",
    border: "1px solid #3f6d88",
    borderRadius: 4,
    background: "#172632",
    color: "#8bd7ff",
    fontFamily: '"Cascadia Mono", monospace',
    fontSize: 11,
  },
  resolver: { color: "#8793a5", fontSize: 11 },
};

export default Hud;
