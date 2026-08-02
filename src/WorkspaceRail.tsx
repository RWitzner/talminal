import { useEffect, useRef, useState, type CSSProperties } from "react";
import { FROSTED_BACKDROP } from "./canvas/liquidGlass";
import { TOPBAR_HEIGHT, TOPBAR_TOP } from "./canvas/responsiveLayout";
import { rowLabel, visibleWorkspaces, type WorkspaceSummary } from "./workspaces";

/** Hvor laenge en rad maa staa "klikket, men ikke besvaret". Samme tal som
 *  protokollens `LAUNCH_DEADLINE_SECS` (workspaces/mod.rs). */
const PENDING_TIMEOUT_MS = 8_000;

export interface WorkspaceRailProps {
  workspaces: WorkspaceSummary[];
  /** Kontrolleret af App'en (ikke intern state): "Vis skjulte" skal kunne
   *  saettes udefra — bl.a. naar default-workspacet hentes frem igen. */
  showHidden: boolean;
  onActivate(slug: string): void;
  onClose(slug: string): void;
  /** Rowens "Fjern"/"Hent frem" — `.hidden`-sidecaren, ikke filteret. */
  onToggleHidden(slug: string, hidden: boolean): void;
  onAdd(): void;
  /** "Vis skjulte"-filteret. Valgfri, fordi den frosne prop-kontrakt fra planens
   *  test kun kraever de fem ovenstaaende; uden handler er knappen inaktiv. */
  onShowHiddenChange?(next: boolean): void;
  /**
   * Aabner indstillings-vinduet. Valgfri af samme grund som ovenstaaende:
   * kontrakten fra planens test maa ikke braekke af en tilfoejelse.
   *
   * Vinduet tegnes IKKE herfra. Rail-zonen i AppShell er 208 px med
   * `overflow: hidden`, saa en flade der foldede ud herfra ville blive
   * klippet ved zonens kant. App'en ejer tilstanden og tegner vinduet i
   * canvas-zonen; rail'en bidrager kun med knappen.
   */
  onOpenSettings?(): void;
  /** Er vinduet aabent? Kun til `aria-expanded` og den aktive markering. */
  settingsOpen?: boolean;
}

/**
 * Workspace-rail'en: én post pr. projekt, med prik ved `attention`, ✕ og
 * "Fjern" som rigtige knapper i tab-raekkefoelgen, og "+ Tilføj projekt" /
 * "Vis skjulte" nederst (spec §5.2).
 *
 * Tre ting der ser ud som pynt, men er krav:
 *  - Handlingerne er `opacity: 0` indtil hover/`:focus-within` — IKKE
 *    `display:none`. Sidstnaevnte ville tage dem ud af tab-raekkefoelgen og
 *    goere rail'en utilgaengelig med tastatur.
 *  - Vinduet er `decorations: false` OG `resizable: true`. Rail'en baerer
 *    derfor selv en drag-flade (topbaren daekker efter Task 9 kun
 *    canvas-zonen) — men den ligger i et BAAND med inset, ikke paa roden,
 *    saa vinduets resize-kanter forbliver resize-kanter. Se `styles.dragBand`.
 *  - Posten er en `<li>` med klik-handler OG en indre aabne-knap: klik paa
 *    raden (mus) og Enter paa knappen (tastatur) fører begge til `onActivate`,
 *    uden at nestne en knap i en knap.
 */
