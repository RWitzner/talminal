import { invoke } from "@tauri-apps/api/core";
import { base64ToBuffer, bytesToBase64 } from "../base64";
import { asError } from "./errors";
import type { VoiceIntent } from "./intents";
import type {
  JsonObject,
  RealtimeToolCall,
  RealtimeTurnCapture,
  RealtimeUiState,
} from "./turnTypes";

// Re-eksport: realtime.test.ts og voice-eval importerer stadig herfra.
export type {
  JsonObject,
  RealtimeToolCall,
  RealtimeTurnCapture,
  RealtimeUiState,
};

export const REALTIME_MODEL = "gpt-realtime-2.1-mini";
const REALTIME_URL = `wss://api.openai.com/v1/realtime?model=${REALTIME_MODEL}`;
const MAX_RECENT_INPUT_ITEMS = 12;

export const REALTIME_INSTRUCTIONS = `You are the deterministic voice-command executor for Talminal, a Windows canvas of numbered Claude Code cards.

Map each settled actionable utterance to exactly one matching function tool. Do not emit conversational filler while calling a tool. Use no tool when there is no settled action, actions conflict, or multiple spoken targets remain possible. Never invent a card number. The deterministic resolver owns missing-target and focus errors: clear singular commands without a spoken card number MUST call their tool with card:null. A clear work request without a card MUST call send_prompt with card:null. Explicit numbered destructive targets execute immediately — never ask for confirmation. Target-less destructive requests ("luk to af dem") must not invent targets: call close_cards with an empty cards list so the resolver can ask a clarifying question. "Luk alle kort" / "close all cards" means every open card: call close_cards with all:true and an empty cards list — the resolver expands "all" from live state; never enumerate numbers you did not hear. Danish and English may be mixed. Convert Danish number words et/en=1, to=2, tre=3, fire=4, fem=5 only when they explicitly identify a card. "Åbn en browser" (optionally naming github or google) MUST call open_browser; never invent URLs. Status questions, focus/switch commands and creates that name a project or folder have no tool — call no tool for them.

Examples:
"Genstart kort et" -> restart_card({"card":1})
"Luk kort to og kort tre" -> close_cards({"cards":[2,3]})
"Luk 2 af dem" -> close_cards({"cards":[]})
"Luk alle kort" -> close_cards({"cards":[],"all":true})
"Åbn 4 terminaler" -> new_card({"count":4})
"Åbn en browser på GitHub" -> open_browser({"url_hint":"github"})`;

export type RealtimeGuardResult =
  | { ok: true }
  | { ok: false; reason: "action_conflict" | "target_conflict" };

function objectParameters(properties: JsonObject = {}, required: string[] = []) {
  return {
    type: "object",
    properties,
    required,
    additionalProperties: false,
  };
}

function cardParameter() {
  return {
    anyOf: [{ type: "integer", minimum: 1 }, { type: "null" }],
    description: "Explicitly spoken card number, otherwise null.",
  };
}

function cardTool(name: string, description: string) {
  return {
    type: "function",
    name,
    description,
    parameters: objectParameters({ card: cardParameter() }, ["card"]),
  };
}

