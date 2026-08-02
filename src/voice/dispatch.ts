import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { CanvasController } from "../CanvasSurface";
import { isBrowserCard, type CardInfo } from "../types";
import { emitCast } from "./cast";
import {
  resolveTarget,
  resolveTargets,
  type VoiceIntent,
} from "./intents";
import {
  getActiveVoiceTrace,
  markPerf,
  perfInvokeArgs,
  registerPendingCreate,
  registerPendingClose,
} from "../perfTrace";

export type TauriInvoke = (
  command: string,
  args?: Record<string, unknown>,
) => Promise<unknown>;

export type DispatchHudEvent =
  | { kind: "error"; message: string; rawTranscript?: string }
  | { kind: "info"; message: string }
  | { kind: "status"; message: string; card: number; data: unknown };

export type DispatchResult =
  | {
      ok: true;
      kind: VoiceIntent["kind"];
      message: string;
      card?: number;
      cards?: number[];
      data?: unknown;
    }
  | { ok: false; code: string; message: string };

export interface DispatchDeps {
  invoke?: TauriInvoke;
  canvas: CanvasController;
  getProject(): Promise<{ root: string; name: string }>;
  onHudEvent(event: DispatchHudEvent): void;
  onCardsClosing?(cards: number[]): void;
  onWorkspaceMutation?: () => void | Promise<void>;
}

export interface VoiceDispatcher {
  dispatch(intent: VoiceIntent | null, rawTranscript?: string): Promise<DispatchResult>;
}

const TARGET_ERRORS = {
  ambiguous_focus: "Intet entydigt fokuseret kort",
  no_such_card: "Kortet findes ikke",
  no_target: "Kommandoen mangler et kortnummer",
} as const;

const SITE_HINTS = {
  github: "https://github.com",
  google: "https://www.google.com",
} as const;

function defaultInvoke(command: string, args?: Record<string, unknown>) {
  return tauriInvoke(command, args);
}

function failure(code: string, message: string): DispatchResult {
  return { ok: false, code, message };
}