export function WorkspaceRail({
  workspaces,
  showHidden,
  onActivate,
  onClose,
  onToggleHidden,
  onAdd,
  onShowHiddenChange,
  onOpenSettings,
  settingsOpen = false,
}: WorkspaceRailProps) {
  const rows = visibleWorkspaces(workspaces, showHidden);

  // Lokal single-flight. Backendens `state === "starting"` er FOERST tilbage
  // naar `workspaces-changed` er landet; indtil da staar posten stadig som
  // "stopped". Uden denne slug ville et helt normalt dobbeltklik sende to
  // `activate_workspace`, og det andet ville fejle — brugeren fik en
  // fejlnotits for at klikke to gange.
  const [pending, setPending] = useState<string | null>(null);
  // Listen som klikket blev afgivet mod. Se ryddningen nedenfor.
  const listAtActivate = useRef<WorkspaceSummary[] | null>(null);

  useEffect(() => {
    if (pending === null) return;
    const p = workspaces.find((x) => x.slug === pending);
    // Ryddes paa to signaler, ikke ét:
    //  1. Backenden har talt om netop denne post (alt andet end "stopped" —
    //     bevidst bredere end starting/running: fejler spawn'et, eller
    //     forsvinder posten, skal raden kunne klikkes igen).
    //  2. Der er kommet en NY liste overhovedet. Afvises kaldet uden at
    //     aendre state, ville (1) alene laase posten resten af sessionen.
    if (p === undefined || p.state !== "stopped" || workspaces !== listAtActivate.current) {
      setPending(null);
    }
  }, [workspaces, pending]);

  // SIDSTE UDVEJ. Begge signaler ovenfor forudsaetter at der KOMMER en ny liste.
  // Afvises `activate_workspace` — ukendt slug, `current_exe()` fejlede, spawn
  // slog fejl — aendrer backenden ingenting, og badge-tick'et emitter kun VED
  // DIFF. Saa kommer der ingen ny liste, og raden ville staa `aria-busy` og
  // uklikkbar resten af sessionen, mens brugeren fik én fejlnotits og ellers
  // ingen forklaring. Fristen er den samme som protokollens `launch_deadline`
  // (8 s): naar backenden har opgivet at vente, skal raden ogsaa vaere fri.
  useEffect(() => {
    if (pending === null) return;
    const frist = setTimeout(() => setPending(null), PENDING_TIMEOUT_MS);
    return () => clearTimeout(frist);
  }, [pending]);

  const activate = (slug: string, trackPending = true) => {
    if (trackPending) {
      listAtActivate.current = workspaces;
      setPending(slug);
    }
    onActivate(slug);
  };

  return (
    <nav data-workspace-rail aria-label="Projekter" style={styles.rail}>
      <style>{railCss}</style>

      {/* Drag-fladen. Topbaren ligger efter Task 9 inde i canvas-zonen og
          starter ved x = rail-bredden + 18, saa uden dette baand kan vinduet
          ikke traekkes i sine oeverste 208 px til venstre. Baandet — og ikke
          rod-nav'en — baerer drag'en, fordi nav'en er inset:0 i en fixed
          shell og dermed ville laegge drag-fladen PAA vinduets venstre kant
          og de to venstre hjoerner. Se `styles.dragBand`. */}
      <span
        data-workspace-drag-region
        data-tauri-drag-region="true"
        aria-hidden="true"
        style={styles.dragBand}
      />

      <header style={styles.header}>
        <span style={styles.headerTitle}>Projekter</span>
      </header>

      {workspaces.length === 0 ? (
        <div data-workspace-empty="first-run" style={styles.empty}>
          Ingen projekter endnu. Tilføj dit første projekt med knappen nedenfor.
        </div>
      ) : rows.length === 0 ? (
        // IKKE foerste gang: der ER projekter, de er bare alle fjernet fra
        // listen. Foerste-gangs-opfordringen ville her vaere direkte usand og
        // peger paa den forkerte knap.
        <div data-workspace-empty="all-hidden" style={styles.empty}>
          Alle dine projekter er fjernet fra listen. Tryk “Vis skjulte” for at
          hente dem frem igen.
        </div>
      ) : (
        <ul data-workspace-list style={styles.list}>
          {rows.map((w) => (
            <WorkspaceRow
              key={w.slug}
              workspace={w}
              pending={pending === w.slug}
              onActivate={activate}
              onClose={onClose}
              onToggleHidden={onToggleHidden}
            />
          ))}
        </ul>
      )}

      <div style={styles.footer}>
        <button
          type="button"
          data-workspace-add
          style={styles.addButton}
          onClick={onAdd}
        >
          + Tilføj projekt
        </button>
        <div style={styles.footerRow}>
          <button
            type="button"
            data-workspace-show-hidden
            aria-pressed={showHidden}
            style={{
              ...styles.ghostButton,
              ...(showHidden ? styles.ghostButtonOn : null),
            }}
            onClick={() => onShowHiddenChange?.(!showHidden)}
          >
            Vis skjulte
          </button>
          <button
            type="button"
            data-workspace-settings
            aria-label="Indstillinger"
            title="Indstillinger"
            aria-haspopup="dialog"
            aria-expanded={settingsOpen}
            style={{
              ...styles.settingsButton,
              ...(settingsOpen ? styles.settingsButtonOn : null),
            }}
            onClick={() => onOpenSettings?.()}
          >
            <svg
              aria-hidden="true"
              width="14"
              height="14"
              viewBox="0 0 20 20"
              fill="none"
            >
              <path
                d="M8.5 2.5h3l.55 2.02c.4.17.78.39 1.13.65l2-.56 1.5 2.6-1.45 1.47c.05.43.05.87 0 1.3l1.45 1.48-1.5 2.59-2-.55c-.35.26-.73.47-1.13.64l-.55 2.03h-3l-.55-2.03a6.2 6.2 0 0 1-1.13-.64l-2 .55-1.5-2.6 1.45-1.47a5.8 5.8 0 0 1 0-1.3L3.32 7.2l1.5-2.59 2 .56c.35-.26.73-.48 1.13-.65L8.5 2.5Z"
                stroke="currentColor"
                strokeWidth="1.25"
                strokeLinejoin="round"
              />
              <circle
                cx="10"
                cy="9.34"
                r="2.25"
                stroke="currentColor"
                strokeWidth="1.25"
              />
            </svg>
          </button>
        </div>
      </div>
    </nav>
  );
}

