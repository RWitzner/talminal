import {
  memo,
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { AppShell } from "./AppShell";
import { WorkspaceRail } from "./WorkspaceRail";
import {
  confirmationDemand,
  loadWorkspaces,
  workspaceListFrom,
  type WorkspaceSummary,
} from "./workspaces";
import { CloseWorkspaceDialog } from "./CloseWorkspaceDialog";
import { CanvasSurface, type CanvasController } from "./CanvasSurface";
import { loadAppSnapshot, type ProjectInfo } from "./appSnapshot";
import { FROSTED_BACKDROP } from "./canvas/liquidGlass";
import {
  TOPBAR_CLEARANCE,
  TOPBAR_HEIGHT,
  TOPBAR_TOP,
} from "./canvas/responsiveLayout";
import { setOcclusionReason } from "./browser/occlusion";
import { SettingsWindow } from "./SettingsWindow";
import WindowControls from "./WindowControls";
import type {
  CardInfo,
  CardsStatus,
  VoiceRoutes,
  WorkspaceResponse,
} from "./types";
import { createPcmPlayer, type PcmPlayer } from "./voice/audioPlayer";
import { createVoiceDispatcher, type DispatchHudEvent, type DispatchResult } from "./voice/dispatch";
import { createDryRunDispatch } from "./voice/dryRun";
import Hud, {
  deriveHudView,
  type HudResolver,
  type HudSessionState,
  type HudState,
} from "./voice/Hud";
import CastLayer from "./voice/CastLayer";
import Orb from "./voice/Orb";
import { UsageHud } from "./UsageHud";
import { withLevelMeter } from "./voice/orbLevel";
import { createSoundPlayer, withSoundFeedback } from "./voice/sound";
import {
  isKeyboardBinding,
  registerPttKey,
  startBrowserCapture,
} from "./voice/ptt";
import {
  createPipelineVoiceSession,
  type PipelineUiState,
} from "./voice/pipeline";
import { createOpenAiSttClient, STT_DOMAIN_PROMPT } from "./voice/stt";
import { loadReplyClips } from "./voice/clipAssets";
import { createClipTts } from "./voice/clipTts";
import {
  DEFAULT_DICTATION_HOTKEY,
  DEFAULT_PTT_HOTKEY as DEFAULT_VOICE_HOTKEY,
} from "./hotkeyDefaults";
import { createDictationSession } from "./voice/dictation";
import {
  describeBlockedTarget,
  resolveDictationTarget,
} from "./voice/dictationTarget";
import { createHotkeyBridge } from "./voice/hotkeyBridge";
import { createMicArbiter } from "./voice/micArbiter";
import { emitDictationInsert } from "./dictationInsert";
import {
  closeTraceFor,
  flushPerfTrace,
  getActiveVoiceTrace,
  installPerfHarness,
  markPerf,
  startPerfTrace,
  type PerfTrace,
} from "./perfTrace";

// KUN Settings lazy-loades, og selve lazy-soemmen bor nu i `SettingsWindow`
// sammen med den flade der bruger den. CastLayer blev proevet og rullet
// tilbage: laget renderes UBETINGET nedenfor, saa React starter dets import ved
// foerste render alligevel — splitningen udskoed intet, kostede en ekstra
// modul-request og flyttede mount'et en microtask senere end orben, der maaler
// op imod det.

const VITE_ENV = (import.meta as ImportMeta & {
  env?: Record<string, string | undefined>;
}).env;
const DRY_RUN_FORCED = ["1", "true", "yes", "on"].includes(
  VITE_ENV?.VITE_VOICE_DRY_RUN?.toLocaleLowerCase() ?? "",
);
// Vite saetter DEV=false i `vite build`, altsaa i den binaer der sendes ud.
// Laeses for sig og ikke gennem VITE_ENV: dén er typet som strenge, mens DEV
// er en aegte boolean. Det valgfrie opslag daekker testmiljoeer uden env.
const DEV_BUILD =
  (import.meta as ImportMeta & { env?: { DEV?: boolean } }).env?.DEV === true;
// Sikkerhedsnet mod en haengende taleknap — IKKE en forventet laengde. Naar
// den rammer, KASSERES turen (pipeline.ts:520, dictation.ts:124 aflyser og
// melder fejl), saa alt det sagte er tabt. Graensen skal derfor ligge over den
// laengste ytring et menneske faktisk siger i ét straek, ikke taet paa den:
// ét minut cuttede ejeren midt i saetningen.
//
// Samme tal for begge veje gennem mikrofonen. Dikteringen er den af de to der
// realistisk loeber laengst (fri tekst frem for én kommando), saa et lavere
// tal dér ville ramme foerst og haardest.
const MAX_UTTERANCE_MS = 3 * 60_000;

const INITIAL_HUD: HudState = {
  session: "asleep",
  transcript: "",
  responseText: "",
  tool: null,
  resolver: null,
  error: null,
  chain: null,
};

/**
 * Den luk-bekraeftelse der staar aaben, hvis nogen.
 *
 * `kind` afgoer hvor svaret skal hen, og det er hele pointen med at have to
 * varianter: en global app-lukning er allerede i gang i backendens
 * CloseRequested-funnel og venter paa `confirm_close(ok)` — svarer vi ikke,
 * bliver appen staaende i `Confirming` og kan aldrig lukkes igen. Et ANDET
 * (evt. skjult) workspace har derimod ingen ventende funnel: der er
 * bekraeftelsen ren forhaandsgodkendelse, og et "nej" skal ikke sende noget
 * som helst.
 */
type CloseTarget =
  | {
      kind: "application";
      workspaces: number;
      running: number;
    }
  | {
      kind: "workspace";
      slug: string;
      name: string;
      running: number;
      /**
       * Kom anmodningen fra "Fjern fra listen"? Saa besvares bekraeftelsen ved
       * at kalde `set_workspace_hidden` IGEN med `confirmed: true`, og
       * backenden lukker og skjuler i ét og samme kald.
       *
       * Skjulningen maa ikke armeres her i React-state og udfoeres senere:
       * "Fjern" rammer ogsaa det workspace brugeren SELV sidder i, og dér doer
       * staten sammen med processen, foer nogen betingelse kan indtraeffe
       * (slutreview B1).
       */
      thenHide: boolean;
    };

type QuitSummary = {
  workspaces: number;
  running_cards: number;
};

function quitSummaryFrom(payload: unknown): QuitSummary {
  if (payload !== null && typeof payload === "object") {
    const value = payload as Record<string, unknown>;
    const workspaces = Number(value.workspaces);
    const running = Number(value.running_cards);
    if (Number.isFinite(workspaces) && Number.isFinite(running)) {
      return {
        workspaces: Math.max(1, Math.trunc(workspaces)),
        running_cards: Math.max(0, Math.trunc(running)),
      };
    }
  }
  // Kompatibilitet med en ældre backend under dev hot-reload, hvor payloaden
  // kun var dette vindues antal kørende kort.
  const running = Number(payload);
  return {
    workspaces: 1,
    running_cards: Number.isFinite(running) ? Math.max(0, Math.trunc(running)) : 0,
  };
}

/** En anmodning paa vej gennem koen. */
type NewClose = CloseTarget & {
  /** Elementet fokus skal tilbage til, fanget paa gerningsstedet. */
  restoreFocus: HTMLElement | null;
};

type PendingClose = NewClose & {
  /** Monoton, unik pr. anmodning. Bruges som React-`key` paa dialogen, saa to
   *  anmodninger ALDRIG kan reconcile til den samme knap — se `closeQueue`. */
  id: number;
};

function pipelineSessionForHud(state: PipelineUiState): HudSessionState {
  switch (state) {
    case "idle":
      return "idle";
    case "listening":
      return "listening";
    case "finalizing":
    case "processing":
      return "processing";
    case "speaking":
      return "speaking";
  }
}

/** Staar tekstmarkoeren i et chat-korts composer? I saa fald traad-id'et.
 *
 *  DOM-fokus er den mest direkte sandhed om hvor en diktering hoerer hjemme,
 *  og for chat-kort er det den ENESTE: canvas' type-mode-fokus kan aldrig
 *  pege paa dem (se voice/dictationTarget.ts' hoved-kommentar). `data-chat-card`
 *  baerer traad-id'et — kortnavnet staar ingen steder i chat-kortets traeer. */
function focusedChatThread(): string | null {
  if (typeof document === "undefined") return null;
  const active = document.activeElement;
  if (!(active instanceof HTMLElement)) return null;
  if (!active.matches("[data-chat-input]")) return null;
  return active.closest("[data-chat-card]")?.getAttribute("data-chat-card") ?? null;
}

function resolverFromResult(result: unknown): HudResolver | null {
  if (!result || typeof result !== "object" || !("ok" in result)) return null;
  const dispatchResult = result as DispatchResult;
  if (!dispatchResult.ok) return { ok: false, reason: dispatchResult.message };
  if (dispatchResult.cards) return { ok: true, cards: dispatchResult.cards };
  if (dispatchResult.card !== undefined) return { ok: true, card: dispatchResult.card };
  return { ok: true };
}

function PerfHarnessInstaller({
  refresh,
}: {
  refresh(trace?: PerfTrace | null): Promise<void>;
}) {
  useEffect(() => installPerfHarness({ refresh }), [refresh]);
  return null;
}

// App ejer bl.a. realtime/HUD-state og kan derfor rendre langt oftere end
// workspace-listen ændrer sig. Rail'en har sin egen pending-state; memo holder
// de urelaterede canvas-renders ude af dens rækker uden at blokere klikfeedback.
const MemoizedWorkspaceRail = memo(WorkspaceRail);

export default function App() {
  const [cards, setCards] = useState<CardInfo[] | null>(null);
  const [workspace, setWorkspace] = useState<WorkspaceResponse | null>(null);
  // Rail'ens liste. Tom er en gyldig tilstand — ogsaa naar backenden endnu
  // ikke kender `list_workspaces` (se refreshWorkspaces).
  const [workspaces, setWorkspaces] = useState<WorkspaceSummary[]>([]);
  const [showHidden, setShowHidden] = useState(false);
  // Luk-bekraeftelsen. Vises ALTID her i det synlige vindue — ogsaa naar det er
  // et skjult workspace i en anden proces der lukkes, for den proces kan ikke
  // tegne noget paa skaermen.
  //
  // Det er en KØ og ikke ét felt (fix-runde 1, fund 2). Der er to kilder —
  // rail'ens ✕ og backendens `close-confirm-requested` (Alt+F4, taskbar) — og
  // med ét felt overskrev den sidste den foerste. Da samme indre dialog stod
  // paa samme plads, reconcilede React det som en PROP-aendring: "Luk Alpha?"
  // muterede PAA STEDET til "Afslut Talminal?" med fare-knappen paa uaendret
  // position, og et museklik der allerede var paa vej ned lukkede hele vinduet
  // i stedet for ét projekt. Samtidig faldt Alpha-anmodningen tavst paa gulvet.
  // Koen loeser begge: den nye anmodning stiller sig BAG den aabne, og
  // `id` som React-`key` garanterer unmount+mount ved skiftet, saa fokus tages
  // forfra og knappen aldrig skifter betydning under markoeren.
  const [closeQueue, setCloseQueue] = useState<PendingClose[]>([]);
  const closeIdRef = useRef(0);
  const closeRequest = closeQueue[0] ?? null;
  const [project, setProject] = useState<ProjectInfo | null>(null);
  const [cardsStatus, setCardsStatus] = useState<CardsStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  // Tilstanden bor her og ikke i vinduet, fordi baade rail'ens tandhjul og
  // selve fladen skal kende den — de bor i hver sin AppShell-zone.
  // Handlerne er `useCallback`, fordi rail'en er `memo`'et: to friske
  // funktioner pr. render ville gen-tegne den ved hver canvas-opdatering og
  // dermed aflive praecis den memoisering, MemoizedWorkspaceRail findes for.
  const [settingsOpen, setSettingsOpen] = useState(false);
  const openSettings = useCallback(() => setSettingsOpen(true), []);
  const closeSettings = useCallback(() => setSettingsOpen(false), []);
  const [dryRun, setDryRun] = useState(DRY_RUN_FORCED);
  const [capturePath, setCapturePath] = useState<string | null>(null);
  const [hud, setHud] = useState<HudState>(INITIAL_HUD);
  // Backend-fejl (spawn fejlet, degraderede browser-tools, keeper-kapring)
  // har sin EGEN kanal ved siden af voice-fejlen i `hud.error`. Grunden er en
  // reel kapløbssituation (T9-review): create_card returnerer Ok(info) OGSÅ
  // når best-effort-spawn'et fejlede, så voice-vejens new_card fyrer sit
  // rutine-info-event ("N kort oprettet i …") LIGE efter Rust har emittet
  // fejlen — og handleHudEvent's info-arm sætter error: null. Med to kanaler
  // kan en rutine-success kun rydde SIN egen (voice-)fejl; backend-fejlen
  // står til en ny voice-tur begynder eller til en ny fejl afløser den.
  const [backendError, setBackendError] = useState<string | null>(null);
  const [settingsWarning, setSettingsWarning] = useState<string | null>(null);
  const warningShownRef = useRef(false);
  // Orbens fejl-blus er edge-trigget: tick'et bumper pr. fejl-patch i
  // updateHud, så identiske fejltekster blusser hver gang.
  const [errorTick, setErrorTick] = useState(0);
  const micLevelRef = useRef({ level: 0, at: 0 });
  const playerRef = useRef<PcmPlayer | null>(null);
  const controllerRef = useRef<CanvasController>(null);
  const dryRunRef = useRef(dryRun);
  const cardsRef = useRef<CardInfo[]>([]);
  const voiceRoutesRef = useRef<VoiceRoutes | null>(null);
  const captureReadyRef = useRef<Promise<string | null>>(Promise.resolve(null));
  const voiceEngineRef = useRef<string | null>(null);
  dryRunRef.current = dryRun;
  cardsRef.current = cards ?? [];
  voiceRoutesRef.current = workspace?.voice_routes ?? null;

  // Monotont refresh-token: to overlappende refetches kan resolve i omvendt
  // rækkefølge, og et forældet snapshot ville så vinde permanent (der er
  // ingen periodisk poll til at rette det — kun events trigger refresh).
  // Kun det SENEST startede kalds svar må sætte state (ghost-kort-hærdning).
  const refreshSeqRef = useRef(0);
  const refresh = useCallback(async (trace?: PerfTrace | null) => {
    const seq = ++refreshSeqRef.current;
    if (PERF_ENABLED) markPerf(trace, "frontend.refresh.begin", { seq });
    try {
      const snapshot = await loadAppSnapshot();
      if (PERF_ENABLED) {
        markPerf(trace, "frontend.refresh.snapshot_resolved", {
          seq,
          card_count: snapshot.cards.length,
        });
      }
      if (seq !== refreshSeqRef.current) return;
      setCards(snapshot.cards);
      setWorkspace(snapshot.workspace);
      // Et transient get_project-udfald må ikke rive en allerede vist label ned.
      setProject((current) => snapshot.project ?? current);
      setLoadError(null);
      if (PERF_ENABLED) {
        markPerf(trace, "frontend.refresh.state_enqueued", { seq });
        void flushPerfTrace(trace);
      }
    } catch (err) {
      if (seq !== refreshSeqRef.current) return;
      if (PERF_ENABLED) {
        markPerf(trace, "frontend.refresh.failed", {
          seq,
          error: String(err),
        });
        void flushPerfTrace(trace, true);
      }
      setLoadError(String(err));
    }
  }, []);

  // Fælles chokepunkt for ALLE backend-fejl-events: skriv til backend-kanalen,
  // ryd en ældre voice-fejl (nyeste fejl vinder — der vises altid kun én) og
  // bump orbens edge-triggede blus.
  const showBackendError = useCallback((message: string) => {
    setBackendError(message);
    setHud((current) =>
      current.error === null ? current : { ...current, error: null },
    );
    setErrorTick((tick) => tick + 1);
  }, []);

  // Kort-status er en engangsdiagnose (findes kortfilen, kunne den laeses) og
  // haenger ikke paa nogen af de events der invaliderer kortlisten. Selve
  // KORTLISTEN hentes derimod foerst naar lytterne staar — se effekten nedenfor.
  useEffect(() => {
    invoke<CardsStatus>("get_cards_status")
      .then(setCardsStatus)
      .catch(() => {});
  }, []);

  // Kortlistens effekt: den registrerer alle de events der invaliderer listen,
  // og henter FOERST derefter det foerste snapshot (raekkefoelgen er bindende —
  // se kommentaren ved `void refresh()` nederst).
  //
  // Browser-kort-sporet: Rust emitter browser-card-updated (navigation ->
  // ny url/title) og browser-card-dead (webview lukket/crashet). Begge
  // invaliderer kortlisten, saa vi re-henter snapshot'et (samme listen-moenster
  // som Card.tsx: registreringen afventes, cleanup afmelder).
  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];
    let disposed = false;
    void (async () => {
      const registered = await Promise.all([
        listen("browser-card-updated", () => void refresh()),
        // Kort skabt af en AGENT (card_pair's partner og chat-kort) — Rust
        // emitter dette, fordi kortlisten her ikke polles. Uden lytteren laa
        // begge kort usynlige indtil ejeren tilfaeldigvis selv oprettede et
        // kort, hvis create_card-vej selv kalder refresh (dogfood-fund
        // 2026-07-25).
        listen("cards-changed", () => void refresh()),
        // Spec §6.2: reveal SKAL udløse et frontend-refresh. `settings.json` er
        // GLOBAL (`project::global_base()`), så en ændring i ét workspace
        // rammer disken øjeblikkeligt — men aldrig et skjult workspaces
        // React-state, for der findes ingen periodisk poll (se refresh's
        // kommentar). Uden lytteren vågnede B op med A's gamle wallpaper OG med
        // den gamle wake-hotkey stadig registreret globalt, så den nye ikke
        // virkede nogen steder. `refresh()` henter præcis samme snapshot som
        // ved mount (cards + workspace.settings + project) og har allerede sin
        // egen sekvensvagt.
        listen("workspace-revealed", () => void refresh()),
        listen<{ names?: string[] }>("browser-card-dead", (event) => {
          if (PERF_ENABLED) {
            for (const name of event.payload.names ?? []) {
              const trace = closeTraceFor(name);
              markPerf(trace, "close.frontend.browser_dead_event", { name });
              void flushPerfTrace(trace);
            }
          }
          void refresh();
        }),
        // Degraderet worker-spawn (bug 6-fladen): Talminal-MCP/--no-chrome kom
        // ikke med, så agenten kan falde tilbage til Claude in Chrome. Det må
        // aldrig være usynligt — vis det i HUD'ens fejlkanal med orb-blus.
        listen<{ card: string; reason: string }>(
          "worker-browser-tools-degraded",
          (event) => {
            showBackendError(
              `${event.payload.card}: browser-tools mangler (${event.payload.reason}) — agenten kan bruge Claude in Chrome i stedet`,
            );
          },
        ),
        // En agent navigerede keeperen udenom kort-systemet (sprang
        // browser_card_open over). Backenden har nulstillet siden; kapringen
        // må ikke være usynlig for operatøren.
        listen<{ scope: string; owner: string }>(
          "browser-keeper-hijacked",
          (event) => {
            showBackendError(
              `${event.payload.owner}: browser-styring udenom kortene (keeper nulstillet) — agenten skal bruge browser_card_open`,
            );
          },
        ),
        // N4 (T9): create-vejens best-effort-spawn kan fejle — exe'en kunne
        // ikke oploeses (codex ikke paa PATH) eller selve PTY-spawnet slog
        // fejl. En SLETTET cwd er derimod IKKE en trigger: portable-pty
        // falder tavst tilbage til %USERPROFILE% (cmdbuilder.rs'
        // current_directory), saa agenten starter blot et andet sted.
        // Kortet forbliver u-spawnet (exit-overlayets
        // manuelle vej er fallback), men det maa ikke vaere lydloest. Rust-
        // fejlteksten (fx fra resolve_spawn_program) ER brugerteksten og
        // vises uaendret. Ingen agent-gate — rammer ogsaa et Claude-kort.
        listen<{ name: string; number: number; error: string }>(
          "card-spawn-failed",
          (event) => {
            showBackendError(`${event.payload.name}: ${event.payload.error}`);
          },
        ),
        // Lukningen blev AFLYST: successoren kvitterede ikke inden for fristen,
        // og backenden lukker ikke et synligt vindue uden at nogen har overtaget
        // skaermen (spec §6.4 — invarianten "mindst ét synligt vindue" vejer
        // tungere end det ✕ brugeren lige trykkede paa). Uden denne notits saa
        // det ud som en doed knap.
        listen<string>("workspace-handoff-failed", (event) => {
          showBackendError(
            `${event.payload} svarede ikke — vinduet blev staaende. Proev igen, eller skift projekt foerst.`,
          );
        }),
      ]);
      if (disposed) {
        registered.forEach((unlisten) => unlisten());
        return;
      }
      unlisteners.push(...registered);
      // **Foerste hentning ligger EFTER registreringen — bindende raekkefoelge**,
      // samme regel som workspace-lytteren nedenfor og af samme grund: `listen()`
      // er en asynkron round-trip til Rust. Hentede vi foerst, fandtes der et
      // vindue hvor `cards-changed` (eller browser-card-*) kunne udsendes uden at
      // nogen lyttede, og INGEN af dem gentages. Uden en periodisk poll (se
      // refresh's kommentar) ville kortlisten saa staa forkert indtil naeste gang
      // noget tilfaeldigvis aendrede sig — praecis den fejl der gjorde card_pair's
      // to kort usynlige (dogfood-fund 2026-07-25).
      //
      // Den sene start er ufarlig: lander et event mens registreringen er i
      // luften, starter dets refresh FOER denne, og `refreshSeqRef` lader den
      // senest startede vinde. Begge henter friske data, saa der findes ingen
      // vej hvor et forældet snapshot overskriver et nyere.
      void refresh();
    })().catch((err) => {
      // Da hentningen flyttede ind BAG registreringen, blev fail-silent-vejen
      // bredere: foerste snapshot haenger nu paa at ALLE `listen()`-kaldene i
      // `Promise.all` resolver. Afvises ét eneste af dem, naar `void refresh()`
      // aldrig at koere — og saa faar brugeren hverken kort ELLER den fejltekst
      // en doed IPC plejede at give (dengang fejlede `loadAppSnapshot`, og
      // `loadError` satte notitsen oeverst paa skaermen).
      //
      // Ingen retry: uden lyttere er kortlisten doed for resten af sessionen
      // (der er ingen periodisk poll — se refresh's kommentar), saa et forsoeg
      // mere ville kun udskyde tavsheden. Derfor skrives fejlen BAADE i
      // konsollen (samme form som Card.tsx' card-exit-lytter) og i den synlige
      // `loadError`-kanal, saa den tomme flade faar en aarsag.
      console.error("kortliste-lyttere fejlede:", err);
      setLoadError(`kortliste-lytterne kunne ikke registreres: ${String(err)}`);
    });
    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [refresh, showBackendError]);

  // Workspace-rail'en. Listen hentes ved mount og opdateres KUN af
  // `workspaces-changed` — ingen frontend-poll (spec §5.2).
  const refreshWorkspaces = useCallback(async () => {
    // loadWorkspaces sluger fejlen og giver en tom liste. Det er tilsigtet:
    // Task 8's `list_workspaces` findes ikke endnu, og Tauri afviser ukendte
    // commands — rail'en skal degradere, ikke faelde appen.
    setWorkspaces(await loadWorkspaces(() => invoke("list_workspaces")));
  }, []);

  /**
   * **Raekkefoelgen er bindende: lytteren FOERST, hentningen derefter.**
   *
   * `listen()` er en asynkron round-trip til Rust. Startede hentningen foerst,
   * fandtes der et vindue hvor backenden kunne udsende `workspaces-changed`
   * uden at nogen lyttede — og fordi badge-tick'et kun emitter VED DIFF, kommer
   * der ikke automatisk et nyt event bagefter. Rail'en kunne dermed staa med en
   * forkert liste indtil naeste gang noget tilfaeldigvis aendrede sig.
   *
   * `etEventErLandet` daekker den anden halvdel: et event der lander mens den
   * foerste hentning er i luften, er NYERE end svaret paa hentningen. Uden
   * vagten ville det gamle svar overskrive det.
   */
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let disposed = false;
    let etEventErLandet = false;
    void (async () => {
      const registered = await listen("workspaces-changed", (event) => {
        etEventErLandet = true;
        // Eventet baerer listen selv (kontrakt B). Baerer det noget andet,
        // henter vi frem for at rydde rail'en paa et gaet.
        const list = workspaceListFrom(event.payload);
        if (list === null) void refreshWorkspaces();
        else setWorkspaces(list);
      });
      if (disposed) {
        registered();
        return;
      }
      unlisten = registered;
      const liste = await loadWorkspaces(() => invoke("list_workspaces"));
      if (!disposed && !etEventErLandet) setWorkspaces(liste);
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [refreshWorkspaces]);

  // Rail'ens fire handlinger. Alle fire commands ejes af senere tasks (T8,
  // T11, T12); indtil de lander afvises kaldet, og fejlen vises i
  // backend-kanalen frem for at forsvinde i en tom catch.
  const workspaceCommand = useCallback(
    (command: string, args: Record<string, unknown>, hvad: string) => {
      void invoke(command, args).catch((err) => {
        showBackendError(`${hvad}: ${String(err)}`);
      });
    },
    [showBackendError],
  );
  const activateWorkspace = useCallback(
    (slug: string) => workspaceCommand("activate_workspace", { slug }, "Kunne ikke skifte projekt"),
    [workspaceCommand],
  );
  /** Brugerens "ja" paa ✕-dialogen. `confirmed: true` er svaret — se
   *  `closeWorkspace` for foerste halvdel af den to-trins-form. */
  const requestCloseWorkspace = useCallback(
    (slug: string) =>
      workspaceCommand(
        "request_close_workspace",
        { slug, confirmed: true },
        "Kunne ikke lukke projektet",
      ),
    [workspaceCommand],
  );
  /** Fokus paa gerningsstedet — dialogen kan ikke selv finde det tilbage. */
  const activeElementNow = (): HTMLElement | null =>
    document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null;
  const enqueueClose = useCallback(
    (entry: NewClose) =>
      setCloseQueue((queue) => [...queue, { ...entry, id: ++closeIdRef.current }]),
    [],
  );
  /**
   * Rail'ens ✕ — ÉN vej, uanset hvilken post der lukkes.
   *
   * Kaldet gaar altid til `request_close_workspace(slug, confirmed: false)`, og
   * backenden svarer enten "kør bare" eller `kraever_bekraeftelse:<n>`. Er der
   * kort med koerende sessioner, aabner vi den EKSISTERENDE dialog paa
   * backendens tal og kalder igen med `confirmed: true` — praecis samme
   * to-trins-form som "Fjern fra listen".
   *
   * **Hvorfor ikke laengere en gren paa `is_active`.** Foer traf frontenden to
   * beslutninger paa rail'ens liste, og den liste er op til ét sekund gammel
   * (badge-kadencen):
   *
   * 1. *Skal der spoerges?* — `running_cards <= 0` sprang dialogen over. Et kort
   *    der startede i det sekund var usynligt for beslutningen, og et workspace
   *    med en levende agent-session kunne lukkes uden et spoergsmaal. Tallet
   *    laeses nu paa beslutningstidspunktet, i den proces der ejer det.
   * 2. *Er det MIG?* — `is_active` afgjorde om vi kaldte
   *    `getCurrentWindow().close()`. Umiddelbart efter et skift staar den
   *    markering paa den FORRIGE post i op til et sekund, saa ✕ paa den post
   *    lukkede det vindue brugeren sad i. Identitet maa ikke udledes af en
   *    tilstand der halter.
   *
   * At vejen nu gaar ud i control-kanalen og tilbage til vores egen poller
   * koster op til ét poll-tick (200 ms) paa vores eget workspace. Der kommer
   * ikke to spoergsmaal ud af det: `begin_peer_close` springer sin egen
   * bekraeftelse over, netop fordi den allerede er givet her. Titelbjaelkens ✕,
   * Alt+F4 og taskbar-luk gaar uaendret gennem `CloseRequested`-funnelen.
   */
  const closeWorkspace = useCallback(
    (slug: string) => {
      const restoreFocus = activeElementNow();
      void invoke("request_close_workspace", { slug, confirmed: false }).catch(
        (err) => {
          const running = confirmationDemand(err);
          if (running === null) {
            showBackendError(`Kunne ikke lukke projektet: ${String(err)}`);
            return;
          }
          enqueueClose({
            kind: "workspace",
            slug,
            // Listen er kun til NAVNET her — er posten forsvundet imens, er
            // sluggen stadig noget brugeren kan genkende.
            name: workspaces.find((w) => w.slug === slug)?.name ?? slug,
            running,
            thenHide: false,
            restoreFocus,
          });
        },
      );
    },
    [enqueueClose, showBackendError, workspaces],
  );
  /**
   * Rail'ens "Fjern"/"Hent frem".
   *
   * "Hent frem" (`hidden === false`) er harmloest og gaar direkte igennem.
   *
   * "Fjern" paa et KOERENDE workspace bliver afvist af backenden med
   * `kraever_bekraeftelse:<n>`, FOER den rykker noget: lukningen skal ske ad
   * samme vej som ✕ (spec punkt 7), og den vej spoerger. Vi genkender
   * praefikset, laeser antallet ud af det og aabner den EKSISTERENDE
   * bekraeftelse — samme dialog, samme koe, samme fokus-haandtering som rail'ens
   * ✕. Kan praefikset ikke laeses, falder vi tilbage til den almindelige notits.
   *
   * Det gaelder OGSAA det workspace brugeren selv sidder i: `instance_alive` er
   * sand for en selv, saa afvisningen kommer ogsaa dér, og bekraeftelsen er
   * lige saa paakraevet.
   *
   * Fokus fanges HER paa gerningsstedet — naar afvisningen lander, har React
   * ikke rendret noget nyt endnu, men vi er inde i en asynkron callback, og
   * dokumentets fokus kan i mellemtiden vaere flyttet.
   */
  const setWorkspaceHidden = useCallback(
    (slug: string, hidden: boolean) => {
      if (!hidden) {
        workspaceCommand(
          "set_workspace_hidden",
          { slug, hidden, confirmed: false },
          "Kunne ikke opdatere listen",
        );
        return;
      }
      const restoreFocus = activeElementNow();
      // `confirmed: false` er hele pointen med foerste kald: backenden faar lov
      // at afvise, saa bekraeftelsen stilles paa backendens tal og ikke paa et
      // gaet fra rail'ens liste.
      void invoke("set_workspace_hidden", {
        slug,
        hidden: true,
        confirmed: false,
      }).catch((err) => {
        const running = confirmationDemand(err);
        if (running === null) {
          showBackendError(`Kunne ikke opdatere listen: ${String(err)}`);
          return;
        }
        enqueueClose({
          kind: "workspace",
          slug,
          // Listen er backendens sandhed; er posten forsvundet imens, er
          // sluggen stadig et navn brugeren kan genkende.
          name: workspaces.find((w) => w.slug === slug)?.name ?? slug,
          running,
          thenHide: true,
          restoreFocus,
        });
      });
    },
    [enqueueClose, showBackendError, workspaceCommand, workspaces],
  );
  const addWorkspace = useCallback(
    () => workspaceCommand("add_workspace", {}, "Kunne ikke tilføje projektet"),
    [workspaceCommand],
  );

  /**
   * Global Talminal-exit. Backendens CloseRequested-funnel har standset
   * titel-✕/Alt+F4/taskbar-luk og beder om ét svar; payloaden er et frisk
   * snapshot af alle levende workspaces og deres kørende kort.
   *
   * Fokus fanges her i lytteren og ikke i dialogen: paa dette tidspunkt staar
   * fokus stadig paa titelbjaelkens ✕ (React har ikke rendret endnu), og det
   * er praecis det element brugeren skal have tilbage.
   *
   * Anmodningen stiller sig BAGERST i koen. Staar der allerede en dialog, maa
   * den ikke mutere (fund 2); og der koees hoejst ÉN `application`, for
   * funnelen spoerger kun én gang pr. lukning — to poster ville betyde to
   * `confirm_close`-svar paa ét spoergsmaal.
   */
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let disposed = false;
    void (async () => {
      const registered = await listen("close-confirm-requested", (event) => {
        const summary = quitSummaryFrom(event.payload);
        const restoreFocus = activeElementNow();
        setCloseQueue((queue) =>
          queue.some((entry) => entry.kind === "application")
            ? queue
            : [
                ...queue,
                {
                  id: ++closeIdRef.current,
                  kind: "application",
                  workspaces: summary.workspaces,
                  running: summary.running_cards,
                  restoreFocus,
                },
              ],
        );
      });
      if (disposed) {
        registered();
        return;
      }
      unlisten = registered;
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  /** Fjern den besvarede anmodning; en evt. koeet anmodning rykker frem og
   *  monteres som en NY dialog (nyt `id` = ny `key` = unmount+mount). */
  const dequeueClose = useCallback(
    () => setCloseQueue((queue) => queue.slice(1)),
    [],
  );

  /**
   * "Fjern"-vejens anden halvdel: samme command, nu med brugerens svar.
   *
   * **Skjulningen SKAL ske i backenden** (slutreview B1). "Fjern" rammer ogsaa
   * det workspace brugeren selv sidder i, og dér kan en armeret frontend-
   * skjulning aldrig udfoeres: posten kan pr. konstruktion ikke melde sig nede
   * foer processen er doed — hverken mens overdragelsen venter, eller mens
   * ExitRequested draener PtyHostene — og saa er React-staten vaek med den.
   * Foer fixet betoed det at ejeren kunne trykke "Fjern", besvare en destruktiv
   * bekraeftelse, miste sine agent-sessioner — og finde posten uaendret i
   * listen ved naeste opstart, uden en eneste fejlbesked.
   *
   * Med `confirmed: true` skriver backenden `.hidden` synkront i selve kaldet,
   * i samme rækkefoelge som nul-kort-grenen allerede havde: luk-anmodningen
   * foerst (fejler den, skjules der intet — en skjult post med levende agenter
   * er praecis det vi undgaar), sidecaren umiddelbart efter, laenge foer en
   * lukning kan naa at blive udfoert.
   */
  const confirmClose = useCallback(() => {
    const pending = closeRequest;
    dequeueClose();
    if (pending === null) return;
    if (pending.kind === "application") {
      workspaceCommand("confirm_close", { ok: true }, "Kunne ikke afslutte Talminal");
    } else if (pending.thenHide) {
      workspaceCommand(
        "set_workspace_hidden",
        { slug: pending.slug, hidden: true, confirmed: true },
        "Kunne ikke opdatere listen",
      );
    } else {
      requestCloseWorkspace(pending.slug);
    }
  }, [
    closeRequest,
    dequeueClose,
    requestCloseWorkspace,
    workspaceCommand,
  ]);

  const cancelClose = useCallback(() => {
    const pending = closeRequest;
    dequeueClose();
    // Kun application-vejen har en funnel der venter. Uden "nej"'et bliver den
    // staaende i Confirming, og en ny global lukning kan ikke begynde.
    if (pending?.kind === "application") {
      workspaceCommand(
        "confirm_close",
        { ok: false },
        "Kunne ikke afbryde lukningen",
      );
    }
  }, [closeRequest, workspaceCommand]);

  // Browser-kortenes webviews er native boern og maler OVER DOM'et. Uden denne
  // gate ville et aabent browser-kort daekke bekraeftelsen — samme aarsag som
  // settings-panelets registrering nedenfor (spec §4a).
  const closeDialogOpen = closeRequest !== null;
  useEffect(() => {
    setOcclusionReason("close-dialog", closeDialogOpen);
    return () => setOcclusionReason("close-dialog", false);
  }, [closeDialogOpen]);

  const voiceReady = workspace !== null;
  const voiceHotkey =
    workspace?.settings?.ptt_hotkey?.trim() || DEFAULT_VOICE_HOTKEY;
  const dictationHotkey =
    workspace?.settings?.dictation_hotkey?.trim() || DEFAULT_DICTATION_HOTKEY;
  // `|| null` og IKKE `|| DEFAULT`: alt-bindingerne har ingen default. En tom
  // streng betyder "ryddet", og den maa aldrig naa parse-laget — `""` er en
  // parse-fejl efter den delte grammatik, ikke en maade at sige "ingen".
  const voiceHotkeyAlt =
    workspace?.settings?.ptt_hotkey_alt?.trim() || null;
  const dictationHotkeyAlt =
    workspace?.settings?.dictation_hotkey_alt?.trim() || null;
  // Via ref og IKKE via effektens deps: stemme-effekten river hele motoren ned
  // og bygger den op igen naar dens deps skifter (pipeline.stop(),
  // player.close(), alle lyttere af- og paamonteres). Laa flaget i deps, ville
  // ét klik paa kontakten i indstillingerne koste den fulde teardown.
  const dictationSubmitRef = useRef(false);
  dictationSubmitRef.current = workspace?.settings?.dictation_submit === true;
  const voiceEngine =
    VITE_ENV?.VITE_VOICE_ENGINE ??
    workspace?.settings?.voice_engine ??
    "realtime";
  if (workspace !== null && voiceEngineRef.current === null) {
    voiceEngineRef.current = voiceEngine;
  }
  const activeVoiceEngine = voiceEngineRef.current ?? voiceEngine;
  useEffect(() => {
    const warning = workspace?.settings_warning ?? null;
    if (warning === null || warningShownRef.current) return;
    warningShownRef.current = true;
    setSettingsWarning(warning);
  }, [workspace]);
  // De to fejlkanaler flettes FØRST her, ved render-sømmen: Hud'en kender kun
  // ét error-felt, og der er højst én fejl live ad gangen (kanalerne rydder
  // hinanden — nyeste vinder), så `??` er blot en defensiv rangorden.
  const hudState: HudState =
    backendError === null ? hud : { ...hud, error: hud.error ?? backendError };
  // Den permanente voice-status bæres af orben i bundbåndet (chippen i
  // topbaren er nedlagt); hudView leverer fortsat session-labelen til aria.
  const hudView = deriveHudView(hudState, 0);

  useEffect(() => {
    if (!dryRun) {
      captureReadyRef.current = Promise.resolve(null);
      setCapturePath(null);
      return;
    }
    const ready = invoke<string>("reset_voice_capture")
      .then((path) => {
        setCapturePath(path);
        return path;
      })
      .catch((error) => {
        setHud((current) => ({
          ...current,
          error: `Dry-run capture kunne ikke startes: ${String(error)}`,
        }));
        return null;
      });
    captureReadyRef.current = ready;
  }, [dryRun]);

  useEffect(() => {
    if (!voiceReady) return;
    let disposed = false;
    let unregisterWake: (() => void) | null = null;
    let unregisterDictation: (() => void) | null = null;
    const player = createPcmPlayer();
    playerRef.current = player;
    const sounds = createSoundPlayer();

    const updateHud = (patch: Partial<HudState>) => {
      if (disposed) return;
      if (typeof patch.error === "string" && patch.error !== "") {
        // Fejl-blus (orb) og "eh-eh"-lyd deler chokepunkt, så øje og øre
        // altid fortæller det samme.
        setErrorTick((tick) => tick + 1);
        sounds.play("error");
        // Nyeste fejl vinder: en ægte NY voice-fejl afløser en stående
        // backend-fejl, så fejl aldrig bliver udødelige. Et `error: null`
        // rører derimod ALDRIG backend-kanalen — det er hele pointen med
        // de to kanaler (T9-review).
        setBackendError(null);
      }
      setHud((current) => ({ ...current, ...patch }));
    };

    // Ny tur = ren HUD. Dette er operatørens vej tilbage fra en stående
    // backend-fejl: den er sticky mod rutine-success, ikke evig. Bruges kun
    // dér hvor en NY tur reelt begynder (transcript, wake) — ikke ved
    // effekt-remounts, som ellers ville viske en frisk fejl ud.
    const startFreshTurn = (patch: Partial<HudState>) => {
      if (disposed) return;
      setBackendError(null);
      updateHud(patch);
    };

    // Mic-niveau til orben: chunks der alligevel flyder til STT, metered som
    // ren sideeffekt — én kilde, begge motorer. Lydfeedbacken ligger YDERST
    // om samme søm (Redaptings mikrofon-sandheds-princip): start-blip når
    // mikrofonen reelt er åben, stop-blip når den er lukket.
    const meteredCapture = withLevelMeter(startBrowserCapture, (level) => {
      micLevelRef.current = { level, at: performance.now() };
    });
    const feedbackCapture = withSoundFeedback(meteredCapture, sounds.play);

    const canvas: CanvasController = {
      focusCard(card) {
        controllerRef.current?.focusCard(card);
      },
      prepareFreshSpawn(card) {
        controllerRef.current?.prepareFreshSpawn(card);
      },
      getFocusedCard() {
        return controllerRef.current?.getFocusedCard() ?? null;
      },
    };

    function handleHudEvent(event: DispatchHudEvent) {
      switch (event.kind) {
        case "error":
          updateHud({ error: event.message });
          return;
        case "info":
          updateHud({ responseText: event.message, error: null });
          return;
        case "status":
          updateHud({ responseText: event.message, error: null });
      }
    }

    const getProject = () =>
      invoke<{ root: string; name: string }>("get_project");

    const dispatcher = createVoiceDispatcher({
      canvas,
      getProject,
      onHudEvent: handleHudEvent,
      onCardsClosing(numbers) {
        if (PERF_ENABLED) {
          const trace = getActiveVoiceTrace();
          markPerf(trace, "close.frontend.voice_optimistic_remove", { numbers });
        }
        const closing = new Set(numbers);
        setCards((current) =>
          current?.filter((card) => !closing.has(card.number)) ?? current,
        );
      },
      onWorkspaceMutation: refresh,
    });
    const dryRunDispatch = createDryRunDispatch({
      getCards: () => cardsRef.current,
      getFocusedCard: () => canvas.getFocusedCard(),
      getProject,
    });

    // Svar-stemmen er 100 % pre-genererede klip (kommandosæt v4, spec §5) —
      // realtime-TTS forlod pipeline-svarvejen. Klippene loader async ved
      // mount (14 små filer, lokal asset); manglende klip = stilhed + HUD.
      const clipAssets = new Map<string, ArrayBuffer>();
      const clipLoadTrace = PERF_ENABLED
        ? startPerfTrace("voice", { entry: "clip_asset_load" })
        : null;
      if (PERF_ENABLED) {
        markPerf(clipLoadTrace, "voice.clip_assets.load.begin");
      }
      void loadReplyClips().then((loaded) => {
        for (const [key, buffer] of loaded) clipAssets.set(key, buffer);
        if (PERF_ENABLED) {
          markPerf(clipLoadTrace, "voice.clip_assets.load.end", {
            asset_count: clipAssets.size,
            pcm_bytes: [...clipAssets.values()].reduce(
              (sum, buffer) => sum + buffer.byteLength,
              0,
            ),
          });
          void flushPerfTrace(clipLoadTrace, true);
        }
      });
      const tts = createClipTts({ assets: clipAssets, player });
      const pipeline = createPipelineVoiceSession({
        stt: () => {
          const route = voiceRoutesRef.current?.stt;
          if (!route) throw new Error("Voice-ruten er ikke indlæst endnu");
          return createOpenAiSttClient({
            model: route.model,
            endpoint: route.endpoint,
            prompt: route.supports_domain_prompt ? STT_DOMAIN_PROMPT : null,
          });
        },
        routeHasPartials: () =>
          voiceRoutesRef.current?.stt.supports_partials ?? true,
        onTurnStart: () =>
          startFreshTurn({
            transcript: "",
            responseText: "",
            tool: null,
            resolver: null,
            error: null,
            chain: null,
          }),
        maxUtteranceMs: MAX_UTTERANCE_MS,
        dispatch: dispatcher.dispatch,
        dryRunDispatch,
        getDryRun: () => dryRunRef.current,
        speak: tts.speak,
        startCapture: feedbackCapture,
        onState(state) {
          updateHud({ session: pipelineSessionForHud(state) });
        },
        onTranscript(transcript) {
          startFreshTurn({
            transcript,
            responseText: "",
            tool: null,
            resolver: null,
            error: null,
            chain: null,
          });
        },
        onToolCall(call) {
          updateHud({
            tool: { name: call.name, arguments: call.arguments },
            error: null,
          });
        },
        onDispatchResult(result) {
          updateHud({ resolver: resolverFromResult(result) });
        },
        onChainResult(commands) {
          updateHud({
            chain: commands.map(({ intent, result }) => ({
              name: intent.kind,
              ok: result.ok,
              label: result.message,
            })),
          });
        },
        onResponseText(text) {
          updateHud({ responseText: text });
        },
        onLatency(milliseconds) {
          console.debug("voice.pipeline.latency_ms", milliseconds);
        },
        async onTurnComplete(entry) {
          const resolver = entry.resolver as {
            dry_run?: boolean;
            action_count?: number;
          } | null;
          if (resolver?.dry_run !== true || resolver.action_count !== 0) return;
          await captureReadyRef.current;
          const path = await invoke<string>("append_voice_capture", { entry });
          if (!disposed) setCapturePath(path);
        },
        onError(error) {
          updateHud({ error: error.message });
        },
      });

      // Diktering: samme mikrofon og samme STT som pipelinen, men uden
      // router, dispatch og svarklip. Ordene gaar raat ind i det fokuserede
      // korts composer — se voice/dictation.ts.
      const dictation = createDictationSession({
        stt: () => {
          const route = voiceRoutesRef.current?.stt;
          if (!route) throw new Error("Voice-ruten er ikke indlæst endnu");
          return createOpenAiSttClient({
            model: route.model,
            endpoint: route.endpoint,
            prompt: route.supports_domain_prompt ? STT_DOMAIN_PROMPT : null,
          });
        },
        // SAMME capture-wrapper som pipelinen: uden den staar orben stille og
        // der er intet start/stop-blip, saa brugeren ikke kan se at
        // mikrofonen er aaben (mikrofon-sandheds-princippet ovenfor).
        startCapture: feedbackCapture,
        maxUtteranceMs: MAX_UTTERANCE_MS,
        warm: () => {
          void invoke("warm_voice_connections").catch(() => undefined);
        },
        onState(state) {
          updateHud({ session: state === "idle" ? "idle" : "listening" });
        },
        onTranscript(text) {
          startFreshTurn({
            transcript: text,
            responseText: "",
            tool: null,
            resolver: null,
            error: null,
            chain: null,
          });
        },
        onEmpty() {
          updateHud({ responseText: "Ingen lyd fanget", error: null });
        },
        onError(error) {
          updateHud({ error: error.message });
        },
        async insert(text) {
          const target = resolveDictationTarget({
            chatInputThread: focusedChatThread(),
            focusedCard: canvas.getFocusedCard(),
            cards: cardsRef.current,
          });
          if (target.kind === "none") {
            updateHud({ error: describeBlockedTarget(target.reason) });
            return;
          }
          const submit = dictationSubmitRef.current;
          if (target.kind === "terminal" && submit) {
            // submit_prompt skriver SELV teksten og ejer koreografien med
            // tekst + `\r` som to writes (submit.rs). Et paste foerst ville
            // lande ordene to gange. Fejlkontrakten er vaerd at kende: den
            // venter op til 15 s paa at agentens prompt er klar, og staar i
            // koe bag en igangvaerende voice-dispatch.
            await invoke("submit_prompt", { name: target.name, text });
            updateHud({ responseText: `Sendt til ${target.name}`, error: null });
            return;
          }
          emitDictationInsert({
            name: target.name,
            text,
            submit: target.kind === "chat" && submit,
          });
        },
      });

      updateHud({ session: "idle", error: null });

      // To genveje, én mikrofon. Reglerne — og hvorfor gaten ikke maa vaere
      // "er den anden session idle" — bor i voice/micArbiter.ts.
      const mic = createMicArbiter();

      const pttBridge = createHotkeyBridge({
        onPress() {
          if (!mic.acquire("ptt")) return;
          pipeline.press();
        },
        onRelease() {
          if (!mic.release("ptt")) return;
          pipeline.release();
        },
        onDebug: (message) => console.debug(message),
        onWarn: (message) => console.warn(message),
      });

      const dictationBridge = createHotkeyBridge({
        onPress() {
          if (!mic.acquire("dictation")) return;
          // Spiller pipelinen stadig et svarklip, skal det tie: brugeren vil
          // tale NU. Samme hoeflighed som pipeline.press() viser sig selv.
          if (pipeline.state() === "speaking") pipeline.cancel();
          dictation.press();
        },
        onRelease() {
          if (!mic.release("dictation")) return;
          dictation.release();
        },
        onDebug: (message) => console.debug(message),
        onWarn: (message) => console.warn(message),
      });

      let unlistenNativePtt: UnlistenFn | null = null;
      // Hver binding faar sit EGET try/catch. Laa de i samme blok, ville en
      // ulaeselig alternativ binding ogsaa rive den primaeres DOM-registrering
      // med sig — og den primaere er den brugeren faktisk trykker paa.
      const registerBinding = (
        accel: string | null,
        binding: number,
        bridge: { domPress(b?: number): void; domRelease(b?: number): void },
        label: string,
      ): (() => void) | null => {
        if (!accel) return null;
        try {
          if (!isKeyboardBinding(accel)) return null;
          return registerPttKey(accel, {
            onPress: () => bridge.domPress(binding),
            onRelease: () => bridge.domRelease(binding),
          });
        } catch (error) {
          updateHud({ error: `${label} fejlede: ${String(error)}` });
          return null;
        }
      };
      const unregisterWakeAlt = registerBinding(
        voiceHotkeyAlt,
        1,
        pttBridge,
        "Alternativ voice-hotkey",
      );
      const unregisterDictationAlt = registerBinding(
        dictationHotkeyAlt,
        1,
        dictationBridge,
        "Alternativ diktér-hotkey",
      );
      unregisterWake = registerBinding(
        voiceHotkey,
        0,
        pttBridge,
        "Voice-hotkey",
      );
      unregisterDictation = registerBinding(
        dictationHotkey,
        0,
        dictationBridge,
        "Diktér-hotkey",
      );

      void (async () => {
        try {
          // Lytteren registreres FOER polleren startes (reviewer-P2: et tryk i
          // handoff-vinduet maa aldrig tabes — det er praecis "foerste tryk er
          // doedt"-symptomet polleren skal dræbe). Selve arbitrationen mellem
          // DOM og poller bor i voice/hotkeyBridge.ts, hvor den er testet.
          const unlisten = await listen("wake-hotkey", (event) => {
            const payload = event.payload as {
              combo?: string;
              edge?: string;
            };
            if (payload.edge !== "press" && payload.edge !== "release") return;
            if (payload.combo === "ptt") pttBridge.nativeEdge(payload.edge);
            else if (payload.combo === "dictation") {
              dictationBridge.nativeEdge(payload.edge);
            }
          });
          if (disposed) {
            unlisten();
            return;
          }
          unlistenNativePtt = unlisten;
          await invoke("configure_wake_hotkey", {
            accel: voiceHotkey,
            alt: voiceHotkeyAlt,
          });
          pttBridge.markNativeReady();
          await invoke("configure_dictation_hotkey", {
            accel: dictationHotkey,
            alt: dictationHotkeyAlt,
          });
          dictationBridge.markNativeReady();
        } catch (error) {
          updateHud({
            error: `Wake-hotkey kunne ikke konfigureres: ${String(error)}`,
          });
        }
      })();

      const onVoiceBlur = () => {
        const pipelineState = pipeline.state();
        if (pipelineState === "listening" || pipelineState === "finalizing") {
          pipeline.cancel();
          mic.release("ptt");
        }
        // Uden den her stod et diktér-hold der mistede fokus i "listening"
        // med mikrofonen aaben. registerPttKey's egen onBlur daekker kun
        // DOM-vejen — polleren har ingen.
        const dictationState = dictation.state();
        if (dictationState === "listening" || dictationState === "finalizing") {
          dictation.cancel();
          mic.release("dictation");
        }
      };
      window.addEventListener("blur", onVoiceBlur);

    return () => {
      disposed = true;
      playerRef.current = null;
      unlistenNativePtt?.();
      unregisterWake?.();
      unregisterDictation?.();
      unregisterWakeAlt?.();
      unregisterDictationAlt?.();
      window.removeEventListener("blur", onVoiceBlur);
      dictation.cancel();
      void sounds.close();
      void pipeline.stop().finally(() => player.close());
    };
  }, [
    activeVoiceEngine,
    dictationHotkey,
    dictationHotkeyAlt,
    refresh,
    voiceHotkey,
    voiceHotkeyAlt,
    voiceReady,
  ]);

  return (
    <AppShell
      rail={
        <MemoizedWorkspaceRail
          workspaces={workspaces}
          showHidden={showHidden}
          onActivate={activateWorkspace}
          onClose={closeWorkspace}
          onToggleHidden={setWorkspaceHidden}
          onAdd={addWorkspace}
          onShowHiddenChange={setShowHidden}
          onOpenSettings={openSettings}
          settingsOpen={settingsOpen}
        />
      }
    >
      <main style={styles.main}>
        {PERF_ENABLED ? <PerfHarnessInstaller refresh={refresh} /> : null}
        <style>{globalCss}</style>
        {/* Frameless vindue (decorations: false): topbaren ER titelbjælken.
            Drag-region-attributten sidder på baren OG projekt-elementerne
            (Tauri starter kun drag fra elementer der selv bærer attributten);
            interaktive børn (Indstillinger, vindueskontroller) bærer den ikke
            og forbliver klikbare. Dobbeltklik på drag-region = maksimér. */}
        <header data-global-topbar data-tauri-drag-region="true" style={styles.topbar}>
          <div
            style={styles.projectGroup}
            title={project?.root}
            data-tauri-drag-region="true"
          >
            <span style={styles.projectMark} aria-hidden="true" data-tauri-drag-region="true" />
            <span style={styles.projectName} data-tauri-drag-region="true">
              {project?.name ?? "Workspace"}
            </span>
            {project?.root && (
              <span style={styles.projectPath} data-tauri-drag-region="true">
                {project.root}
              </span>
            )}
          </div>

          <WindowControls closeDisabled={closeDialogOpen} />
        </header>

        {loadError !== null && (
          <div style={styles.notice}>workspace-load fejlede: {loadError}</div>
        )}
        {settingsWarning !== null && (
          <div style={styles.notice}>
            {settingsWarning}{" "}
            <button type="button" onClick={() => setSettingsWarning(null)}>
              Luk advarsel
            </button>
          </div>
        )}
        {loadError === null && cards !== null && cards.length === 0 && cardsStatus?.error != null && (
          <div style={styles.notice}>
            {`cards.toml er defekt: ${cardsStatus.error}`}
          </div>
        )}
        {cards !== null && workspace !== null && (
          <CanvasSurface
            ref={controllerRef}
            cards={cards}
            wallpaper={workspace.settings.wallpaper}
            onWorkspaceMutation={refresh}
          />
        )}

        {/* Bund-dock (ejer-beslutning 2026-07-22): voice-chippen nederst til
            HØJRE — modsat UsageHud'en i bundvenstre. Folder OPAD on-demand.
            Indstillinger LAA her indtil 2026-07-29; tandhjulet bor nu i
            rail'ens footer, fordi indholdet er app-kram og ikke sessionsstatus.
            Chippen blev: den beskriver orben 18 px derfra. */}
        <div data-bottom-dock style={styles.bottomDock}>
          <Hud state={hudState} />
        </div>

        {/* Indstillings-vinduet. Bor i <main> — altsaa i canvas-zonen — saa
            dets daempning daekker praecis kortarealet og LADER rail'en staa
            aktiv (ejer-valg 2026-07-29). Altid monteret; se SettingsWindow. */}
        <SettingsWindow
          open={settingsOpen}
          onClose={closeSettings}
          dryRun={dryRun}
          onDryRunChange={setDryRun}
          dryRunForced={DRY_RUN_FORCED}
          /* Fejlfindings-blokken er skrevet til udvikling, ikke til brug:
             "action_count=0" siger ikke noget til nogen udefra. `DEV` er
             falsk i `vite build` — altsaa i den udsendte binaer — og
             env-flaget daekker en bevidst bygget eval-version. */
          showDebug={DEV_BUILD || DRY_RUN_FORCED}
          capturePath={capturePath}
          onSaved={refresh}
        />

        <UsageHud cards={cards} />

        <Orb
          session={hud.session}
          errorTick={errorTick}
          sessionLabel={hudView.sessionLabel}
          getMicLevel={() => micLevelRef.current}
          getOutputLevel={() => playerRef.current?.getOutputLevel() ?? 0}
        />

        {/* Cast-straalen (spec 2026-07-22): orb->kort-beam ved send_prompt/
            new_card/open_browser. Ren pynt-overlay under orben (z 39). */}
        <CastLayer />

        {/* Bekraeftelsen er modal for HELE vinduet, rail'en inklusive —
            rail'ens ✕ er selv en af de veje der aabner den. Den bor alligevel
            HER i main og ikke som sibling til AppShell, af samme grund som
            CastLayer: laget er `position: fixed`, og ingen forfader er
            containing block for fixed (ingen transform/filter/contain), saa
            det daekker vinduet og ikke kun canvas-zonen. */}
        <CloseWorkspaceDialog
          /* `key` er ikke pynt: to anmodninger i traek maa ALDRIG reconcile til
             den samme knap. Uden den skifter en destruktiv knap betydning under
             en markoer der allerede er paa vej ned (fix-runde 1, fund 2). */
          key={closeRequest?.id ?? "tom"}
          request={
            closeRequest === null
              ? null
              : closeRequest.kind === "application"
                ? {
                    kind: "application",
                    workspaces: closeRequest.workspaces,
                    running: closeRequest.running,
                  }
                : {
                    kind: "workspace",
                    workspaceName: closeRequest.name,
                    running: closeRequest.running,
                  }
          }
          restoreFocusTo={closeRequest?.restoreFocus ?? null}
          onConfirm={confirmClose}
          onCancel={cancelClose}
        />
      </main>
    </AppShell>
  );
}

const globalCss = `
  html, body, #root { margin: 0; padding: 0; height: 100vh; overflow: hidden; }
  body { background: #0d0f12; color: #d4d4d4;
         font-family: "Segoe UI", system-ui, sans-serif; }
  /* Frameless drag: WebView2 håndterer app-region NATIVT (wry sætter
     IsNonClientRegionSupportEnabled) — data-tauri-drag-region-attributterne
     er kun fallback for platforme uden non-client-support. VIGTIGT:
     interaktive børn SKAL være no-drag, ellers æder den native drag-flade
     deres klik. */
  [data-global-topbar] { -webkit-app-region: drag; }
  [data-global-topbar] button { -webkit-app-region: no-drag; }
`;

const styles: Record<string, CSSProperties> = {
  // `absolute`, ikke `fixed`: main bor i AppShell's viewport-zone
  // (position: relative), saa inset:0 betyder "canvas-zonen" og ikke "hele
  // vinduet". Uden det ville alt herinde flyde hen over rail'en.
  main: { position: "absolute", inset: 0, overflow: "hidden" },
  // Usynlig container (ejer-valg 2026-07-20): geometri/layout er bevaret
  // 1:1 (drag-fladen og elementernes positioner er uændrede) — kun den
  // synlige pille (kant/baggrund/skygge/glas) er fjernet.
  topbar: {
    // fixed -> absolute: ankres til canvas-zonen (main), ikke til vinduet.
    position: "absolute",
    top: TOPBAR_TOP,
    left: 18,
    right: 18,
    zIndex: 60,
    height: TOPBAR_HEIGHT,
    display: "flex",
    alignItems: "center",
    gap: 14,
    boxSizing: "border-box",
    padding: "0 7px 0 12px",
    color: "#b9c8d9",
    userSelect: "none",
  },
  projectGroup: {
    display: "flex",
    alignItems: "center",
    gap: 8,
    minWidth: 0,
    flex: "1 1 auto",
  },
  projectMark: {
    width: 8,
    height: 8,
    flex: "0 0 auto",
    borderRadius: 3,
    background: "#7ab6e8",
    boxShadow: "0 0 12px rgba(89, 171, 236, 0.44)",
  },
  projectName: {
    color: "#edf5fc",
    fontSize: 12,
    fontWeight: 600,
    whiteSpace: "nowrap",
  },
  projectPath: {
    minWidth: 0,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    color: "#718297",
    fontFamily: '"Cascadia Mono", monospace',
    fontSize: 10,
  },
  notice: {
    position: "absolute",
    top: TOPBAR_CLEARANCE + TOPBAR_TOP,
    left: 18,
    right: 18,
    zIndex: 10,
    padding: "10px 16px",
    border: "1px solid rgba(221, 240, 255, 0.2)",
    borderRadius: 12,
    color: "#aebdd0",
    fontSize: 14,
    background:
      "linear-gradient(135deg, rgba(48, 76, 108, 0.28), rgba(3, 11, 23, 0.3))",
    boxShadow:
      "0 18px 46px rgba(0, 7, 24, 0.36), inset 0 1px 0 rgba(240, 249, 255, 0.22)",
    backdropFilter: FROSTED_BACKDROP,
    WebkitBackdropFilter: FROSTED_BACKDROP,
  },
  // Bund-dock: bundhøjre-hjørnet af CANVAS-ZONEN (modsat UsageHud'ens
  // left:18/bottom:24) med voice-chippen og Indstillinger som knap-række.
  // zIndex 50 = samme lag som Hud'en havde i topzonen (over orb-dockens 40,
  // under topbarens 60). fixed -> absolute af samme grund som topbaren.
  bottomDock: {
    position: "absolute",
    right: 18,
    bottom: 24,
    zIndex: 50,
    display: "flex",
    alignItems: "center",
    gap: 8,
  },
};