export function buildRealtimeSessionConfig({ apiAudio }: { apiAudio: boolean }) {
  return {
    type: "realtime",
    model: REALTIME_MODEL,
    instructions: REALTIME_INSTRUCTIONS,
    output_modalities: [apiAudio ? "audio" : "text"],
    audio: {
      input: {
        format: { type: "audio/pcm", rate: 24_000 },
        // Samme model og samme dialekt som pipeline-vejen (`providers.rs`'
        // STT_ROUTES). Modellen staar hardkodet her og ikke via ruten, fordi
        // denne motor er slukket: `voice_engine` normaliseres altid til
        // "pipeline", saa App.tsx naar aldrig herind. Den opdateres alligevel,
        // saa `gpt-4o-transcribe` ikke bliver staaende ét sted i traeet og
        // saar tvivl om hvad der gaelder — `voice-eval/realtime-config.mjs`
        // importerer stadig herfra.
        //
        // `languages` (array) og ikke `language`: gpt-transcribe tager
        // flertalsformen, og den forkerte form giver 200 med et tavst ignoreret
        // hint. Se `LanguageField` i providers.rs.
        transcription: { model: "gpt-transcribe", languages: ["da"] },
        turn_detection: {
          type: "server_vad",
          threshold: 0.5,
          prefix_padding_ms: 300,
          silence_duration_ms: 800,
          create_response: true,
          interrupt_response: true,
        },
      },
      output: {
        format: { type: "audio/pcm", rate: 24_000 },
        voice: "marin",
      },
    },
    tools: [
      {
        type: "function",
        name: "send_prompt",
        description: "Send requested work text to one card.",
        parameters: objectParameters(
          {
            card: cardParameter(),
            text: {
              type: "string",
              minLength: 1,
              description: "Requested work with routing words removed.",
            },
          },
          ["card", "text"],
        ),
      },
      {
        type: "function",
        name: "new_card",
        description: "Create one or more cards in the canvas project.",
        parameters: objectParameters(
          { count: { type: "integer", minimum: 1, maximum: 10 } },
          ["count"],
        ),
      },
      {
        type: "function",
        name: "close_cards",
        description:
          "Close explicitly numbered cards as one batch immediately, or every open card when the utterance says all.",
        parameters: objectParameters(
          {
            cards: {
              type: "array",
              items: { type: "integer", minimum: 1 },
              description: "Explicit card numbers only; empty when none were spoken.",
            },
            all: {
              type: "boolean",
              description:
                'True only when the utterance says all cards ("alle"/"all"); cards must stay empty.',
            },
          },
          ["cards"],
        ),
      },
      {
        type: "function",
        name: "open_browser",
        description: "Open a browser card for previews or web research.",
        parameters: objectParameters(
          {
            url_hint: {
              type: "string",
              enum: ["github", "google"],
              description: "Only when a known site was spoken; omit otherwise.",
            },
          },
          [],
        ),
      },
      cardTool("restart_card", "Restart one explicitly named card immediately."),
    ],
    tool_choice: "auto",
  };
}

const NUMBER_WORDS: Record<string, number> = {
  et: 1,
  en: 1,
  one: 1,
  to: 2,
  two: 2,
  tre: 3,
  three: 3,
  fire: 4,
  four: 4,
  fem: 5,
  five: 5,
};

function normalized(text: string): string {
  return text.normalize("NFKC").toLocaleLowerCase("da-DK").trim();
}

function transcriptSaysAll(text: string): boolean {
  const value = normalized(text);
  return (
    /\b(alle|all)\b/u.test(value) ||
    // Dansk "luk alle" bliver lejlighedsvis slået sammen til "Lokal." af
    // transskriptionen, selv om Realtime-modellen hører og router korrekt.
    /^(lokal|lukal)[.!?]*$/u.test(value)
  );
}

function transcriptAction(text: string): string | null {
  const value = normalized(text);
  if (/^(fortæl|bed|send|ask|sig til|tell)\b/u.test(value)) return "send_prompt";
  if (/^(opret|lav)\b.*\bkort/u.test(value)) return "new_card";
  if (/^(luk|close)\b/u.test(value)) return "close_cards";
  if (/^(genstart|restart)\b/u.test(value)) return "restart_card";
  return null;
}

const NUMBER_TOKEN = "et|en|one|to|two|tre|three|fire|four|fem|five|\\d+";

function transcriptCards(text: string): number[] {
  const cards: number[] = [];
  const pattern = new RegExp(
    `\\b(?:kort|cards?)\\s+(${NUMBER_TOKEN})((?:\\s*(?:og|and|,)\\s*(?:${NUMBER_TOKEN})\\b)*)`,
    "giu",
  );
  for (const match of normalized(text).matchAll(pattern)) {
    const tokens = [
      match[1],
      ...[...(match[2] ?? "").matchAll(new RegExp(`\\b(${NUMBER_TOKEN})\\b`, "giu"))].map(
        (tail) => tail[1],
      ),
    ];
    for (const token of tokens) {
      const card = NUMBER_WORDS[token] ?? Number(token);
      if (Number.isInteger(card) && card > 0 && !cards.includes(card)) cards.push(card);
    }
  }
  return cards;
}

function toolCards(call: RealtimeToolCall): number[] | null {
  if (call.name === "close_cards") {
    const cards = call.arguments.cards;
    return Array.isArray(cards)
      ? cards.filter((card): card is number => Number.isInteger(card) && Number(card) > 0)
      : [];
  }
  if (["send_prompt", "restart_card"].includes(call.name)) {
    const card = call.arguments.card;
    return Number.isInteger(card) && Number(card) > 0 ? [Number(card)] : [];
  }
  return null;
}