function WorkspaceRow({
  workspace,
  pending,
  onActivate,
  onClose,
  onToggleHidden,
}: {
  workspace: WorkspaceSummary;
  /** Klikket er afgivet, men backendens `starting` er ikke ekkoet endnu. */
  pending: boolean;
  onActivate(slug: string, trackPending?: boolean): void;
  onClose(slug: string): void;
  onToggleHidden(slug: string, hidden: boolean): void;
}) {
  const w = workspace;
  const label = rowLabel(w);
  // Single-flight: en post der allerede starter maa ikke kunne bestille et
  // spawn til. Klikket sluges, og `aria-busy` fortaeller hvorfor.
  const starting = w.state === "starting";
  const busy = starting || pending;

  return (
    <li
      data-workspace-row={w.slug}
      data-state={w.state}
      aria-current={w.is_active ? "true" : undefined}
      aria-busy={busy ? "true" : undefined}
      style={{
        ...styles.row,
        ...(w.is_active ? styles.rowActive : null),
        ...(w.state === "stopped" || w.hidden ? styles.rowDim : null),
      }}
      onClick={() => {
        // Den aktive række er navigationens destination, ikke en refresh-knap.
        // Vi delegerer stadig til backendens autoritative gate, fordi rail-listen
        // kan være stale under et meget hurtigt A→B→A-skift. Men vi sætter ikke
        // 8 s lokal pending-state for den forventede no-op.
        if (!busy) onActivate(w.slug, !w.is_active);
      }}
    >
      <button
        type="button"
        data-workspace-open={w.slug}
        // Fuld sti paa det trunkerede navn (spec §5.2). Uden `root` er
        // navnet selv det bedste vi har.
        title={w.root ?? label.title}
        // `starting`, ikke `busy`: disables knappen allerede paa det lokale
        // klik, flytter browseren fokus til <body> midt i en tastaturbrugers
        // egen aktivering. Det lokale pending sluger klikket — det behoever
        // ikke ogsaa at rive fokus vaek.
        disabled={starting}
        style={styles.open}
      >
        <span
          aria-hidden="true"
          data-attention-kind={effectiveAttentionKind(w)}
          style={{ ...styles.dot, ...dotStyle(w) }}
        />
        <span style={styles.text}>
          <span data-workspace-name={w.slug} style={styles.name}>
            {label.title}
          </span>
          {label.subtitle !== null && (
            <span style={styles.subtitle}>{label.subtitle}</span>
          )}
        </span>
        {w.cards > 0 && (
          <span aria-hidden="true" style={styles.count}>
            {w.cards}
          </span>
        )}
        {/* Tekst-aekvivalenten til prikken og til alt det farven siger. */}
        <span className="workspace-rail-sr">{statusText(w)}</span>
      </button>

      <span data-workspace-actions style={styles.actions}>
        <button
          type="button"
          data-workspace-close={w.slug}
          aria-label={`Luk ${label.title}`}
          title={`Luk ${label.title}`}
          style={styles.iconButton}
          onClick={(event) => {
            // Uden dette ville klikket boble til raden og OGSAA aktivere det
            // workspace brugeren netop bad om at lukke.
            event.stopPropagation();
            onClose(w.slug);
          }}
        >
          ✕
        </button>
        <button
          type="button"
          data-workspace-hide={w.slug}
          title={
            w.hidden
              ? `Hent ${label.title} tilbage i listen`
              : `Fjern ${label.title} fra listen (sletter intet)`
          }
          style={styles.textButton}
          onClick={(event) => {
            event.stopPropagation();
            onToggleHidden(w.slug, !w.hidden);
          }}
        >
          {w.hidden ? "Hent frem" : "Fjern"}
        </button>
      </span>
    </li>
  );
}

