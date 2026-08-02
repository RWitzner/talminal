import { createDryRunDispatch } from "../src/voice/dryRun.ts";
import {
  buildRealtimeSessionConfig,
  guardRealtimeTurn,
  realtimeToolCallToIntent,
} from "../src/voice/realtime.ts";

export const REALTIME_EVAL_MODEL = "gpt-realtime-2.1-mini";

/** Schema/instructions spejler appens v3-config (T5); output forbliver text-mode til eval. */
export function buildRealtimeEvalConfig() {
  const session = buildRealtimeSessionConfig({ apiAudio: false });
  return {
    ...session,
    model: REALTIME_EVAL_MODEL,
    output_modalities: ["text"],
  };
}

export function normalizeRealtimeIntent(call) {
  if (!call || typeof call !== "object" || typeof call.name !== "string") return null;
  const args =
    call.arguments && typeof call.arguments === "object" && !Array.isArray(call.arguments)
      ? call.arguments
      : {};
  if (call.name === "close_cards") {
    const cards = Array.isArray(args.cards)
      ? args.cards.filter((card) => Number.isInteger(card) && card > 0)
      : [];
    return {
      kind: "close_cards",
      cards,
      ...(args.all === true ? { all: true } : {}),
    };
  }
  return { kind: call.name, ...args };
}

export async function evaluateRealtimeDryRunTurn({
  transcript,
  call,
  focusedCard,
  latencyMs,
  ts = new Date().toISOString(),
}) {
  const tool = {
    name: call?.name ?? "",
    arguments:
      call?.arguments && typeof call.arguments === "object" && !Array.isArray(call.arguments)
        ? call.arguments
        : {},
  };
  const hasToolCall = typeof call?.name === "string" && call.name.length > 0;
  let resolver;
  if (!hasToolCall) {
    resolver = {
      ok: false,
      code: "no_tool_call",
      message: "Dry-run modtog intet tool-kald",
      dry_run: true,
      action_count: 0,
    };
  } else {
    const guard = guardRealtimeTurn(transcript, tool);
    if (!guard.ok) {
      resolver = {
        ok: false,
        code: guard.reason,
        message: `Dry-run guard blokerede: ${guard.reason}`,
        dry_run: true,
        action_count: 0,
      };
    } else {
      const intent = realtimeToolCallToIntent(tool);
      if (!intent) {
        resolver = {
          ok: false,
          code: "invalid_tool_call",
          message: "Dry-run kunne ikke konvertere tool-kaldet",
          dry_run: true,
          action_count: 0,
        };
      } else {
        const dispatch = createDryRunDispatch({
          getCards: () => [1, 2, 3, 5].map((number) => ({ number })),
          getFocusedCard: () => focusedCard,
          getProject: async () => ({
            root: "C:\\projekter\\demo",
            name: "demo",
          }),
        });
        resolver = await dispatch(intent, transcript);
      }
    }
  }
  return {
    ts,
    transcript,
    tool,
    resolver,
    latency_ms: latencyMs,
    action_count: 0,
  };
}

const FOCUSED_CARD_BY_CONTEXT = { A: 2, B: null, C: 2 };

export async function evaluateRealtimeResult({ row, result, ts }) {
  const sourceToolCalls = Array.isArray(result.toolCalls) ? result.toolCalls : [];
  const call = sourceToolCalls.length === 1 ? sourceToolCalls[0] : null;
  const capture = await evaluateRealtimeDryRunTurn({
    transcript: row.utterance,
    call,
    focusedCard: FOCUSED_CARD_BY_CONTEXT[row.context] ?? null,
    latencyMs: result.latencyMs,
    ts,
  });
  if (sourceToolCalls.length > 1) {
    capture.resolver = {
      ok: false,
      code: "tool_call_count",
      message: `Dry-run forventede højst ét tool-kald, modtog ${sourceToolCalls.length}`,
      dry_run: true,
      action_count: 0,
    };
  }

  const toolCalls = sourceToolCalls.map((sourceCall) => {
    const normalized = normalizeRealtimeIntent(sourceCall);
    if (!normalized) return sourceCall;
    const { kind, ...arguments_ } = normalized;
    return {
      ...sourceCall,
      name: kind,
      arguments: arguments_,
      rawArguments: JSON.stringify(arguments_),
    };
  });

  return {
    result: { ...result, toolCalls },
    capture,
  };
}

export function passesRealtimeGate({
  intentAccuracy,
  targetAccuracy,
  ambiguousRejected,
  ambiguousTotal,
}) {
  // Realtime-evalen kører KUN single-subsettet (multi-spec §10.2): 17 gyldige
  // + 17 tvetydige efter kæde-filteret — kæde-cases er pipeline-only.
  return (
    intentAccuracy >= 0.9 &&
    targetAccuracy >= 0.95 &&
    ambiguousTotal === 17 &&
    ambiguousRejected === ambiguousTotal
  );
}