export function guardRealtimeTurn(
  transcript: string,
  call: RealtimeToolCall,
): RealtimeGuardResult {
  const action = transcriptAction(transcript);
  if (action !== null && action !== call.name) {
    return { ok: false, reason: "action_conflict" };
  }
  const expectedCards = transcriptCards(transcript);
  const actualCards = toolCards(call);
  if (call.name === "close_cards" && call.arguments.all === true) {
    // all:true er kun gyldigt når "alle"/"all" faktisk blev udtalt, og hverken
    // transkript eller args baerer numre — ellers er flaget uverificerbart.
    // transcriptSaysAll medtager den observerede fonetiske STT-sammenfletning.
    const saysAll = transcriptSaysAll(transcript);
    return saysAll && expectedCards.length === 0 && (actualCards?.length ?? 0) === 0
      ? { ok: true }
      : { ok: false, reason: "target_conflict" };
  }
  if (
    actualCards !== null &&
    (actualCards.length !== expectedCards.length ||
      actualCards.some((card, index) => card !== expectedCards[index]))
  ) {
    return { ok: false, reason: "target_conflict" };
  }
  return { ok: true };
}

export function realtimeToolCallToIntent(call: RealtimeToolCall): VoiceIntent | null {
  const card = Number.isInteger(call.arguments.card) && Number(call.arguments.card) > 0
    ? Number(call.arguments.card)
    : null;
  switch (call.name) {
    case "send_prompt":
      return typeof call.arguments.text === "string" && call.arguments.text.trim()
        ? { kind: call.name, card, text: call.arguments.text.trim() }
        : null;
    case "new_card":
      if (!Number.isInteger(call.arguments.count)) return null;
      return { kind: call.name, count: Number(call.arguments.count) };
    case "close_cards":
      return {
        kind: call.name,
        cards: Array.isArray(call.arguments.cards)
          ? call.arguments.cards.filter(
              (candidate): candidate is number =>
                Number.isInteger(candidate) && Number(candidate) > 0,
            )
          : [],
        ...(call.arguments.all === true ? { all: true } : {}),
      };
    case "restart_card":
      return { kind: call.name, card };
    case "open_browser":
      return {
        kind: call.name,
        url_hint:
          call.arguments.url_hint === "github" || call.arguments.url_hint === "google"
            ? call.arguments.url_hint
            : null,
      };
    default:
      return null;
  }
}

interface PendingTurn {
  itemId: string;
  committedAt: number;
  responseId?: string;
  transcript?: string;
  response?: JsonObject;
  failed: boolean;
  processing: boolean;
  latencyMs?: number;
}

export interface RealtimeVoiceSession {
  wake(options: { apiAudio: boolean }): Promise<void>;
  sleep(): Promise<void>;
  setApiAudio(enabled: boolean): void;
  appendAudio(chunk: ArrayBuffer): void;
  state(): RealtimeUiState;
}

