import { invoke } from "@tauri-apps/api/core";

import type { DispatchResult, VoiceDispatcher } from "./dispatch";
import type { DryRunResult } from "./dryRun";
import type { VoiceIntent } from "./intents";
import { createPtt, startBrowserCapture, type StartAudioCapture } from "./ptt";
import {
  type RealtimeToolCall,
  type RealtimeTurnCapture,
} from "./turnTypes";
import {
  templateReplySource,
  type ActionOrigin,
  type Reply,
  type ReplySource,
  type TurnOutcome,
} from "./replies";
import { routeVoiceTranscript } from "./router";
import type { SttClient } from "./stt";
import { createOpenAiTts, type TtsPlayback } from "./tts";
import {
  beginVoiceTrace,
  finishVoiceTrace,
  flushPerfTrace,
  getActiveVoiceTrace,
  markPerf,
} from "../perfTrace";

export type PipelineUiState =
  | "idle"
  | "listening"
  | "finalizing"
  | "processing"
  | "speaking";

type ToolTurn = {
  transcript: string;
  call: RealtimeToolCall;
  resolver: DispatchResult | DryRunResult;
  commands?: Array<{
    tool: RealtimeToolCall;
    resolver: DispatchResult | DryRunResult;
  }>;
  command_count?: number;
};

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}

function intentArguments(intent: VoiceIntent): Record<string, unknown> {
  switch (intent.kind) {
    case "send_prompt":
      return { card: intent.card, text: intent.text };
    case "new_card":
      return {
        count: intent.count,
        ...(intent.agent ? { agent: intent.agent } : {}),
      };
    case "close_cards":
      return {
        cards: intent.cards,
        ...(intent.all === true ? { all: true } : {}),
      };
    case "restart_card":
      return { card: intent.card };
    case "open_browser":
      return { url_hint: intent.url_hint };
  }
}

