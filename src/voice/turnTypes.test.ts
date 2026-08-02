import { describe, expect, it } from "vitest";
import type {
  JsonObject,
  RealtimeToolCall,
  RealtimeTurnCapture,
  RealtimeUiState,
} from "./turnTypes";

describe("turnTypes", () => {
  it("bærer tool-call-formen uændret", () => {
    const call: RealtimeToolCall = {
      name: "close_cards",
      arguments: { cards: [2, 3] } satisfies JsonObject,
      callId: "call_1",
    };
    expect(call.name).toBe("close_cards");
    expect(call.callId).toBe("call_1");
  });

  it("tillader tool-call uden callId", () => {
    const call: RealtimeToolCall = { name: "reject", arguments: {} };
    expect(call.callId).toBeUndefined();
  });

  it("dækker alle syv ui-states", () => {
    const states: RealtimeUiState[] = [
      "asleep",
      "waking",
      "awake",
      "listening",
      "processing",
      "speaking",
      "sleeping",
    ];
    expect(states).toHaveLength(7);
  });

  it("bærer turn-capture-formen uændret", () => {
    const capture: RealtimeTurnCapture = {
      ts: "2026-07-28T12:00:00.000Z",
      transcript: "Luk kort to og tre.",
      tool: { name: "close_cards", arguments: { cards: [2, 3] } },
      resolver: null,
      latency_ms: 940,
      action_count: 0,
    };
    expect(capture.action_count).toBe(0);
    expect(capture.command_count).toBeUndefined();
  });
});