export function createRealtimeVoiceSession(dependencies: {
  mintSecret?: () => Promise<{ value: string; expires_at: number }>;
  createSocket?: (url: string, protocols: string[]) => WebSocket;
  dispatch(intent: VoiceIntent, rawTranscript?: string): Promise<unknown>;
  getDryRun?: () => boolean;
  waitForAudioDrain?: () => Promise<void>;
  onError?(error: Error): void;
  onState?(state: RealtimeUiState): void;
  onTranscript?(text: string): void;
  onUsage?(usage: unknown): void;
  onAudioChunk?(chunk: ArrayBuffer): void;
  onResponseText?(text: string): void | Promise<void>;
  onToolCall?(call: RealtimeToolCall): void;
  onDispatchResult?(result: unknown): void;
  onLatency?(milliseconds: number): void;
  onTurnComplete?(entry: RealtimeTurnCapture): void | Promise<void>;
  onLogicalTurnEnd?(): void;
}): RealtimeVoiceSession {
  const mintSecret =
    dependencies.mintSecret ??
    (() => invoke<{ value: string; expires_at: number }>("mint_realtime_secret"));
  const createSocket =
    dependencies.createSocket ?? ((url, protocols) => new WebSocket(url, protocols));
  let uiState: RealtimeUiState = "asleep";
  let active = false;
  let socket: WebSocket | null = null;
  let connectionGeneration = 0;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let wakeOptions = { apiAudio: true };
  const turnsByItem = new Map<string, PendingTurn>();
  const turnsByResponse = new Map<string, PendingTurn>();
  const unassignedItems: string[] = [];
  const recentInputItems: string[] = [];

  function transition(next: RealtimeUiState) {
    uiState = next;
    dependencies.onState?.(next);
  }

  function send(event: JsonObject) {
    if (!socket || socket.readyState !== 1) throw new Error("Realtime socket is not open");
    socket.send(JSON.stringify(event));
  }

  function clearReconnectTimer() {
    if (reconnectTimer) clearTimeout(reconnectTimer);
    reconnectTimer = null;
  }

  function clearTurns() {
    turnsByItem.clear();
    turnsByResponse.clear();
    unassignedItems.splice(0);
  }

  function cleanupTurn(turn: PendingTurn) {
    turnsByItem.delete(turn.itemId);
    if (turn.responseId) turnsByResponse.delete(turn.responseId);
  }

  function reportError(error: unknown) {
    dependencies.onError?.(asError(error));
  }

  function responseText(response: JsonObject): string {
    const output = Array.isArray(response.output) ? response.output : [];
    return output
      .flatMap((item) =>
        item && typeof item === "object" && Array.isArray((item as JsonObject).content)
          ? ((item as JsonObject).content as unknown[])
          : [],
      )
      .filter((content) => content && typeof content === "object")
      .map((content) => content as JsonObject)
      .filter((content) => content.type === "output_text" || content.type === "output_audio")
      .map((content) =>
        typeof content.text === "string"
          ? content.text
          : typeof content.transcript === "string"
            ? content.transcript
            : "",
      )
      .join("");
  }

  function responseCall(response: JsonObject): RealtimeToolCall | null {
    const output = Array.isArray(response.output) ? response.output : [];
    const calls = output.filter(
      (item): item is JsonObject =>
        Boolean(item) && typeof item === "object" && (item as JsonObject).type === "function_call",
    );
    if (calls.length !== 1) return null;
    const item = calls[0];
    if (typeof item.name !== "string" || typeof item.call_id !== "string") return null;
    try {
      const parsed = JSON.parse(typeof item.arguments === "string" ? item.arguments : "{}");
      if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return null;
      return { name: item.name, arguments: parsed as JsonObject, callId: item.call_id };
    } catch {
      return null;
    }
  }

  async function finishLogicalTurn() {
    try {
      await dependencies.waitForAudioDrain?.();
    } catch (error) {
      reportError(error);
    }
    dependencies.onLogicalTurnEnd?.();
    if (active) transition("listening");
  }

  async function processTurn(turn: PendingTurn) {
    if (
      turn.processing ||
      turn.failed ||
      turn.transcript === undefined ||
      turn.response === undefined
    ) {
      return;
    }
    turn.processing = true;
    const call = responseCall(turn.response);
    if (!call) {
      cleanupTurn(turn);
      await finishLogicalTurn();
      return;
    }
    dependencies.onToolCall?.(call);

    function rejectCall(code: string, message: string) {
      if (active && socket?.readyState === 1) {
        send({
          type: "conversation.item.create",
          item: {
            type: "function_call_output",
            call_id: call!.callId,
            output: JSON.stringify({ ok: false, code, message }),
          },
        });
        send({ type: "response.create" });
      }
    }

    const guard = guardRealtimeTurn(turn.transcript, call);
    if (!guard.ok) {
      const message =
        guard.reason === "action_conflict"
          ? "Det hørte og den valgte handling stemte ikke overens. Prøv kommandoen igen."
          : "Det hørte og de valgte kort stemte ikke overens. Prøv kommandoen igen.";
      dependencies.onDispatchResult?.({
        ok: false,
        code: guard.reason,
        message,
      });
      reportError(new Error(message));
      rejectCall(guard.reason, message);
      cleanupTurn(turn);
      return;
    }
    const intent = realtimeToolCallToIntent(call);
    if (!intent) {
      const message = "Voice-værktøjet havde ugyldige argumenter. Prøv kommandoen igen.";
      dependencies.onDispatchResult?.({
        ok: false,
        code: "invalid_arguments",
        message,
      });
      reportError(new Error(message));
      rejectCall("invalid_arguments", message);
      cleanupTurn(turn);
      return;
    }

    transition("processing");
    let result: unknown;
    if (dependencies.getDryRun?.()) {
      result = { ok: true, dry_run: true };
      dependencies.onDispatchResult?.(result);
    } else {
      try {
        result = await dependencies.dispatch(intent, turn.transcript);
        dependencies.onDispatchResult?.(result);
      } catch (error) {
        const dispatchError = asError(error);
        reportError(dispatchError);
        result = { ok: false, message: dispatchError.message };
        dependencies.onDispatchResult?.(result);
      }
    }

    if (dependencies.onTurnComplete) {
      try {
        await dependencies.onTurnComplete({
          ts: new Date().toISOString(),
          transcript: turn.transcript,
          tool: { name: call.name, arguments: call.arguments },
          resolver: result,
          latency_ms: turn.latencyMs ?? Date.now() - turn.committedAt,
          action_count: 0,
        });
      } catch (error) {
        reportError(new Error(`Voice capture fejlede: ${asError(error).message}`));
      }
    }

    if (active && socket?.readyState === 1) {
      send({
        type: "conversation.item.create",
        item: {
          type: "function_call_output",
          call_id: call.callId,
          output: JSON.stringify(result),
        },
      });
      send({ type: "response.create" });
    }
    cleanupTurn(turn);
    if (active) transition("listening");
  }

  function scheduleReconnect(expiresAt: unknown) {
    clearReconnectTimer();
    if (typeof expiresAt !== "number") return;
    const delay = expiresAt * 1_000 - Date.now() - 30_000;
    if (delay <= 0 || delay > 3_600_000) return;
    reconnectTimer = setTimeout(() => {
      if (!active) return;
      const current = socket;
      socket = null;
      current?.close();
      transition("waking");
      void establish().catch((error) => {
        reportError(error);
        active = false;
        transition("asleep");
      });
    }, delay);
  }

  function handleMessage(message: JsonObject) {
    switch (message.type) {
      case "session.updated": {
        const sessionInfo = message.session as JsonObject | undefined;
        scheduleReconnect(sessionInfo?.expires_at);
        transition("awake");
        transition("listening");
        return;
      }
      case "input_audio_buffer.speech_started":
        transition("listening");
        return;
      case "input_audio_buffer.committed": {
        if (typeof message.item_id !== "string") return;
        const turn: PendingTurn = {
          itemId: message.item_id,
          committedAt: Date.now(),
          failed: false,
          processing: false,
        };
        turnsByItem.set(turn.itemId, turn);
        unassignedItems.push(turn.itemId);
        recentInputItems.push(turn.itemId);
        if (recentInputItems.length > MAX_RECENT_INPUT_ITEMS && socket?.readyState === 1) {
          const oldest = recentInputItems.shift();
          if (oldest) send({ type: "conversation.item.delete", item_id: oldest });
        }
        transition("processing");
        return;
      }
      case "response.created": {
        const response = message.response as JsonObject | undefined;
        if (typeof response?.id !== "string") return;
        const itemId = unassignedItems.shift();
        if (!itemId) return;
        const turn = turnsByItem.get(itemId);
        if (!turn) return;
        turn.responseId = response.id;
        turnsByResponse.set(response.id, turn);
        return;
      }
      case "conversation.item.input_audio_transcription.completed": {
        if (typeof message.item_id !== "string" || typeof message.transcript !== "string") return;
        const turn = turnsByItem.get(message.item_id);
        if (!turn || turn.failed) return;
        turn.transcript = message.transcript.trim();
        dependencies.onTranscript?.(turn.transcript);
        if (message.usage !== undefined) dependencies.onUsage?.(message.usage);
        void processTurn(turn);
        return;
      }
      case "conversation.item.input_audio_transcription.failed": {
        if (typeof message.item_id === "string") {
          const turn = turnsByItem.get(message.item_id);
          if (turn) turn.failed = true;
        }
        reportError(new Error("Realtime transcription fejlede"));
        return;
      }
      case "response.output_audio.delta":
        if (typeof message.delta === "string") {
          transition("speaking");
          dependencies.onAudioChunk?.(base64ToBuffer(message.delta));
        }
        return;
      case "response.output_audio_transcript.done":
      case "response.output_text.done":
        if (typeof message.transcript === "string") {
          void dependencies.onResponseText?.(message.transcript);
        } else if (typeof message.text === "string") {
          void dependencies.onResponseText?.(message.text);
        }
        return;
      case "response.done": {
        const response = message.response as JsonObject | undefined;
        if (!response || typeof response.id !== "string") return;
        if (response.usage !== undefined) dependencies.onUsage?.(response.usage);
        const text = responseText(response);
        if (text) void dependencies.onResponseText?.(text);
        const turn = turnsByResponse.get(response.id);
        if (!turn) {
          // Follow-up after tool/reject: call-free response ends the logical turn.
          if (response.status === "completed" && !responseCall(response)) {
            void finishLogicalTurn();
          } else if (active) {
            transition("listening");
          }
          return;
        }
        turn.latencyMs = Date.now() - turn.committedAt;
        dependencies.onLatency?.(turn.latencyMs);
        if (response.status !== "completed") {
          turn.failed = true;
          cleanupTurn(turn);
          return;
        }
        turn.response = response;
        void processTurn(turn);
        return;
      }
      case "error": {
        for (const turn of turnsByItem.values()) turn.failed = true;
        const payload = message.error as JsonObject | undefined;
        reportError(new Error(typeof payload?.message === "string" ? payload.message : "Realtime API-fejl"));
        void session.sleep();
        return;
      }
    }
  }

  async function establish(): Promise<void> {
    const generation = ++connectionGeneration;
    const secret = await mintSecret();
    if (!active || generation !== connectionGeneration) return;
    const nextSocket = createSocket(REALTIME_URL, [
      "realtime",
      `openai-insecure-api-key.${secret.value}`,
    ]);
    socket = nextSocket;

    await new Promise<void>((resolve, reject) => {
      let configured = false;
      const fail = (error: unknown) => {
        if (!configured) reject(asError(error));
        reportError(error);
      };
      nextSocket.addEventListener("open", () => {
        if (!active || socket !== nextSocket) return;
        nextSocket.send(
          JSON.stringify({
            type: "session.update",
            session: buildRealtimeSessionConfig(wakeOptions),
          }),
        );
      });
      nextSocket.addEventListener("message", (event) => {
        if (!active || socket !== nextSocket) return;
        try {
          const message = JSON.parse(String(event.data)) as JsonObject;
          handleMessage(message);
          if (message.type === "session.updated" && !configured) {
            configured = true;
            resolve();
          }
        } catch (error) {
          fail(error);
        }
      });
      nextSocket.addEventListener("error", () => fail(new Error("Realtime socket-fejl")));
      nextSocket.addEventListener("close", () => {
        if (socket !== nextSocket) return;
        socket = null;
        if (active) {
          active = false;
          clearReconnectTimer();
          clearTurns();
          transition("asleep");
          if (!configured) reject(new Error("Realtime socket lukkede før sessionen var klar"));
          else reportError(new Error("Realtime socket lukkede uventet"));
        }
      });
    });
  }

  const session: RealtimeVoiceSession = {
    async wake(options) {
      if (active) return;
      active = true;
      wakeOptions = options;
      transition("waking");
      try {
        await establish();
      } catch (error) {
        active = false;
        const current = socket;
        socket = null;
        current?.close();
        clearTurns();
        transition("asleep");
        throw error;
      }
    },
    async sleep() {
      if (!active && uiState === "asleep") return;
      transition("sleeping");
      active = false;
      connectionGeneration += 1;
      clearReconnectTimer();
      clearTurns();
      const current = socket;
      socket = null;
      current?.close();
      transition("asleep");
    },
    setApiAudio(enabled) {
      wakeOptions = { apiAudio: enabled };
      if (active && socket?.readyState === 1) {
        send({
          type: "session.update",
          session: buildRealtimeSessionConfig(wakeOptions),
        });
      }
    },
    appendAudio(chunk) {
      if (!active) throw new Error("Realtime session is asleep");
      if (!socket || socket.readyState !== 1) return;
      send({ type: "input_audio_buffer.append", audio: bytesToBase64(chunk) });
    },
    state() {
      return uiState;
    },
  };

  return session;
}