export function createPipelineVoiceSession(deps: {
  stt: () => SttClient;
  routeHasPartials?: () => boolean;
  onTurnStart?: () => void;
  maxUtteranceMs?: number;
  route?: typeof routeVoiceTranscript;
  dispatch: VoiceDispatcher["dispatch"];
  dryRunDispatch?: (
    intent: VoiceIntent,
    rawTranscript?: string,
  ) => Promise<DryRunResult>;
  getDryRun?: () => boolean;
  speak?: (reply: Reply) => TtsPlayback;
  replyFor?: ReplySource;
  startCapture?: StartAudioCapture;
  onState?(s: PipelineUiState): void;
  onTranscript?(text: string): void;
  onToolCall?(call: {
    name: string;
    arguments: Record<string, unknown>;
  }): void;
  onDispatchResult?(result: DispatchResult | DryRunResult): void;
  onChainResult?(
    commands: Array<{
      intent: VoiceIntent;
      result: DispatchResult | DryRunResult;
    }>,
  ): void;
  onResponseText?(text: string): void;
  onError?(error: Error): void;
  onLatency?(ms: number): void;
  onTurnComplete?(
    entry: RealtimeTurnCapture & { origin: ActionOrigin },
  ): void | Promise<void>;
  warm?(): void;
}): {
  press(): void;
  release(): void;
  cancel(): void;
  stop(): Promise<void>;
  state(): PipelineUiState;
} {
  const stt = deps.stt;
  const route = deps.route ?? routeVoiceTranscript;
  const replyFor = deps.replyFor ?? templateReplySource;
  const tts = deps.speak ? null : createOpenAiTts();
  const speak = deps.speak ?? ((reply: Reply) => tts!.speak(reply));
  const startCapture = deps.startCapture ?? startBrowserCapture;
  // TLS-opvarmning ved PTT-tryk: mens brugeren taler, håndtrykkes gateway-
  // og OpenAI-forbindelserne, så router-/TTS-kaldene efter slip er varme
  // (~200-400 ms målt forskel). Best-effort — fejl er tavse.
  const warm =
    deps.warm ??
    (() => {
      void invoke("warm_voice_connections").catch(() => undefined);
    });

  let currentState: PipelineUiState = "idle";
  let generation = 0;
  let currentPtt: ReturnType<typeof createPtt> | null = null;
  let currentPlayback: TtsPlayback | null = null;
  let releasedAt: number | null = null;
  let maxDurationTimer: ReturnType<typeof setTimeout> | null = null;

  function isCurrent(token: number): boolean {
    return token === generation;
  }

  function transition(next: PipelineUiState) {
    if (currentState === next) return;
    currentState = next;
    deps.onState?.(next);
  }

  function elapsedSinceRelease(): number {
    if (releasedAt === null) {
      throw new Error("Pipeline tool-tur mangler release-tidspunkt");
    }
    return Math.max(0, performance.now() - releasedAt);
  }

  async function captureTurn(
    token: number,
    turn: ToolTurn,
    latencyMs: number,
  ) {
    if (!isCurrent(token)) return;
    try {
      await deps.onTurnComplete?.({
        ts: new Date().toISOString(),
        transcript: turn.transcript,
        tool: turn.call,
        resolver: turn.resolver,
        latency_ms: latencyMs,
        action_count: 0,
        origin: "voice",
        ...(turn.commands !== undefined
          ? { commands: turn.commands, command_count: turn.command_count }
          : {}),
      });
    } catch (error) {
      if (isCurrent(token)) deps.onError?.(asError(error));
    }
  }

  async function answer(
    token: number,
    outcome: TurnOutcome,
    toolTurn?: ToolTurn,
  ) {
    if (!isCurrent(token)) return;
    const perfTrace = PERF_ENABLED ? getActiveVoiceTrace() : null;
    const reply = replyFor(outcome);
    if (PERF_ENABLED) {
      markPerf(perfTrace, "voice.reply.selected", {
        outcome: outcome.kind,
        audio_key: reply.audioKey,
      });
    }
    deps.onResponseText?.(reply.text);
    if (!isCurrent(token)) return;

    transition("speaking");
    let playback: TtsPlayback;
    try {
      if (PERF_ENABLED) {
        markPerf(perfTrace, "voice.clip.speak.begin", {
          audio_key: reply.audioKey,
        });
      }
      playback = speak(reply);
    } catch (error) {
      if (!isCurrent(token)) return;
      deps.onError?.(asError(error));
      if (toolTurn) {
        await captureTurn(token, toolTurn, elapsedSinceRelease());
      }
      if (PERF_ENABLED) {
        markPerf(perfTrace, "voice.clip.speak.threw", {
          error: String(error),
        });
        finishVoiceTrace(perfTrace);
      }
      if (isCurrent(token)) transition("idle");
      return;
    }
    currentPlayback = playback;

    let latencyMs: number | null = null;
    let playbackErrorReported = false;
    const reportPlaybackError = (error: unknown) => {
      if (!isCurrent(token) || playbackErrorReported) return;
      playbackErrorReported = true;
      deps.onError?.(asError(error));
    };
    void playback.firstAudio.then(
      () => {
        if (!isCurrent(token) || releasedAt === null) return;
        latencyMs = Math.max(0, performance.now() - releasedAt);
        if (PERF_ENABLED) {
          markPerf(perfTrace, "voice.clip.first_audio_promise", {
            release_to_first_audio_ms: latencyMs,
          });
          void flushPerfTrace(perfTrace);
        }
        deps.onLatency?.(latencyMs);
      },
      (error) => {
        if (!isCurrent(token)) return;
        if (releasedAt !== null) {
          latencyMs = elapsedSinceRelease();
        }
        reportPlaybackError(error);
        if (PERF_ENABLED) {
          markPerf(perfTrace, "voice.clip.first_audio_rejected", {
            error: String(error),
          });
        }
      },
    );

    try {
      await playback.done;
    } catch (error) {
      if (isCurrent(token)) {
        if (latencyMs === null && releasedAt !== null) {
          latencyMs = elapsedSinceRelease();
        }
        reportPlaybackError(error);
      }
    }
    if (!isCurrent(token)) return;

    currentPlayback = null;
    if (PERF_ENABLED) {
      markPerf(perfTrace, "voice.clip.playback.done", {
        release_to_done_ms:
          releasedAt === null ? null : elapsedSinceRelease(),
      });
    }
    if (toolTurn) {
      const captureLatency = latencyMs ?? elapsedSinceRelease();
      await captureTurn(token, toolTurn, captureLatency);
    }
    if (PERF_ENABLED) finishVoiceTrace(perfTrace);
    if (isCurrent(token)) transition("idle");
  }

  async function processTranscript(token: number, transcript: string) {
    if (!isCurrent(token)) return;
    const perfTrace = PERF_ENABLED ? getActiveVoiceTrace() : null;
    if (PERF_ENABLED) {
      markPerf(perfTrace, "voice.stt.final", {
        transcript_chars: transcript.length,
        release_to_final_ms: elapsedSinceRelease(),
      });
    }
    deps.onTranscript?.(transcript);
    if (!isCurrent(token)) return;

    if (!transcript.trim()) {
      // Tom final = intet hørt — det er en lyd-hændelse, ikke en kommando-
      // afvisning (v4-spec §6.3): "Ingen lyd fanget — prøv igen".
      await answer(token, { kind: "stt_empty" });
      return;
    }

    transition("processing");

    let intents: VoiceIntent[] | null;
    const routerStarted = PERF_ENABLED ? performance.now() : 0;
    if (PERF_ENABLED) {
      markPerf(perfTrace, "voice.router.frontend.begin", {
        transcript_chars: transcript.length,
      });
    }
    try {
      intents = await route(transcript);
      if (PERF_ENABLED) {
        markPerf(perfTrace, "voice.router.frontend.end", {
          duration_ms: performance.now() - routerStarted,
          command_count: intents?.length ?? 0,
          ok: true,
        });
      }
    } catch (error) {
      if (!isCurrent(token)) return;
      if (PERF_ENABLED) {
        markPerf(perfTrace, "voice.router.frontend.end", {
          duration_ms: performance.now() - routerStarted,
          ok: false,
          error: String(error),
        });
      }
      deps.onError?.(asError(error));
      await answer(token, { kind: "router_reject" });
      return;
    }
    if (!isCurrent(token)) return;
    if (intents === null || intents.length === 0) {
      await answer(token, { kind: "router_reject" });
      return;
    }
    // Transcript-guarderne (guardRealtimeTurn + count-guard) er FJERNET fra
    // pipelinen (ejer-beslutning 2026-07-19, dogfood: "Luk terminal to og
    // tre" blev skudt ned fordi regex-ordbogen kraevede ordet "kort").
    // Routeren er prompt-laast mod opfundne numre (eval 30/30) og resolveren
    // er fail-closed — genindsaettes kun hvis dogfooding viser behov.

    const dryRun = deps.getDryRun?.() === true;
    const executed: Array<{
      intent: VoiceIntent;
      result: DispatchResult | DryRunResult;
    }> = [];

    for (const chained of intents) {
      if (!isCurrent(token)) return;
      // Fokus-fallback er slået FRA i kæder (multi-spec §5): en card:null-
      // send_prompt ville ellers tavst adressere det FØR ytringen fokuserede
      // kort. Én-kommando-ytringer beholder fallbacken (resolveTarget).
      if (
        intents.length >= 2 &&
        chained.kind === "send_prompt" &&
        chained.card === null
      ) {
        // Også blocked led melder deres tool-kald, så HUD'ens tool/resolver-
        // slots aldrig viser umage par (tool fra ét led, resolver fra et andet).
        deps.onToolCall?.({
          name: chained.kind,
          arguments: intentArguments(chained),
        });
        // I dry-run-mode skal det syntetiske resultat være et ÆGTE
        // DryRunResult (dry_run/action_count), så App.tsx-persist-gaten ikke
        // tavst skipper capturen når blocked står først i kæden — og så
        // as-DryRunResult-castene i outcome-konstruktionen holder. I
        // dispatch-mode forbliver det et rent DispatchResult.
        const blocked: DispatchResult | DryRunResult = dryRun
          ? {
              ok: false,
              code: "no_target",
              message: "Kommandoen mangler et kortnummer",
              dry_run: true,
              action_count: 0,
            }
          : {
              ok: false,
              code: "no_target",
              message: "Kommandoen mangler et kortnummer",
            };
        executed.push({ intent: chained, result: blocked });
        deps.onDispatchResult?.(blocked);
        continue;
      }

      const call: RealtimeToolCall = {
        name: chained.kind,
        arguments: intentArguments(chained),
      };
      deps.onToolCall?.(call);
      if (!isCurrent(token)) return;

      let result: DispatchResult | DryRunResult;
      const dispatchStarted = PERF_ENABLED ? performance.now() : 0;
      if (PERF_ENABLED) {
        markPerf(perfTrace, "voice.dispatch.command.begin", {
          command: chained.kind,
          command_index: executed.length,
        });
      }
      if (dryRun) {
        try {
          if (!deps.dryRunDispatch) {
            throw new Error("dryRunDispatch mangler i dry-run mode");
          }
          result = await deps.dryRunDispatch(chained, transcript);
        } catch (error) {
          if (!isCurrent(token)) return;
          const dispatchError = asError(error);
          deps.onError?.(dispatchError);
          // Ægte DryRunResult (dry_run/action_count) af samme grund som
          // blocked-grenen ovenfor: persist-gate + as-castene.
          result = {
            ok: false,
            code: "dry_run_exception",
            message: dispatchError.message,
            dry_run: true,
            action_count: 0,
          };
        }
      } else {
        try {
          result = await deps.dispatch(chained, transcript);
        } catch (error) {
          if (!isCurrent(token)) return;
          const dispatchError = asError(error);
          deps.onError?.(dispatchError);
          result = {
            ok: false,
            code: "dispatch_exception",
            message: dispatchError.message,
          };
        }
      }
      if (!isCurrent(token)) return;
      if (PERF_ENABLED) {
        markPerf(perfTrace, "voice.dispatch.command.end", {
          command: chained.kind,
          command_index: executed.length,
          duration_ms: performance.now() - dispatchStarted,
          ok: result.ok,
        });
      }
      executed.push({ intent: chained, result });
      deps.onDispatchResult?.(result);
    }

    if (intents.length >= 2) deps.onChainResult?.(executed);

    let outcome: TurnOutcome;
    if (intents.length === 1) {
      const single = executed[0];
      outcome = dryRun
        ? { kind: "dry_run", intent: single.intent, result: single.result as DryRunResult }
        : { kind: "dispatched", intent: single.intent, result: single.result as DispatchResult };
    } else {
      outcome = dryRun
        ? { kind: "dry_run_chain", intents, results: executed.map(({ result }) => result) as DryRunResult[] }
        : { kind: "dispatched_chain", intents, results: executed.map(({ result }) => result) as DispatchResult[] };
    }

    const first = executed[0];
    const firstCall: RealtimeToolCall = {
      name: first.intent.kind,
      arguments: intentArguments(first.intent),
    };
    await answer(token, outcome, {
      transcript,
      call: firstCall,
      resolver: first.result,
      ...(intents.length >= 2
        ? {
            commands: executed.map(({ intent: chainedIntent, result }) => ({
              tool: {
                name: chainedIntent.kind,
                arguments: intentArguments(chainedIntent),
              },
              resolver: result,
            })),
            command_count: intents.length,
          }
        : {}),
    });
  }

  async function handleSttError(token: number, error: Error) {
    if (!isCurrent(token)) return;
    deps.onError?.(error);
    await answer(token, { kind: "stt_error", message: error.message });
  }

  function startTurn() {
    const token = ++generation;
    releasedAt = null;
    currentPlayback = null;
    transition("listening");

    try {
      const sttClient = stt();
      if (deps.routeHasPartials?.() === false) {
        deps.onTurnStart?.();
      }
      let partialTranscript = "";
      sttClient.onPartial((delta) => {
        if (!isCurrent(token) || currentState !== "listening") return;
        partialTranscript += delta;
        deps.onTranscript?.(partialTranscript);
      });
      currentPtt = createPtt({
        stt: sttClient,
        startCapture,
        onState: () => undefined,
        onFinal: (transcript) => {
          if (isCurrent(token)) void processTranscript(token, transcript);
        },
        onError: (error) => {
          if (isCurrent(token)) void handleSttError(token, error);
        },
      });
      currentPtt.press();
      if (deps.maxUtteranceMs !== undefined) {
        const limitMs = deps.maxUtteranceMs;
        maxDurationTimer = setTimeout(() => {
          deps.onError?.(
            new Error(
              `Optagelsen nåede grænsen på ${Math.round(limitMs / 1000)} sekunder`,
            ),
          );
          cancelCurrent();
        }, limitMs);
      }
    } catch (error) {
      void handleSttError(token, asError(error));
    }
  }

  function cancelCurrent() {
    if (maxDurationTimer !== null) {
      clearTimeout(maxDurationTimer);
      maxDurationTimer = null;
    }
    if (PERF_ENABLED) {
      const perfTrace = getActiveVoiceTrace();
      markPerf(perfTrace, "voice.turn.cancelled");
      finishVoiceTrace(perfTrace);
    }
    generation += 1;
    currentPtt?.cancel();
    currentPtt = null;
    currentPlayback?.stop();
    currentPlayback = null;
    releasedAt = null;
    transition("idle");
  }

  return {
    press() {
      if (currentState === "listening") return;
      warm();
      if (currentState === "speaking") {
        currentPlayback?.stop();
      } else if (
        currentState === "finalizing" ||
        currentState === "processing"
      ) {
        currentPtt?.cancel();
      }
      startTurn();
    },

    release() {
      if (currentState !== "listening") return;
      if (maxDurationTimer !== null) {
        clearTimeout(maxDurationTimer);
        maxDurationTimer = null;
      }
      const perfTrace = PERF_ENABLED ? beginVoiceTrace("pipeline_release") : null;
      releasedAt = performance.now();
      if (PERF_ENABLED) {
        markPerf(perfTrace, "voice.release.pipeline", {
          state: currentState,
        });
      }
      transition("finalizing");
      currentPtt?.release();
    },

    cancel() {
      cancelCurrent();
    },

    async stop() {
      const playback = currentPlayback;
      cancelCurrent();
      if (playback) {
        try {
          await playback.done;
        } catch {
          // stop() is best-effort and always lands idle.
        }
      }
    },

    state() {
      return currentState;
    },
  };
}