function statusText(w: WorkspaceSummary): string {
  const parts: string[] = [];
  if (w.defect) parts.push("kan ikke laeses");
  const attentionKind = effectiveAttentionKind(w);
  if (attentionKind === "needs_you") parts.push("kræver din handling");
  else if (attentionKind === "done_unread") parts.push("venter på dig");
  else if (w.state === "starting") parts.push("starter");
  else if (w.state === "running") parts.push("kører");
  else if (w.state === "failed") parts.push("kunne ikke startes");
  else parts.push("ikke åben");
  if (w.cards > 0) parts.push(`${w.cards} kort`);
  if (w.is_active) parts.push("vises nu");
  if (w.hidden) parts.push("skjult");
  return parts.join(", ");
}

function dotStyle(w: WorkspaceSummary): CSSProperties {
  const attentionKind = effectiveAttentionKind(w);
  if (attentionKind === "needs_you") {
    return {
      background: "#ff665c",
      boxShadow:
        "0 0 0 2px rgba(255, 102, 92, 0.18), 0 0 14px rgba(255, 102, 92, 0.9)",
    };
  }
  if (attentionKind === "done_unread") {
    return { background: "#e8b046", boxShadow: "0 0 10px rgba(232, 176, 70, 0.7)" };
  }
  if (w.defect || w.state === "failed") {
    return { background: "#e06058", boxShadow: "0 0 10px rgba(224, 96, 88, 0.55)" };
  }
  if (w.state === "running") {
    return { background: "#4dd6b7", boxShadow: "0 0 10px rgba(77, 214, 183, 0.5)" };
  }
  if (w.state === "starting") {
    return { background: "#7ab6e8", boxShadow: "0 0 10px rgba(122, 182, 232, 0.55)" };
  }
  return { background: "#46505e", boxShadow: "none" };
}

function effectiveAttentionKind(
  w: WorkspaceSummary,
): WorkspaceSummary["attention_kind"] {
  // Skævt deploy: en status fra den gamle skriver kan kun sige
  // `attention:true`. Kombinationen none+true er uproducerbar for den nye
  // skriver og eskaleres derfor konservativt.
  //
  // `?? "none"` er ikke pynt: en ældre backend sender feltet SLET IKKE, og
  // uden defaulten returnerede funktionen `undefined`. Så matcher hverken
  // needs_you- eller done_unread-grenen i statusText/dotStyle, og et kort der
  // venter på ejeren ville falde igennem til state-grenene og stå som "kører"
  // med normal prik — dårligere end i dag. Det er præcis det tilfælde
  // `WorkspaceStatus::effective_attention_kind` dækker på Rust-siden.
  const kind = w.attention_kind ?? "none";
  if (w.attention && kind === "none") return "needs_you";
  return kind;
}

/**
 * Eksporteret, fordi rail'ens drag-kontrakt testes mod den: testen laeser
 * selektorerne ud af CSS'en i stedet for at gentage dem.
 *
 * `-webkit-app-region` haandteres NATIVT af WebView2 (samme mekanik som
 * App.tsx' `[data-global-topbar]`-regel). Drag'en ligger paa ÉT baand med
 * inset — ikke paa roden. Roden er `inset:0` i en `position:fixed`-shell med
 * nulstillet body-margin, saa en drag-flade dér begynder paa vinduets venstre
 * kant og loeber til bunden; WebView2 melder HTCAPTION for drag-regioner, og
 * et traek i kanten eller i et af de to venstre hjoerner ville FLYTTE vinduet
 * i stedet for at resize det (vinduet er `decorations:false, resizable:true`).
 * `no-drag` paa knapperne bliver staaende som vaern for et fremtidigt
 * interaktivt element inde i baandet.
 */
