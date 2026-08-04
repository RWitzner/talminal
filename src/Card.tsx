import { useEffect, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import type {
  CardColor,
  CardExitEvent,
  CardInfo,
  CardState,
  PtyOutputEvent,
} from "./types";
import { base64ToBytes } from "./base64";
import { CARD_HEADER, CARD_NUMBER_BADGE, CARD_SHELL } from "./cardChrome";
import { cardColor } from "./colors";
import { cardLabel } from "./cardLabel";
import { ContextBadge } from "./ContextBadge";
import { nudgeRepaint } from "./nudge";
import {
  DICTATION_INSERT_EVENT,
  type DictationInsertDetail,
} from "./dictationInsert";
import { hexDump, isBracketedPaste, isTerminalReply } from "./terminalReply";
import { attachTerminalClipboard } from "./terminalClipboard";
import { writeClipboardText } from "./clipboard";
import {
  afterPaintOpportunity,
  completePendingCreate,
  createTraceFor,
  flushPerfTrace,
  markPerf,
  markPerfOnce,
} from "./perfTrace";

export interface CardProps {
  card: CardInfo;
  focused?: boolean;
  /** Kortets context-forbrug 0-100 (contextPercentFor i CanvasSurface) —
   *  null/udeladt = intet snapshot, ingen badge. */
  contextPercent?: number | null;
  /** Kaldes naar kortet ER lukket Rust-side, saa fladen kan hente kortlisten
   *  igen. Uden den bliver kortet staaende indtil en urelateret refresh —
   *  samme klasse som `cards-changed`-fundet. */
  onClosed?: () => void;
}

export const PREPARE_FRESH_SPAWN_EVENT = "talminal:prepare-fresh-spawn";

interface PrepareFreshSpawnDetail {
  name: string;
}

const STATUS_COLOR: Record<CardColor, string> = {
  green: "rgba(69, 226, 145, 0.72)",
  red: "rgba(255, 107, 119, 0.76)",
  neutral: "rgba(207, 224, 242, 0.46)",
  gray: "rgba(151, 177, 204, 0.34)",
};

// Terminal-protokol-auto-svar (fix F1, udvidet efter M0b-FUND 2): fuldt
// xterm.js-auto-svar-katalog + dokumentation bor i terminalReply.ts (testet
// i terminalReply.test.ts). Matches sendes med source:"terminal"; alt andet
// er human-input med hex-diagnostik af ESC-indledte chunks (fantom-jagt).
// NB (Task 4): pause-/epoch-semantikken er ude af MVP-pathen — write_pty
// kaldes altid med epoch: 0 (frossen IPC-kontrakt: parametrene bestaar,
// default-state ignorerer dem), og source-klassifikationen BEVARES.

// Single-flight-vaern (fix F11 aendrer formaalet, ikke behovet): spawn er
// KNAP-drevet, men spawn_card/respawn_card maa stadig kun vaere in-flight
// een ad gangen pr. kortnavn — dobbeltklik, HMR/remounts (og en evt.
// fremtidig StrictMode) er dermed ufarlige.
const spawnPromises = new Map<string, Promise<void>>();

export function Card({
  card,
  focused = false,
  contextPercent = null,
  onClosed,
}: CardProps) {
  const { name } = card;
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  // Review-fix (Task 9): frontend-sandhed om "pty'en koerer" — gater syncSize,
  // saa sene/debounced resizes efter exit eller foer start ikke rammer
  // resize_pty forgaeves og stoejer i konsollen ("card not running").
  const runningRef = useRef(card.running);
  const focusedRef = useRef(focused);
  focusedRef.current = focused;
  const syncSizeRef = useRef<(() => void) | null>(null);
  const lifecycleVersionRef = useRef(0);
  const freshSpawnNeedsResizeRef = useRef(false);
  // running som render-state (statusprik via cardColor); runningRef er den
  // imperative tvilling inde i effect-closurerne.
  const [running, setRunning] = useState(card.running);
  const [exited, setExited] = useState(card.exited !== null);
  // Exit-koden fra pty-child'en (farve: 0 => neutral, !=0 => red). card-exit-
  // eventet baerer ingen kode — den hentes via get_card_state (samme
  // hydrerings-kommando som reload-stien).
  const [exitCode, setExitCode] = useState<number | null>(card.exited);
  // started: kortet koerer eller har koert (styrer Start-overlay, fix F11).
  const [started, setStarted] = useState(card.running || card.exited !== null);
  // En afvist lukning er haard og handlingsbar og maa ALDRIG forsvinde i et
  // tomt catch — det er den fejlklasse browser-kortets bug 2/3 handlede om.
  const [closeError, setCloseError] = useState<string | null>(null);

  const closeCard = () => {
    setCloseError(null);
    void invoke("close_card", { name })
      .then(() => onClosed?.())
      .catch((error: unknown) =>
        setCloseError("Kunne ikke lukke kortet: " + String(error)),
      );
  };

  // Fix F11: spawn er KNAP-drevet (Start / Fortsaet / Frisk session).
  // Single-flight via spawnPromises.
  const startCard = (cmd: "spawn_card" | "respawn_card") => {
    if (spawnPromises.has(name)) return; // allerede in-flight
    const lifecycleVersion = ++lifecycleVersionRef.current;
    termRef.current?.reset();
    setStarted(true);
    setExited(false);
    setExitCode(null);
    const p = invoke<void>(cmd, { name })
      .then(() => {
        if (lifecycleVersion !== lifecycleVersionRef.current) return;
        runningRef.current = true;
        setRunning(true);
        syncSizeRef.current?.();
        // The clicked overlay button owns DOM focus. It is unmounted by the
        // running-state commit, which otherwise leaves focus on BODY while
        // CanvasSurface is already in type mode. Focus xterm after that paint
        // opportunity so picker keys reach the new PTY without a second click.
        window.requestAnimationFrame(() => {
          if (
            lifecycleVersion === lifecycleVersionRef.current &&
            runningRef.current &&
            focusedRef.current
          ) {
            termRef.current?.focus();
          }
        });
      })
      .catch((err) => {
        if (lifecycleVersion !== lifecycleVersionRef.current) return;
        setStarted(false);
        termRef.current?.writeln(`[canvas] ${cmd}(${name}) fejlede: ${err}`);
      })
      .finally(() => {
        spawnPromises.delete(name);
      });
    spawnPromises.set(name, p);
  };

  const applyLifecycleState = (state: Pick<CardState, "running" | "exited">) => {
    runningRef.current = state.running;
    setRunning(state.running);
    setExited(state.exited !== null);
    setExitCode(state.exited);
    setStarted(state.running || state.exited !== null);
  };

  const reconcileLifecycleState = (
    version: number,
    state: CardState,
    wasRunning: boolean,
  ) => {
    if (version !== lifecycleVersionRef.current) return;
    applyLifecycleState(state);
    if (
      state.running &&
      (!wasRunning || freshSpawnNeedsResizeRef.current)
    ) {
      freshSpawnNeedsResizeRef.current = false;
      syncSizeRef.current?.();
    }
  };

  // Voice restart mutates Rust outside this component. App.refresh supplies a
  // new CardInfo object with the same stable name. Apply it immediately, then
  // reconcile against the current run so stale snapshot/event ordering cannot
  // leave the exit overlay over a live fresh process (or revive a dead one).
  useEffect(() => {
    if (spawnPromises.has(name)) return;
    const version = ++lifecycleVersionRef.current;
    const wasRunning = runningRef.current;
    applyLifecycleState(card);
    void invoke<CardState>("get_card_state", { name })
      .then((state) => reconcileLifecycleState(version, state, wasRunning))
      .catch(() => {
        /* snapshot-state bevares, hvis Rust-read fejler */
      });
  }, [card, name]);

  // Ingen stop-knap i kortet (ejer-beslutning 2026-07-20): kort stoppes via
  // voice-kommandoer (kill_card ad den vej); M10-flowet Stop -> Frisk session
  // består Rust-side.

  // Hold lifecycle-state frisk fra Rust, så stop/exit afspejles i kortet.
  useEffect(() => {
    let disposed = false;
    let unlisten: UnlistenFn | null = null;
    void listen<CardExitEvent>("card-exit", (e) => {
      if (disposed || e.payload.name !== name) return;
      const version = ++lifecycleVersionRef.current;
      const wasRunning = runningRef.current;
      runningRef.current = false;
      setRunning(false);
      setStarted(true);
      setExited(true);
      void invoke<CardState>("get_card_state", { name })
        .then((state) => {
          if (!disposed) reconcileLifecycleState(version, state, wasRunning);
        })
        .catch(() => {
          /* farven forbliver konservativ uden kode */
        });
    })
      .then(async (registered) => {
        if (disposed) {
          registered();
          return;
        }
        unlisten = registered;
        // Listener FOER state-read lukker registreringsvinduet: exits baade
        // foer og efter read observeres i static- og live-mode.
        try {
          const versionAtRead = ++lifecycleVersionRef.current;
          const wasRunning = runningRef.current;
          const state = await invoke<CardState>("get_card_state", { name });
          if (!disposed) reconcileLifecycleState(versionAtRead, state, wasRunning);
        } catch {
          /* CardInfo-startstate bevares, hvis Rust-read fejler */
        }
      })
      .catch((err) => console.error(`card-exit listener(${name}) fejlede:`, err));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [name]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    let disposed = false;
    const alive = () => !disposed;

    // FUND 2 (ConPTY-spike): CC er en alt-screen-app, og for DEN er kortet et
    // rent live-viewport — alt-bufferen har altid hasScrollback === false,
    // uanset hvad vi saetter her. Den oprindelige scrollback: 0 var derfor
    // rigtig for CC og blev forkert da codex kom til: codex' TUI koerer inline
    // i NORMAL-bufferen (D0-spiken), og uden scrollback konverterer xterm
    // hjulet til pil op/ned — som i codex' composer blader i tidligere
    // prompts. 2000 linjer giver codex rigtig rulning; CC er upaavirket.
    const term = new Terminal({
      scrollback: 2000,
      fontSize: 13,
      fontFamily: "'Cascadia Mono', Consolas, monospace",
      cursorBlink: true,
      theme: {
        background: "#02060c",
        foreground: "#d8e2ef",
        cursor: "#a9dcff",
        cursorAccent: "#07101d",
        selectionBackground: "rgba(100, 183, 255, 0.3)",
        selectionInactiveBackground: "rgba(100, 183, 255, 0.18)",
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(container);
    // Ctrl+C er ALTID kopiér (ejer-beslutning 2026-07-28): 0x03 kan ikke
    // sendes fra tastaturet. Afbryd = Esc (keyRouting.ts), afslut = kortets ×.
    // NB: i Claude-kort er markeringsvejen SHIFT+traek — CC rapporterer mus,
    // og xterm slaar sin egen SelectionService fra naar en app goer det.
    attachTerminalClipboard(term, writeClipboardText);
    if (PERF_ENABLED) {
      markPerfOnce(
        createTraceFor(name),
        "create.term_open",
        "create.frontend.term_open",
        {
          name,
          width: container.clientWidth,
          height: container.clientHeight,
        },
      );
    }
    const hasRenderableSize = () =>
      container.clientWidth > 0 && container.clientHeight > 0;
    if (hasRenderableSize()) fit.fit();
    termRef.current = term;

    // Voice restart owns a fresh-session boundary outside this component.
    // Reset synchronously after the old PTY has stopped and BEFORE spawn_card:
    // resetting from a later workspace snapshot can erase the new run's first
    // alt-screen bytes, which are not replayable with scrollback disabled.
    const prepareFreshSpawn = (event: Event) => {
      const detail = (event as CustomEvent<PrepareFreshSpawnDetail>).detail;
      if (!detail || detail.name !== name) return;
      ++lifecycleVersionRef.current;
      // Establish the stopped boundary even if the old card-exit event is
      // delivered after kill_card resolves. The next running snapshot must
      // resize the brand-new PTY (whose backend starts at 120x30).
      runningRef.current = false;
      freshSpawnNeedsResizeRef.current = true;
      term.reset();
    };
    window.addEventListener(PREPARE_FRESH_SPAWN_EVENT, prepareFreshSpawn);

    // Diktering: teksten indsaettes gennem xterms EGEN paste-vej, ikke med et
    // raat write_pty. Det er den samme begrundelse som Ctrl+V's
    // (terminalClipboard.ts): paste bevarer bracketed paste, saa en blok
    // lander som ÉT stykke i agentens composer i stedet for at en newline
    // midt i saetningen submitter for tidligt. Og fordi term.paste() flyder
    // videre gennem onData-handleren nedenfor, er der stadig praecis EEN vej
    // ind i pty'en — med uaendret source-klassifikation og gating.
    const insertDictation = (event: Event) => {
      const detail = (event as CustomEvent<DictationInsertDetail>).detail;
      if (!detail || detail.name !== name) return;
      if (!runningRef.current) return;
      term.paste(detail.text);
    };
    window.addEventListener(DICTATION_INSERT_EVENT, insertDictation);

    // Review-fix (Task 9): ResizeObserver + debounce-timeren koerer videre
    // efter card-exit — runningRef-gaten sikrer, at en sen/debounced resize
    // efter exit (M10-flowet: Stop -> Frisk session) eller foer Start ikke
    // kalder resize_pty forgaeves. Residual-racet (exit Rust-side FOER
    // card-exit-eventet naar frontenden) daekkes i catch-grenen:
    // "card not running" er dér FORVENTET tilstand — logges som debug;
    // alle andre fejl er stadig console.error.
    let repaintPending = false;
    const syncSize = () => {
      if (
        !alive() ||
        !hasRenderableSize()
      )
        return;
      if (repaintPending) {
        repaintPending = false;
        void nudgeRepaint(name, term, fit);
        return;
      }
      if (!runningRef.current) return;
      fit.fit();
      void invoke("resize_pty", { name, cols: term.cols, rows: term.rows }).catch(
        (err) => {
          if (String(err).startsWith("card not running")) {
            console.debug(`resize_pty(${name}) efter exit (forventet):`, err);
            return;
          }
          console.error(`resize_pty(${name}) fejlede:`, err);
        },
      );
    };
    syncSizeRef.current = syncSize;

    // Input -> write_pty. Pause-/epoch-semantikken er ude af MVP-pathen
    // (Task 4): epoch sendes altid som 0 (frossen IPC-kontrakt — default-
    // state accepterer alle vaerdier), og der er ingen ejerskabs-flip.
    // Fix F1: xterm's egne protokol-auto-svar (CPR/DA/fokus/kitty — se
    // TERMINAL_REPLY_RE) er IKKE menneske-input: de sendes med
    // source:"terminal"; source-klassifikationen BEVARES for Rust-sidens
    // gating/telemetri (Task 8-kontrakten).
    const dataDisposable = term.onData((data) => {
      if (isTerminalReply(data)) {
        void invoke("write_pty", {
          name,
          data,
          epoch: 0,
          source: "terminal",
        }).catch((err) => {
          console.error(`write_pty(${name}, terminal) fejlede:`, err);
        });
        return;
      }
      // FUND 2-diagnostik: en ESC-indledt chunk der IKKE matcher kataloget er
      // enten aegte tastatur-ANSI (piletaster mv.) eller en NY auto-svar-form.
      // Hex-dumpes saa et evt. nyt fantom kan identificeres praecist i
      // devtools i stedet for at vaere et tavst mysterium.
      // Indsat tekst er ESC-indledt (bracketed paste) men en KENDT form — uden
      // undtagelsen ville hver indsaetning hex-dumpe hele sit indhold til
      // devtools.
      if (data.startsWith("\x1b") && !isBracketedPaste(data)) {
        console.debug(
          `[canvas] card=${name} ESC-chunk klassificeret som human:`,
          JSON.stringify(data),
          hexDump(data),
        );
      }
      void invoke("write_pty", {
        name,
        data,
        epoch: 0,
        source: "human",
      }).catch((err) => {
        console.error(`write_pty(${name}) fejlede:`, err);
      });
    });
    // Resize -> fit -> resize_pty, debounced 150 ms (FUND 5: resize er robust,
    // men hvert kald koster et repaint-burst - undgaa storm under vindues-traek;
    // kvitteringen er implicit: CC's repaint ankommer som almindelig pty-output).
    let resizeTimer: number | undefined;
    const observer = new ResizeObserver(() => {
      window.clearTimeout(resizeTimer);
      resizeTimer = window.setTimeout(syncSize, 150);
    });
    observer.observe(container);

    // Fix F3: listen()-registreringerne AFVENTES (Promise.all) FOER
    // get_card_state/spawn — events emittet af reader-traaden i
    // registrerings-vinduet (fx cmd's banner) ville ellers tabes lydloest.
    // Fix F10: derefter hydreres kortets Rust-side-tilstand — en
    // webview-reload maa hverken dobbelt-spawne ("already running") eller
    // efterlade en blank terminal.
    let unlisteners: UnlistenFn[] = [];
    void (async () => {
      const registered = await Promise.all([
        listen<PtyOutputEvent>("pty-output", (e) => {
          if (alive() && e.payload.name === name) {
            const bytes = base64ToBytes(e.payload.data_b64);
            if (!PERF_ENABLED) {
              term.write(bytes);
              return;
            }
            const trace = createTraceFor(name);
            if (!trace) {
              term.write(bytes);
              return;
            }
            const first = markPerfOnce(
              trace,
              "create.first_pty_chunk",
              "create.frontend.first_pty_chunk",
              {
                name,
                base64_bytes: e.payload.data_b64.length,
              },
            );
            if (!first) {
              term.write(bytes);
              return;
            }
            term.write(bytes, () => {
              markPerf(trace, "create.frontend.first_pty_write_complete", {
                name,
              });
              afterPaintOpportunity(() => {
                markPerf(
                  trace,
                  "create.frontend.first_pty_chunk_presented_proxy",
                  { name },
                );
                completePendingCreate(name, trace);
                void flushPerfTrace(trace, true);
              });
            });
          }
        }),
      ]);
      if (!alive()) {
        // Cleanup naaede at koere foer registreringen resolvede — afmeld selv.
        registered.forEach((u) => u());
        return;
      }
      unlisteners = registered;
      const trace = PERF_ENABLED ? createTraceFor(name) : null;
      if (PERF_ENABLED) {
        markPerfOnce(
          trace,
          "create.pty_listener_registered",
          "create.frontend.pty_listener_registered",
          { name },
        );
        void flushPerfTrace(trace);
      }
      try {
        if (PERF_ENABLED) {
          markPerf(trace, "create.frontend.card_state.begin", { name });
        }
        const s = await invoke<CardState>("get_card_state", { name });
        if (PERF_ENABLED) {
          markPerf(trace, "create.frontend.card_state.end", {
            name,
            running: s.running,
          });
        }
        if (!alive()) return;
        // Lifecycle-effekten ovenfor ejer running/exited-state og beskytter
        // den mod stale reads. Dette read afgør KUN repaint ved live-attach.
        // (s.owner/s.epoch findes stadig paa wiren, men ignoreres — Task 4.)
        if (s.running) {
          if (PERF_ENABLED) {
            markPerfOnce(
              trace,
              "create.transport_interactive",
              "create.frontend.transport_interactive",
              { name, pty_listener_registered: true, backend_running: true },
            );
            void flushPerfTrace(trace);
          }
          // Reattach efter reload ELLER static->live: xterm er tom (en frisk
          // Terminal er tom uanset scrollback), og output-gaten er aabnet
          // foer Card skifter til live. Task 8-helperen bevarer
          // shrink/grow-koreografien.
          if (hasRenderableSize()) {
            if (PERF_ENABLED) {
              markPerf(trace, "create.frontend.nudge_repaint.begin", { name });
            }
            await nudgeRepaint(name, term, fit);
            if (PERF_ENABLED) {
              markPerf(trace, "create.frontend.nudge_repaint.end", { name });
              void flushPerfTrace(trace);
            }
          } else {
            // Et minimeret/skjult canvas kan rapportere 0x0. Vent med baade
            // fit og backend-resize til ResizeObserver ser en reel stoerrelse;
            // ellers reducerer xterm til 2x1 og blanker den genattach'ede PTY.
            repaintPending = true;
          }
        }
        // Kort autostarter IKKE (fix F11) — Start-knappen kalder spawn_card.
      } catch (err) {
        if (alive()) term.writeln(`[canvas] get_card_state(${name}) fejlede: ${err}`);
      }
    })();

    return () => {
      disposed = true;
      window.clearTimeout(resizeTimer);
      observer.disconnect();
      dataDisposable.dispose();
      window.removeEventListener(PREPARE_FRESH_SPAWN_EVENT, prepareFreshSpawn);
      window.removeEventListener(DICTATION_INSERT_EVENT, insertDictation);
      unlisteners.forEach((u) => u());
      unlisteners = [];
      syncSizeRef.current = null;
      term.dispose();
      termRef.current = null;
    };
  }, [name]);

  // Statusfarve = REN funktion af lokal running/exited (colors.ts, Task 4).
  const color: CardColor = cardColor({ running, exited: exitCode });
  const statusColor = STATUS_COLOR[color];
  const restoreBadge = card.restore_action === "fresh_shared_cwd";
  // P6b: knaptekst pr. profil — spike §5.3 bekraeftede at Fortsaet VISES
  // for codex, blot med resume-flaget i stedet for --continue. Wiring
  // (respawn_card) er uaendret; det er kun det operatoeren LAESER der aendres.
  const resumeLabel =
    card.profile === "codex" ? "Fortsæt (resume --last)" : "Fortsæt (--continue)";
  const cwdLabel =
    card.cwd
      .replace(/[\\/]+$/, "")
      .split(/[\\/]/)
      .filter(Boolean)
      .at(-1) ?? card.cwd;

  return (
    <section style={styles.card}>
      <header style={styles.header}>
        <span
          data-card-number-badge
          style={{
            ...styles.numberBadge,
            ...(focused ? styles.numberBadgeFocused : {}),
          }}
        >
          {card.number}
        </span>
        <span
          style={{
            ...styles.statusDot,
            background: statusColor,
            boxShadow: `0 0 7px ${statusColor}`,
          }}
          title={running ? "Kører" : exited ? "Afsluttet" : "Ikke startet"}
          aria-hidden="true"
        />
        {/* Visning, ikke identitet: `name` er fortsat card-N ud mod backend
            (close_card ovenfor, spawn, pty-events) — kun det laeste er dansk. */}
        <span style={styles.title}>{cardLabel(name)}</span>
        {/* Agent-label (Task 7): profil-slug'et vises for BEGGE agenter —
            blandede claude/codex-workspaces skal kunne aflaeses uden at
            aabne kortet. Samme daempede tone som ContextBadge. */}
        <span style={styles.agentLabel} data-card-agent-label>
          {card.profile}
        </span>
        <span style={styles.cwd} title={card.cwd}>
          {cwdLabel}
        </span>
        {/* Slut-review fix 4: badgen gates EKSPLICIT paa claude-profilen.
            contextHud.ts joiner kun paa kortnavn+cwd — intet agent-felt,
            ingen friskhed — og kortnavne genbruges (registry'ets
            laveste-ledige), saa et codex-kort i samme cwd ellers arver et
            gammelt Claude-snapshot og viser et frossent, fremmed CTX-tal.
            "Har data" er altsaa IKKE laengere Claude-filtret i sig selv. */}
        {running && card.profile === "claude" && (
          <ContextBadge percent={contextPercent} />
        )}
        {restoreBadge && (
          <span style={styles.restoreBadge} title="Delt mappe — frisk session">
            frisk
          </span>
        )}
        {/* Luk-vejen skal kunne SES. Indtil nu var stemmen ("Luk kort N") den
            eneste maade at lukke et terminalkort paa, mens browser-kortet
            havde sit ✕ — en luk-vej man ikke kan se, findes ikke for den der
            ikke allerede kender den. onPointerDown stopper her, saa klikket
            ikke ogsaa saetter kortet i type-mode paa vej ud. */}
        <button
          type="button"
          data-card-close-action
          onPointerDown={(event) => event.stopPropagation()}
          onClick={closeCard}
          title="Luk kort"
          aria-label="Luk kort"
          style={styles.closeButton}
        >
          ✕
        </button>
      </header>
      {closeError !== null && (
        <div
          data-card-error
          style={styles.errorStrip}
          title="Klik for at skjule"
          onClick={() => setCloseError(null)}
        >
          {closeError}
        </div>
      )}
      <div style={styles.termWrap}>
        <div ref={containerRef} style={styles.term} />
        {!started && !exited && (
          <div style={styles.overlay}>
            {/* Fix F11: kort autospawner ikke — operatoeren starter dem.
                Begge veje tilbydes ogsaa HER (ikke kun i exit-overlayet):
                efter en APP-genstart har den friske proces ingen
                exit-historik (exited=None), men recovery-valget
                (claude --continue) skal stadig vaere eksplicit muligt
                (run-bookens Step 11). */}
            <p>Kortet er ikke startet.</p>
            <button style={styles.button} onClick={() => startCard("spawn_card")}>
              Start
            </button>
            <button style={styles.button} onClick={() => startCard("respawn_card")}>
              {resumeLabel}
            </button>
          </div>
        )}
        {exited && (
          <div style={styles.overlay}>
            <p>Kortet er afsluttet.</p>
            {/* Fix F11: recovery-valget er EKSPLICIT operatoerens —
                fortsat samtale (claude --continue) eller frisk session. */}
            <button style={styles.button} onClick={() => startCard("respawn_card")}>
              {resumeLabel}
            </button>
            <button style={styles.button} onClick={() => startCard("spawn_card")}>
              Frisk session
            </button>
          </div>
        )}
      </div>
    </section>
  );
}

const styles: Record<string, CSSProperties> = {
  card: CARD_SHELL,
  header: { ...CARD_HEADER, fontSize: 12 },
  numberBadge: {
    ...CARD_NUMBER_BADGE,
    // Kun terminal-kortets badge skifter udseende (fokus-tilstanden nedenfor),
    // saa kun den har brug for en overgang.
    transition: "background 180ms, border-color 180ms, color 180ms",
  },
  numberBadgeFocused: {
    borderColor: "rgba(116, 192, 255, 0.5)",
    background: "rgba(69, 145, 216, 0.55)",
    color: "#f4f9ff",
  },
  statusDot: {
    width: 5,
    height: 5,
    flex: "0 0 auto",
    borderRadius: "50%",
  },
  title: { color: "#e2ebf4", fontWeight: 600, whiteSpace: "nowrap" },
  agentLabel: {
    flex: "0 0 auto",
    fontSize: 10,
    fontWeight: 600,
    padding: "1px 6px",
    borderRadius: 999,
    border: "1px solid rgba(222, 241, 255, 0.18)",
    background: "rgba(2, 9, 20, 0.35)",
    color: "#9eafc1",
    whiteSpace: "nowrap",
  },
  cwd: {
    minWidth: 0,
    flex: 1,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    color: "#64758a",
    fontFamily: '"Cascadia Mono", monospace',
    fontSize: 9,
  },
  restoreBadge: {
    padding: "1px 5px",
    border: "1px solid rgba(222, 169, 94, 0.24)",
    borderRadius: 5,
    background: "rgba(120, 77, 25, 0.2)",
    color: "#d4ae76",
    fontSize: 9,
    whiteSpace: "nowrap",
  },
  closeButton: {
    // Spejler browser-kortets ikonknap, saa de to korttyper lukkes ens.
    // `marginLeft: auto` skubber den til hoejre kant uanset hvilke badges der
    // staar foran den — cwd'en er den eneste der maa aede resten af pladsen.
    marginLeft: "auto",
    flex: "0 0 auto",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    width: 22,
    height: 22,
    padding: 0,
    border: "1px solid rgba(187, 211, 233, 0.14)",
    borderRadius: 6,
    background: "rgba(112, 137, 163, 0.12)",
    color: "#9eafc1",
    cursor: "pointer",
    fontSize: 12,
    lineHeight: 1,
  },
  errorStrip: {
    flex: "0 0 auto",
    padding: "4px 10px",
    background: "rgba(120, 32, 32, 0.55)",
    borderBottom: "1px solid rgba(255, 120, 120, 0.25)",
    color: "#f0b9b9",
    fontSize: 11,
    cursor: "pointer",
    whiteSpace: "nowrap",
    overflow: "hidden",
    textOverflow: "ellipsis",
  },
  button: {
    background: "#3b82f6",
    color: "#fff",
    border: "none",
    borderRadius: 4,
    padding: "3px 10px",
    cursor: "pointer",
    fontSize: 12,
  },
  termWrap: {
    position: "relative",
    height: 420,
    background: "#02060c",
    boxShadow: "inset 0 14px 32px rgba(0, 3, 10, 0.18)",
  },
  term: { position: "absolute", inset: 0 },
  overlay: {
    position: "absolute",
    inset: 0,
    background: "rgba(2, 8, 18, 0.86)",
    backdropFilter: "blur(18px) saturate(130%)",
    WebkitBackdropFilter: "blur(18px) saturate(130%)",
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
  },
};
