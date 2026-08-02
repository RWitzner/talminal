// Responsivt terminal-canvas.
//
// Kortene ligger i et afledt CSS-grid, så hele den aktuelle kortliste altid
// udfylder den faktiske canvasflade. Layoutet persisteres ikke ved resize:
// browseren ændrer de reelle frame-mål, og Card.tsx' ResizeObserver fitter
// derefter xterm + PTY. Det undgår både transform-skalering og en IPC-skrivestorm.
//
// Tast-arbitration består: klik i en terminal går i type-mode, Shift+Esc går
// tilbage til canvas-mode, og et klik på den tomme canvasflade forlader type-mode.

import {
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent as ReactMouseEvent,
  type SyntheticEvent,
  type PointerEvent as ReactPointerEvent,
  type Ref,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { Card, PREPARE_FRESH_SPAWN_EVENT } from "./Card";
import { BrowserCard } from "./BrowserCard";
import { ChatCard } from "./ChatCard";
import { isBrowserCard, isChatCard, type CardInfo } from "./types";
import { contextPercentFor, type ContextSnapshot } from "./contextHud";
import { POLL_INTERVAL_MS as CONTEXT_POLL_INTERVAL_MS } from "./UsageHud";
import { setOcclusionReason } from "./browser/occlusion";
import { routeKey, type Mode } from "./keyRouting";
import {
  computeResponsiveGridSpacing,
  computeResponsiveTileLayout,
  ORB_DOCK_CLEARANCE,
  TOPBAR_CLEARANCE,
  TOP_ZONE_CLEARANCE,
} from "./canvas/responsiveLayout";
import { CanvasLiquidGlass } from "./canvas/liquidGlass";
import { resolveWallpaperUrl } from "./wallpapers";
import {
  afterPaintOpportunity,
  completePendingClose,
  flushPerfTrace,
  markPerf,
  markPerfOnce,
  pendingCloseEntries,
  pendingCreateEntries,
  perfInvokeArgs,
  registerPendingCreate,
  startPerfTrace,
  type PerfTrace,
} from "./perfTrace";

/** Imperativ kontrol-API — forbruges af voice-dispatch (Task 16b). */
export interface CanvasController {
  /** Saetter type-mode-fokus paa kortet (DOM-fokus til dets xterm-textarea). */
  focusCard(card: number): void;
  /** Nulstiller xterm synkront mellem gammel PTY-stop og frisk spawn. */
  prepareFreshSpawn(card: number): void;
  /** null naar 0 kort er fokuseret (canvas-mode); modellen er single-focus,
   *  saa "2+ fokuserede" kan ikke opstaa. */
  getFocusedCard(): number | null;
}

/** Prefill for opret-kort-dialogen — projektroden; brugeren kan overskrive. */
export async function loadSpawnCwdPrefill(
  getProject: () => Promise<{ root: string }> = () =>
    invoke<{ root: string }>("get_project"),
): Promise<string> {
  const project = await getProject();
  return project.root;
}

/** Race-beslutningen for prefill: kun seneste dialog-aabning maa skrive
 *  (stale resolves ignoreres), og et felt brugeren allerede har tastet i
 *  overskrives ALDRIG. */
export function resolveSpawnPrefill(
  current: string,
  projectRoot: string,
  generationMatches: boolean,
): string {
  if (!generationMatches) return current;
  return current === "" ? projectRoot : current;
}

let spawnPrefillGeneration = 0;

export interface CanvasSurfaceProps {
  cards: CardInfo[];
  wallpaper?: string;
  /** Kaldes efter en persist-mutation er landet Rust-side (App re-henter). */
  onWorkspaceMutation?: (trace?: PerfTrace | null) => void | Promise<void>;
  ref?: Ref<CanvasController>;
}

function PerfFrameObserver({ cards }: { cards: CardInfo[] }) {
  useLayoutEffect(() => {
    const liveNames = new Set(cards.map((card) => card.name));
    for (const [name, trace] of pendingCreateEntries()) {
      if (!liveNames.has(name)) continue;
      if (
        markPerfOnce(
          trace,
          "create.frame_commit",
          "create.frontend.frame_committed",
          { name },
        )
      ) {
        afterPaintOpportunity(() => {
          markPerfOnce(
            trace,
            "create.frame_presented_proxy",
            "create.frontend.frame_presented_proxy",
            { name },
          );
          void flushPerfTrace(trace);
        });
      }
    }
    for (const [name, trace] of pendingCloseEntries()) {
      if (liveNames.has(name)) continue;
      if (
        markPerfOnce(
          trace,
          "close.removed." + name,
          "close.frontend.frame_removed",
          { name },
        )
      ) {
        afterPaintOpportunity(() => {
          markPerfOnce(
            trace,
            "close.removed_presented_proxy." + name,
            "close.frontend.removal_presented_proxy",
            { name },
          );
          const finalTraceMark = completePendingClose(name, trace);
          void flushPerfTrace(trace, finalTraceMark);
        });
      }
    }
  }, [cards]);
  return null;
}

export function CanvasSurface({
  cards,
  wallpaper,
  onWorkspaceMutation,
  ref,
}: CanvasSurfaceProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const frameRefs = useRef(new Map<number, HTMLDivElement>());

  const [mode, setMode] = useState<Mode>("canvas");
  const [focused, setFocused] = useState<number | null>(null);
  const [rootSize, setRootSize] = useState<{ w: number; h: number } | null>(null);
  const [spawnDialogOpen, setSpawnDialogOpen] = useState(false);
  const [spawnCwd, setSpawnCwd] = useState("");
  const [spawnError, setSpawnError] = useState<string | null>(null);
  const [creatingCard, setCreatingCard] = useState(false);
  // Browser-kort-fuldskærm ejes her (spec §8a): hele DOM-framen flyttes ud
  // af grid-layoutet, så dens header bliver liggende over body-rekten.
  // WebView2-barnet må kun få body-rekten; native child-webviews tegner over
  // al DOM, uanset z-index.
  const [fullscreenCard, setFullscreenCard] = useState<number | null>(null);
  // Per-kort context-badges (spec 2026-07-22): ÉN samlet poll for alle kort —
  // ikke én pr. kort. Samme synligheds-gate og kadence som UsageHud: kun
  // mens mindst ét terminal-kort kører.
  const [contextSnapshots, setContextSnapshots] = useState<ContextSnapshot[]>([]);
  const anyRunningTerminal = cards.some(
    (card) => card.kind === "terminal" && card.running,
  );
  useEffect(() => {
    if (!anyRunningTerminal) return;
    let disposed = false;
    const poll = async () => {
      try {
        const next = await invoke<ContextSnapshot[] | null>("read_context_snapshots");
        if (!disposed) setContextSnapshots(Array.isArray(next) ? next : []);
      } catch {
        if (!disposed) setContextSnapshots([]);
      }
    };
    void poll();
    const interval = setInterval(() => void poll(), CONTEXT_POLL_INTERVAL_MS);
    return () => {
      disposed = true;
      clearInterval(interval);
    };
  }, [anyRunningTerminal]);

  // Ref-spejle til stabile native-/pointer-handlers (ingen stale closures).
  const modeRef = useRef(mode);
  modeRef.current = mode;
  const onMutationRef = useRef(onWorkspaceMutation);
  onMutationRef.current = onWorkspaceMutation;

  // Gridden får paddingTop = topbar-clearance og paddingBottom = orb-bånd,
  // så layout-matematikken skal regne på fladen MELLEM de to — ellers er
  // topologivalg og overflow-garantien clearance-højderne forkert.
  const layoutSize = useMemo(
    () => ({
      width: rootSize?.w ?? 1400,
      height: Math.max(
        0,
        (rootSize?.h ?? 900) - TOPBAR_CLEARANCE - ORB_DOCK_CLEARANCE,
      ),
    }),
    [rootSize],
  );
  const tileLayout = useMemo(
    () => computeResponsiveTileLayout(cards.length, layoutSize),
    [cards.length, layoutSize],
  );
  const gridSpacing = useMemo(
    () => computeResponsiveGridSpacing(tileLayout, layoutSize),
    [layoutSize, tileLayout],
  );
  // --- Type-mode ind/ud ----------------------------------------------------

  const enterTypeMode = (number: number) => {
    setMode("type");
    setFocused(number);
  };

  const exitTypeMode = () => {
    // Terminalen maa ikke beholde DOM-fokus i canvas-mode — ellers ville
    // efterfoelgende taster stadig ramme xterm's textarea.
    const ae = document.activeElement;
    if (ae instanceof HTMLElement) ae.blur();
    setMode("canvas");
    setFocused(null);
  };

  useEffect(() => {
    if (focused === null || cards.some((card) => card.number === focused)) return;
    // Et lukket/fjernet kort må ikke efterlade controlleren med et stale
    // type-mode-target.
    setMode("canvas");
    setFocused(null);
  }, [cards, focused]);

  // --- Tast-arbitration (keyRouting.ts er sandheden) -----------------------
  // Capture-fase paa window: Shift+Esc opsnappes FOER xterm's egne listeners
  // (textareaet) ser den — den sendes IKKE til terminalen. Enkelt-Esc og alt
  // andet passerer uroert til terminalen i type-mode (laast beslutning).
  useEffect(() => {
    const onKey = (ev: KeyboardEvent) => {
      const route = routeKey(mode, ev);
      if (route === "exit-type-mode") {
        ev.preventDefault();
        ev.stopPropagation();
        exitTypeMode();
      }
      // "terminal": eventet passerer (xterm-textareaet har DOM-fokus).
      // "canvas": ingen canvas-hotkeys i v0.
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [mode]);

  // NB: denne effekt SKAL registrere sin ResizeObserver FØRST — CanvasSurface.layout.test.tsx antager at rootSize-observeren er ResizeObserverMock.instances[0].
  useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    const syncRootSize = () => {
      const w = root.clientWidth;
      const h = root.clientHeight;
      // WebView2 kan kort rapportere 0x0 under minimize. Behold sidste gyldige
      // grid i stedet for at sende alle terminalframes gennem nul-størrelse.
      if (w <= 0 || h <= 0) return;
      setRootSize((current) =>
        current?.w === w && current.h === h ? current : { w, h },
      );
    };
    syncRootSize();
    const observer = new ResizeObserver(syncRootSize);
    observer.observe(root);
    return () => observer.disconnect();
  }, []);

  // Fuldskaerm foelger Rust-side (spec §8a); auto-exit naar kortet forsvinder/doer.
  // Navnet der sendes er fuldskaerms-kortets — OGSAA naar det ikke er et
  // browser-kort. Det er tilsigtet: `webview_should_show` viser kun det kort
  // hvis navn matcher, saa et chat-kort i fuldskaerm skjuler samtlige
  // browser-webviews. Uden det ville et native WebView2-barn tegne oven paa
  // chat-kortet, som al anden DOM.
  useEffect(() => {
    const card = cards.find((c) => c.number === fullscreenCard);
    if (fullscreenCard !== null && (!card || !card.running)) {
      setFullscreenCard(null);
      return;
    }
    void invoke("set_browser_fullscreen", {
      name: card ? card.name : null,
    }).catch(() => undefined);
  }, [cards, fullscreenCard]);

  // Spawn-dialogen occluderer webviews (central gate - spec §4).
  useEffect(() => {
    setOcclusionReason("spawn-dialog", spawnDialogOpen);
    return () => setOcclusionReason("spawn-dialog", false);
  }, [spawnDialogOpen]);

  // Bounds-rapport: layout-effekt paa BAADE grid-placement OG maalt stoerrelse
  // (ResizeObserver ser kun stoerrelse - rene positionsskift skal ogsaa med),
  // rAF-throttlet. Ogsaa i fuldskaerm bruges den maalte body-rekt: giver vi
  // WebView2 hele roden, dækker native-laget DOM-headerens exit-/URL-knapper.
  // NB: skal blive EFTER rootSize-effekten — dens ResizeObserver skal forblive instances[0] (layout-testens antagelse).
  useEffect(() => {
    let raf = 0;
    const bodyRectOf = (number: number) => {
      const el = frameRefs.current
        .get(number)
        ?.querySelector<HTMLElement>("[data-browser-card-body]");
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { x: r.left, y: r.top, w: r.width, h: r.height };
    };
    const report = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        const root = rootRef.current;
        if (!root) return;
        for (const card of cards) {
          if (!isBrowserCard(card)) continue;
          const rect = bodyRectOf(card.number);
          if (!rect) continue;
          void invoke("set_browser_bounds", {
            name: card.name,
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
          }).catch(() => undefined);
        }
      });
    };
    report();
    const observer = new ResizeObserver(report);
    if (rootRef.current) observer.observe(rootRef.current);
    // Body-elementerne skal OGSÅ observeres: indhold i kortet (fx BrowserCards
    // fejl-strip) kan ændre body-rekten uden at rodens størrelse ændres —
    // uden genrapport bliver den nye DOM dækket af det native WebView2-barn,
    // som tegner over alt ved sin gamle rekt.
    for (const card of cards) {
      if (!isBrowserCard(card)) continue;
      const body = frameRefs.current
        .get(card.number)
        ?.querySelector<HTMLElement>("[data-browser-card-body]");
      if (body) observer.observe(body);
    }
    window.addEventListener("resize", report);
    return () => {
      cancelAnimationFrame(raf);
      observer.disconnect();
      window.removeEventListener("resize", report);
    };
  }, [cards, tileLayout, fullscreenCard]);

  const onRootPointerDown = (ev: ReactPointerEvent) => {
    if (ev.button !== 0) return;
    if ((ev.target as HTMLElement).closest("[data-card-frame], [data-canvas-ui]"))
      return;
    if (modeRef.current === "type") exitTypeMode();
  };

  const onRootDoubleClick = (ev: ReactMouseEvent) => {
    if ((ev.target as HTMLElement).closest("[data-card-frame], [data-canvas-ui]"))
      return;
    setSpawnDialogOpen(true);
    setSpawnError(null);
    setSpawnCwd("");
    const generation = ++spawnPrefillGeneration;
    void loadSpawnCwdPrefill()
      .then((projectRoot) =>
        setSpawnCwd((current) =>
          resolveSpawnPrefill(
            current,
            projectRoot,
            generation === spawnPrefillGeneration,
          ),
        ),
      )
      .catch(() => {});
  };

  const createCardAtSpawnPoint = async (ev: SyntheticEvent<HTMLFormElement>) => {
    ev.preventDefault();
    const cwd = spawnCwd.trim();
    if (!spawnDialogOpen || cwd === "" || creatingCard) return;
    const trace = PERF_ENABLED
      ? startPerfTrace("create", { entry: "ui_create_button" })
      : null;
    if (PERF_ENABLED) markPerf(trace, "create.frontend.submit", { cwd });
    setCreatingCard(true);
    setSpawnError(null);
    try {
      if (PERF_ENABLED) markPerf(trace, "create.frontend.invoke.begin");
      const args = PERF_ENABLED
        ? { cwd, command: null, ...perfInvokeArgs(trace) }
        : { cwd, command: null };
      const info = await invoke<CardInfo>("create_card", args);
      if (PERF_ENABLED) {
        markPerf(trace, "create.frontend.invoke.resolved", {
          card: info.name,
          running: info.running,
        });
        registerPendingCreate(info.name, trace);
      }
      setSpawnDialogOpen(false);
      if (PERF_ENABLED) markPerf(trace, "create.frontend.refresh.requested");
      if (PERF_ENABLED) {
        void onMutationRef.current?.(trace);
      } else {
        void onMutationRef.current?.();
      }
    } catch (err) {
      if (PERF_ENABLED) {
        markPerf(trace, "create.frontend.invoke.rejected", {
          error: String(err),
        });
        void flushPerfTrace(trace, true);
      }
      setSpawnError(String(err));
    } finally {
      setCreatingCard(false);
    }
  };

  const onBodyPointerDown = (ev: ReactPointerEvent, card: CardInfo) => {
    ev.stopPropagation();
    // Klik i kort-krop => type-mode paa det kort (eller skift af kort).
    // xterm's egen click-haandtering giver textareaet DOM-fokus.
    enterTypeMode(card.number);
  };

  // --- Imperativ kontrol-API (Task 16b-forbruger) --------------------------
  useImperativeHandle(
    ref,
    (): CanvasController => ({
      focusCard(number) {
        const target = cards.find((c) => c.number === number);
        if (!target) return;
        // Browser-kort: intet DOM-fokus/type-mode (klik i webviewen naar aldrig
        // DOM'et) — Rust-siden loefter WebView2-barnet i stedet.
        if (isBrowserCard(target)) {
          void invoke("focus_browser_card", { name: target.name }).catch(
            () => undefined,
          );
          return;
        }
        // Chat-kort: ingen xterm, men ejeren skal kunne skrive med det samme.
        // Et chat-kort saetter ALDRIG fokus af sig selv (uprompted spawn maa
        // ikke stjaele fokus, spec §6) — kun naar ejeren selv fokuserer det.
        if (isChatCard(target)) {
          frameRefs.current
            .get(number)
            ?.querySelector<HTMLElement>("[data-chat-input]")
            ?.focus();
          return;
        }
        enterTypeMode(number);
        frameRefs.current
          .get(number)
          ?.querySelector<HTMLElement>(".xterm-helper-textarea")
          ?.focus();
      },
      prepareFreshSpawn(number) {
        const target = cards.find((card) => card.number === number);
        // Kun terminalkort har en PTY at genstarte frisk.
        if (!target || isBrowserCard(target) || isChatCard(target)) return;
        window.dispatchEvent(
          new CustomEvent(PREPARE_FRESH_SPAWN_EVENT, {
            detail: { name: target.name },
          }),
        );
      },
      getFocusedCard() {
        return mode === "type" ? focused : null;
      },
    }),
  );

  // --- Render --------------------------------------------------------------

  return (
    <div
      ref={rootRef}
      data-canvas-root
      data-canvas-mode={mode}
      data-focused-card={mode === "type" && focused !== null ? focused : ""}
      style={styles.root}
      onPointerDown={onRootPointerDown}
      onDoubleClick={onRootDoubleClick}
    >
      {PERF_ENABLED ? <PerfFrameObserver cards={cards} /> : null}
      {/* Kortet strækkes til den responsive grid-frame, og terminalens
          ResizeObserver følger den reelle geometri uden zoom. */}
      <style>{frameCss}</style>
      <CanvasLiquidGlass wallpaperUrl={resolveWallpaperUrl(wallpaper)} />
      <div
        data-terminal-grid
        data-grid-columns={tileLayout.columns}
        data-grid-rows={tileLayout.rows}
        style={{
          ...styles.grid,
          gap: gridSpacing.gap,
          padding: gridSpacing.padding,
          paddingTop: gridSpacing.padding + TOP_ZONE_CLEARANCE,
          paddingBottom: gridSpacing.padding + ORB_DOCK_CLEARANCE,
          gridTemplateColumns:
            tileLayout.columns > 0
              ? `repeat(${tileLayout.columns}, minmax(0, 1fr))`
              : "none",
          gridTemplateRows:
            tileLayout.rows > 0
              ? `repeat(${tileLayout.rows}, minmax(0, 1fr))`
              : "none",
        }}
      >
        {cards.map((card, index) => {
          const placement = tileLayout.tiles[index];
          const isFocused = mode === "type" && focused === card.number;
          // Fuldskaerm hoerer til FRAMEN, ikke til korttypen: chat-kortet har
          // samme knap som browser-kortet. Gaten her var `isBrowserCard`, saa
          // et chat-kort satte fullscreenCard (ikonet skiftede) uden at framen
          // nogensinde forlod sin grid-celle — knappen saa doed ud.
          const isFullscreen = fullscreenCard === card.number;
          // Browser-attributten er stadig BROWSER-specifik: den markerer at et
          // native WebView2-barn skal have body-rekten frem for grid-rekten.
          const isBrowserFullscreen = isBrowserCard(card) && isFullscreen;
          return (
            <div
              key={card.name}
              data-card-frame
              data-card-number={card.number}
              data-card-render-mode="live"
              data-grid-column={placement.column}
              data-grid-row={placement.row}
              data-grid-column-span={placement.columnSpan}
              data-grid-row-span={placement.rowSpan}
              data-card-fullscreen={isFullscreen ? "true" : "false"}
              data-browser-fullscreen-frame={
                isBrowserFullscreen ? "true" : "false"
              }
              ref={(el) => {
                if (el) frameRefs.current.set(card.number, el);
                else frameRefs.current.delete(card.number);
              }}
              style={{
                ...styles.frame,
                ...(isFullscreen
                  ? styles.fullscreenFrame
                  : {
                      gridColumn: `${placement.column} / span ${placement.columnSpan}`,
                      gridRow: `${placement.row} / span ${placement.rowSpan}`,
                    }),
                borderColor: isFocused
                  ? "rgba(104, 184, 255, 0.92)"
                  : "rgba(151, 184, 218, 0.18)",
                boxShadow: isFocused
                  ? "0 0 0 2px rgba(72, 157, 231, 0.2), 0 30px 78px rgba(0, 5, 20, 0.7), inset 0 1px 0 rgba(222, 242, 255, 0.2)"
                  : styles.frame.boxShadow,
              }}
              data-card-focused={isFocused ? "true" : "false"}
            >
              {isBrowserCard(card) ? (
                <BrowserCard
                  card={card}
                  fullscreen={fullscreenCard === card.number}
                  onClosed={onWorkspaceMutation}
                  onToggleFullscreen={() =>
                    setFullscreenCard((current) =>
                      current === card.number ? null : card.number,
                    )
                  }
                />
              ) : isChatCard(card) ? (
                <ChatCard
                  card={card}
                  fullscreen={fullscreenCard === card.number}
                  onToggleFullscreen={() =>
                    setFullscreenCard((current) =>
                      current === card.number ? null : card.number,
                    )
                  }
                />
              ) : (
                <div
                  data-card-body
                  style={styles.body}
                  onPointerDown={(ev) => onBodyPointerDown(ev, card)}
                >
                  <Card
                    card={card}
                    focused={isFocused}
                    onClosed={onWorkspaceMutation}
                    contextPercent={contextPercentFor(
                      contextSnapshots,
                      card.name,
                      card.cwd,
                    )}
                  />
                </div>
              )}
            </div>
          );
        })}
      </div>
      {spawnDialogOpen && (
        <div data-canvas-ui style={styles.dialogBackdrop}>
          <form
            style={styles.dialog}
            onSubmit={createCardAtSpawnPoint}
            onPointerDown={(ev) => ev.stopPropagation()}
          >
            <strong>Nyt kort</strong>
            <label style={styles.dialogLabel}>
              Arbejdsmappe
              <input
                autoFocus
                value={spawnCwd}
                onChange={(ev) => setSpawnCwd(ev.currentTarget.value)}
                placeholder="C:\\sti\\til\\projekt"
                style={styles.dialogInput}
              />
            </label>
            {spawnError !== null && <span style={styles.dialogError}>{spawnError}</span>}
            <div style={styles.dialogActions}>
              <button
                type="button"
                onClick={() => setSpawnDialogOpen(false)}
                style={styles.dialogCancel}
              >
                Annuller
              </button>
              <button
                type="submit"
                disabled={creatingCard || spawnCwd.trim() === ""}
                style={styles.dialogCreate}
              >
                {creatingCard ? "Opretter…" : "Opret"}
              </button>
            </div>
          </form>
        </div>
      )}
    </div>
  );
}