export function createVoiceDispatcher({
  invoke = defaultInvoke,
  canvas,
  getProject,
  onHudEvent,
  onCardsClosing,
  onWorkspaceMutation,
}: DispatchDeps): VoiceDispatcher {
  async function invokeMeasured(
    command: string,
    args?: Record<string, unknown>,
  ): Promise<unknown> {
    const trace = PERF_ENABLED ? getActiveVoiceTrace() : null;
    const started = PERF_ENABLED ? performance.now() : 0;
    if (PERF_ENABLED) {
      markPerf(trace, "voice.dispatch.invoke.begin", { command });
    }
    const tracedArgs =
      PERF_ENABLED &&
      trace &&
      (command === "create_card" || command === "close_cards")
        ? { ...args, ...perfInvokeArgs(trace) }
        : args;
    try {
      const result =
        tracedArgs === undefined
          ? await invoke(command)
          : await invoke(command, tracedArgs);
      if (PERF_ENABLED) {
        markPerf(trace, "voice.dispatch.invoke.end", {
          command,
          duration_ms: performance.now() - started,
          ok: true,
        });
      }
      return result;
    } catch (error) {
      if (PERF_ENABLED) {
        markPerf(trace, "voice.dispatch.invoke.end", {
          command,
          duration_ms: performance.now() - started,
          ok: false,
          error: String(error),
        });
      }
      throw error;
    }
  }

  async function listCards(): Promise<CardInfo[]> {
    const value = await invokeMeasured("list_cards");
    if (!Array.isArray(value)) throw new Error("list_cards returnerede et ugyldigt svar");
    return value as CardInfo[];
  }

  function emitFailure(code: keyof typeof TARGET_ERRORS, rawTranscript?: string) {
    const message = TARGET_ERRORS[code];
    onHudEvent({ kind: "error", message, ...(rawTranscript ? { rawTranscript } : {}) });
    return failure(code, message);
  }

  function rejectBrowserCard(card: CardInfo): DispatchResult | null {
    if (!isBrowserCard(card)) return null;
    // Fanges i TS FØR Rust-kaldet — ellers taler exception-vejen den rå
    // engelske Rust-fejl ("card is a browser: card-N").
    const message = `Kort ${card.number} er en browser`;
    onHudEvent({ kind: "error", message });
    return failure("browser_card", message);
  }

  async function targetCard(
    intent: VoiceIntent,
  ): Promise<{ card: CardInfo } | { result: DispatchResult }> {
    const cards = await listCards();
    const resolved = resolveTarget(intent, canvas.getFocusedCard(), cards);
    if (!resolved.ok) return { result: emitFailure(resolved.reason) };
    const card = cards.find(({ number }) => number === resolved.card);
    return card
      ? { card }
      : { result: emitFailure("no_such_card") };
  }

  async function executeRestartCard(
    intent: Extract<VoiceIntent, { kind: "restart_card" }>,
  ): Promise<DispatchResult> {
    const target = await targetCard(intent);
    if ("result" in target) return target.result;
    const browserReject = rejectBrowserCard(target.card);
    if (browserReject) return browserReject;
    try {
      if (target.card.running) {
        await invokeMeasured("kill_card", { name: target.card.name });
      }
      // Clear the old xterm synchronously after kill and before spawn. A
      // snapshot-driven reset is too late: it can erase the new alt-screen.
      canvas.prepareFreshSpawn(target.card.number);
      // Voice "genstart" is deliberately fresh. Explicit session recovery
      // remains available only through Card's Fortsæt (--continue) button.
      await invokeMeasured("spawn_card", { name: target.card.name });
    } finally {
      await onWorkspaceMutation?.();
    }
    const message = `Kort ${target.card.number} blev genstartet`;
    onHudEvent({ kind: "info", message });
    return { ok: true, kind: intent.kind, card: target.card.number, message };
  }

  async function executeCloseCards(
    intent: Extract<VoiceIntent, { kind: "close_cards" }>,
  ): Promise<DispatchResult> {
    const cards = await listCards();
    const resolved = resolveTargets(intent, cards);
    if (!resolved.ok) {
      const message =
        resolved.reason === "no_target"
          ? "Kommandoen mangler eksplicitte kortnumre"
          : resolved.reason === "no_cards"
            ? "Ingen åbne kort at lukke"
            : TARGET_ERRORS[resolved.reason];
      onHudEvent({ kind: "error", message });
      return failure(resolved.reason, message);
    }
    const selected = resolved.cards.map((number) =>
      cards.find((card) => card.number === number),
    );
    if (selected.some((card) => card === undefined)) return emitFailure("no_such_card");
    const selectedCards = selected as CardInfo[];
    onHudEvent({
      kind: "info",
      message: `Lukker ${selectedCards.length} kort…`,
    });
    if (PERF_ENABLED) {
      const perfTrace = getActiveVoiceTrace();
      for (const card of selectedCards) {
        registerPendingClose(card.name, perfTrace);
      }
    }
    onCardsClosing?.(resolved.cards);
    let batch: unknown;
    try {
      batch = await invokeMeasured("close_cards", {
        names: selectedCards.map((card) => card.name),
      });
    } finally {
      await onWorkspaceMutation?.();
    }
    if (
      !batch ||
      typeof batch !== "object" ||
      !Array.isArray((batch as { closed?: unknown }).closed) ||
      !Array.isArray((batch as { errors?: unknown }).errors)
    ) {
      throw new Error("close_cards returnerede et ugyldigt svar");
    }
    const errors = (batch as {
      errors: Array<{ name?: unknown; message?: unknown }>;
    }).errors;
    if (errors.length > 0) {
      const details = errors
        .map(({ name, message }) => `${String(name)}: ${String(message)}`)
        .join("; ");
      throw new Error(`Nogle kort kunne ikke lukkes: ${details}`);
    }
    const message = `Kort ${resolved.cards.join(", ")} blev lukket`;
    onHudEvent({ kind: "info", message });
    return { ok: true, kind: "close_cards", cards: resolved.cards, message };
  }

  return {
    async dispatch(intent, rawTranscript) {
      if (!intent) {
        const message = "Kommandoen kunne ikke fortolkes";
        onHudEvent({
          kind: "error",
          message,
          ...(rawTranscript ? { rawTranscript } : {}),
        });
        return failure("router_reject", message);
      }

      switch (intent.kind) {
        case "send_prompt": {
          const target = await targetCard(intent);
          if ("result" in target) return target.result;
          const browserReject = rejectBrowserCard(target.card);
          if (browserReject) return browserReject;
          await invokeMeasured("submit_prompt", {
            name: target.card.name,
            text: intent.text,
          });
          // Cast-straalen (spec 2026-07-22): kun efter succes — fejlveje
          // emitter aldrig. emitCast kan ikke kaste (bussen sluger).
          // landing "prompt": straalen lander i terminalens tekstfelt.
          emitCast({ card: target.card.number, landing: "prompt" });
          const message = `Prompt sendt til kort ${target.card.number}`;
          onHudEvent({ kind: "info", message });
          return { ok: true, kind: intent.kind, card: target.card.number, message };
        }
        case "new_card": {
          const cwd = (await getProject()).root;
          const count = intent.count ?? 1;
          if (!Number.isInteger(count) || count < 1 || count > 10) {
            const message = "Antallet af nye kort skal være mellem 1 og 10";
            onHudEvent({ kind: "error", message });
            return failure("invalid_count", message);
          }
          // Choke-pointet (Task 5): profile udelades som default, saa
          // create_card's backend-side default_agent-opslag afgoer agenten.
          // Kun kendte slugs sendes eksplicit — en ukendt streng (krydser
          // voice-model-graensen fra routeren) behandles tolerant som
          // fravaer + advarsel (spec N2) fremfor at fejle kaeden.
          const agent =
            intent.agent === "claude" || intent.agent === "codex"
              ? intent.agent
              : undefined;
          if (intent.agent && !agent) {
            onHudEvent({
              kind: "info",
              message: `ukendt agent "${intent.agent}" — bruger standard`,
            });
          }
          const created: CardInfo[] = [];
          try {
            for (let index = 0; index < count; index += 1) {
              const card = (await invokeMeasured("create_card", {
                cwd,
                command: null,
                ...(agent ? { profile: agent } : {}),
              })) as CardInfo;
              created.push(card);
              if (PERF_ENABLED) {
                registerPendingCreate(card.name, getActiveVoiceTrace());
              }
            }
          } finally {
            // Hvis et senere create fejler, skal allerede oprettede kort stadig
            // ind i gridet med det samme.
            if (created.length > 0) await onWorkspaceMutation?.();
          }
          const numbers = created.map((card) => card.number);
          // Cast-straalerne affyres foerst EFTER onWorkspaceMutation (finally
          // ovenfor), saa kortene kan findes i DOM'en — CastLayer ejer
          // vifte-forskydningen naar flere emits kommer i samme tick.
          for (const number of numbers)
            emitCast({ card: number, landing: "card" });
          const message = agent
            ? `${created.length} ${agent}-kort oprettet i ${cwd}`
            : `${created.length} kort oprettet i ${cwd}`;
          onHudEvent({ kind: "info", message });
          return { ok: true, kind: intent.kind, cards: numbers, message };
        }
        case "restart_card":
          return executeRestartCard(intent);
        case "close_cards":
          return executeCloseCards(intent);
        case "open_browser": {
          const url = intent.url_hint ? SITE_HINTS[intent.url_hint] : null;
          const card = (await invokeMeasured("create_browser_card", {
            url,
            openedBy: null,
          })) as CardInfo;
          await onWorkspaceMutation?.();
          emitCast({ card: card.number, landing: "card" });
          const message = `Browser åbnet som kort ${card.number}`;
          onHudEvent({ kind: "info", message });
          return { ok: true, kind: intent.kind, card: card.number, message };
        }
      }
    },
  };
}