export const railCss = `
  [data-workspace-drag-region] { -webkit-app-region: drag; }
  [data-workspace-rail] button { -webkit-app-region: no-drag; }

  .workspace-rail-sr {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    border: 0;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  /* opacity — IKKE display:none: knapperne skal blive i tab-raekkefoelgen,
     ellers er rail'en utilgaengelig med tastatur (spec §5.2). */
  [data-workspace-actions] { opacity: 0; transition: opacity 140ms ease; }
  [data-workspace-row]:hover [data-workspace-actions],
  [data-workspace-row]:focus-within [data-workspace-actions] { opacity: 1; }

  [data-workspace-row]:hover { background: rgba(126, 179, 225, 0.09); }
  [data-workspace-rail] button:focus-visible {
    outline: 2px solid rgba(103, 183, 255, 0.78);
    outline-offset: 2px;
  }
  [data-workspace-rail] [data-workspace-close]:hover { color: #ffd7d3; }
  [data-workspace-rail] [data-workspace-hide]:hover,
  [data-workspace-rail] [data-workspace-add]:hover,
  [data-workspace-rail] [data-workspace-show-hidden]:hover { color: #e6f2ff; }
`;

const styles: Record<string, CSSProperties> = {
  // Fylder rail-zonen fra AppShell (position: relative, 208 px bred).
  rail: {
    position: "absolute",
    inset: 0,
    display: "flex",
    flexDirection: "column",
    boxSizing: "border-box",
    borderRight: "1px solid rgba(209, 232, 251, 0.1)",
    background:
      "radial-gradient(120% 60% at 0% 0%, rgba(113, 180, 225, 0.1), transparent 62%), linear-gradient(180deg, rgba(10, 20, 34, 0.72), rgba(2, 8, 18, 0.82))",
    backdropFilter: FROSTED_BACKDROP,
    WebkitBackdropFilter: FROSTED_BACKDROP,
    color: "#b9c8d9",
    fontFamily: '"Segoe UI", system-ui, sans-serif',
    fontSize: 12,
    userSelect: "none",
  },
  /**
   * Rail'ens drag-flade — det eneste sted `-webkit-app-region: drag` gaelder.
   *
   * Maalene er den globale topbars (App.tsx `styles.topbar`: `top: TOPBAR_TOP`,
   * `left: 18`), saa husets to drag-flader foelger én regel: hold 18 px fri af
   * vinduets lodrette kant og TOPBAR_TOP fri af den vandrette. Baandet er
   * TOPBAR_HEIGHT hoejt og roerer derfor hverken vinduets bund eller de to
   * venstre hjoerner — resize bliver ved med at vaere resize.
   *
   * `right: 0` er med vilje: rail'ens hoejre kant (x = 208) er en INDRE graense
   * mod canvas-zonen, ikke en vinduekant. Canvas-topbaren starter foerst 18 px
   * inde i sin egen zone, saa der er alligevel en no-drag-rende imellem dem.
   *
   * Baandet ligger over headerens titel (`zIndex: 1`). Titlen er ren tekst uden
   * interaktion, og baandet har ingen baggrund, saa det ses ikke.
   */
  dragBand: {
    position: "absolute",
    top: TOPBAR_TOP,
    left: 18,
    right: 0,
    height: TOPBAR_HEIGHT,
    zIndex: 1,
  },
  // Flugter med topbaren i canvas-zonen (TOPBAR_TOP + TOPBAR_HEIGHT).
  header: {
    flex: "0 0 auto",
    display: "flex",
    alignItems: "center",
    height: TOPBAR_HEIGHT,
    marginTop: TOPBAR_TOP,
    padding: "0 14px",
  },
  headerTitle: {
    color: "#8ea0b5",
    fontSize: 10,
    fontWeight: 700,
    letterSpacing: "0.14em",
    textTransform: "uppercase",
  },
  list: {
    flex: "1 1 auto",
    // minHeight: 0 er ikke pynt — uden den vokser flex-barnet med indholdet
    // i stedet for at scrolle, og post 50 lander under footeren.
    minHeight: 0,
    overflowY: "auto",
    overflowX: "hidden",
    margin: 0,
    padding: "4px 6px",
    listStyle: "none",
  },
  row: {
    position: "relative",
    display: "flex",
    alignItems: "center",
    minHeight: 34,
    borderRadius: 8,
    cursor: "pointer",
    transition: "background 140ms ease",
  },
  rowActive: {
    background: "rgba(122, 182, 232, 0.16)",
    boxShadow: "inset 0 0 0 1px rgba(160, 209, 255, 0.24)",
  },
  rowDim: { opacity: 0.62 },
  open: {
    display: "flex",
    flex: "1 1 auto",
    alignItems: "center",
    gap: 8,
    minWidth: 0,
    border: 0,
    borderRadius: 8,
    padding: "6px 8px",
    background: "transparent",
    color: "inherit",
    font: "inherit",
    textAlign: "left",
    cursor: "inherit",
  },
  dot: {
    width: 8,
    height: 8,
    flex: "0 0 auto",
    borderRadius: 3,
  },
  text: { display: "flex", flexDirection: "column", minWidth: 0, gap: 1 },
  name: {
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    color: "#edf5fc",
    fontSize: 12,
    fontWeight: 600,
  },
  subtitle: {
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    color: "#718297",
    fontFamily: '"Cascadia Mono", monospace',
    fontSize: 10,
  },
  count: {
    flex: "0 0 auto",
    minWidth: 16,
    color: "#8ea0b5",
    fontSize: 10,
    textAlign: "right",
  },
  // Ligger OVER navnet i hoejre side: 208 px er for smalt til en egen kolonne,
  // og handlingerne er alligevel kun synlige ved hover/fokus.
  actions: {
    position: "absolute",
    top: "50%",
    right: 4,
    display: "flex",
    alignItems: "center",
    gap: 2,
    transform: "translateY(-50%)",
    borderRadius: 7,
    paddingLeft: 8,
    background:
      "linear-gradient(90deg, rgba(6, 14, 26, 0), rgba(6, 14, 26, 0.92) 26%)",
  },
  iconButton: {
    display: "grid",
    width: 22,
    height: 22,
    placeItems: "center",
    border: 0,
    borderRadius: 6,
    padding: 0,
    background: "transparent",
    color: "#9eafc1",
    fontSize: 11,
    cursor: "pointer",
  },
  textButton: {
    border: 0,
    borderRadius: 6,
    padding: "3px 5px",
    background: "transparent",
    color: "#8ea0b5",
    font: "inherit",
    fontSize: 10,
    cursor: "pointer",
  },
  empty: {
    flex: "1 1 auto",
    minHeight: 0,
    overflowY: "auto",
    padding: "10px 14px",
    color: "#7f90a4",
    fontSize: 11,
    lineHeight: 1.5,
  },
  footer: {
    flex: "0 0 auto",
    display: "flex",
    flexDirection: "column",
    alignItems: "flex-start",
    gap: 2,
    borderTop: "1px solid rgba(209, 232, 251, 0.08)",
    padding: "8px 10px 10px",
  },
  addButton: {
    width: "100%",
    border: "1px solid rgba(222, 241, 255, 0.18)",
    borderRadius: 8,
    padding: "7px 9px",
    background:
      "radial-gradient(90% 120% at 0% 0%, rgba(113, 180, 225, 0.16), transparent 58%), linear-gradient(145deg, rgba(24, 46, 70, 0.46), rgba(2, 9, 20, 0.4))",
    color: "#c8d9ea",
    font: "inherit",
    fontSize: 11,
    textAlign: "left",
    cursor: "pointer",
  },
  // "Vis skjulte" til venstre, tandhjulet til hoejre. Én raekke, fordi
  // footeren ellers vokser tre knapper i hoejden og aeder af projektlisten i
  // et vindue der ikke er hoejt.
  footerRow: {
    display: "flex",
    width: "100%",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 6,
  },
  ghostButton: {
    border: 0,
    borderRadius: 7,
    padding: "5px 4px",
    background: "transparent",
    color: "#7f90a4",
    font: "inherit",
    fontSize: 10,
    cursor: "pointer",
  },
  ghostButtonOn: { color: "#9fd0ff" },
  settingsButton: {
    display: "grid",
    width: 26,
    height: 26,
    flex: "0 0 auto",
    placeItems: "center",
    border: 0,
    borderRadius: 7,
    padding: 0,
    background: "transparent",
    color: "#8ea0b5",
    cursor: "pointer",
    transition: "background 140ms ease, color 140ms ease",
  },
  settingsButtonOn: {
    background: "rgba(122, 182, 232, 0.16)",
    color: "#edf5fc",
  },
};

export default WorkspaceRail;