const frameCss = `
  [data-card-body] > section { height: 100%; box-sizing: border-box; }
  [data-card-body] > section > div:last-child {
    height: auto !important; flex: 1 1 0; min-height: 0;
  }
`;

const styles: Record<string, CSSProperties> = {
  // Al baggrund (wallpaper, grundfarve, vignette) bor i CanvasLiquidGlass —
  // dens rodlag er opakt og dækker hele fladen, så en baggrund her ville
  // være dødt render.
  root: {
    position: "absolute",
    inset: 0,
    overflow: "hidden",
    touchAction: "none",
    userSelect: "none",
  },
  grid: {
    position: "absolute",
    inset: 0,
    zIndex: 2,
    display: "grid",
    boxSizing: "border-box",
    isolation: "isolate",
  },
  frame: {
    position: "relative",
    display: "flex",
    flexDirection: "column",
    minWidth: 0,
    minHeight: 0,
    overflow: "hidden",
    boxSizing: "border-box",
    border: "1px solid rgba(151, 184, 218, 0.2)",
    borderRadius: 14,
    background:
      "linear-gradient(145deg, rgba(16, 25, 38, 0.98), rgba(3, 8, 16, 0.98))",
    boxShadow:
      "0 28px 72px rgba(0, 4, 16, 0.66), 0 8px 24px rgba(0, 3, 12, 0.46), inset 0 1px 0 rgba(205, 229, 249, 0.12)",
    transition:
      "border-color 180ms ease, box-shadow 220ms ease, transform 180ms ease",
  },
  fullscreenFrame: {
    position: "absolute",
    top: TOP_ZONE_CLEARANCE,
    right: 0,
    bottom: 0,
    left: 0,
    zIndex: 10,
    borderRadius: 0,
  },
  body: { flex: 1, minHeight: 0, overflow: "hidden", position: "relative" },
  dialogBackdrop: {
    position: "absolute",
    inset: 0,
    zIndex: 20,
    display: "grid",
    placeItems: "center",
    background: "rgba(0,0,0,0.45)",
  },
  dialog: {
    width: 360,
    display: "flex",
    flexDirection: "column",
    gap: 14,
    padding: 20,
    border: "1px solid #3b4453",
    borderRadius: 10,
    background: "#1a1d21",
    boxShadow: "0 18px 50px rgba(0,0,0,0.55)",
  },
  dialogLabel: { display: "flex", flexDirection: "column", gap: 6, fontSize: 12 },
  dialogInput: {
    padding: "8px 10px",
    border: "1px solid #4b5563",
    borderRadius: 5,
    background: "#0d0f12",
    color: "#e5e7eb",
    font: "inherit",
  },
  dialogError: { color: "#f87171", fontSize: 12, overflowWrap: "anywhere" },
  dialogActions: { display: "flex", justifyContent: "flex-end", gap: 8 },
  dialogCancel: {
    padding: "6px 12px",
    border: "1px solid #4b5563",
    borderRadius: 5,
    background: "transparent",
    color: "#d1d5db",
    cursor: "pointer",
  },
  dialogCreate: {
    padding: "6px 12px",
    border: "none",
    borderRadius: 5,
    background: "#3b82f6",
    color: "#fff",
    cursor: "pointer",
  },
};
