import { afterEach, describe, expect, it, vi } from "vitest";
import { deferred, flushTimers } from "../testHelpers";
import {
  REALTIME_INSTRUCTIONS,
  REALTIME_MODEL,
  buildRealtimeSessionConfig,
  createRealtimeVoiceSession,
  guardRealtimeTurn,
  realtimeToolCallToIntent,
} from "./realtime";



type Listener = (event: Event | MessageEvent) => void;

class FakeSocket {
  static readonly OPEN = 1;
  static readonly CLOSED = 3;
  readonly sent: string[] = [];
  readonly listeners = new Map<string, Listener[]>();
  readyState = 0;
  closeCalls = 0;

  constructor(
    readonly url: string,
    readonly protocols: string[],
  ) {}

  addEventListener(type: string, listener: Listener) {
    const current = this.listeners.get(type) ?? [];
    current.push(listener);
    this.listeners.set(type, current);
  }

  removeEventListener(type: string, listener: Listener) {
    this.listeners.set(
      type,
      (this.listeners.get(type) ?? []).filter((entry) => entry !== listener),
    );
  }

  send(data: string) {
    this.sent.push(data);
  }

  close() {
    this.closeCalls += 1;
    this.readyState = FakeSocket.CLOSED;
    this.emit("close", new Event("close"));
  }

  open() {
    this.readyState = FakeSocket.OPEN;
    this.emit("open", new Event("open"));
  }

  message(data: unknown) {
    this.emit("message", { data: JSON.stringify(data) } as MessageEvent);
  }

  private emit(type: string, event: Event | MessageEvent) {
    for (const listener of this.listeners.get(type) ?? []) listener(event);
  }
}

function parsedSent(socket: FakeSocket) {
  return socket.sent.map((event) => JSON.parse(event) as Record<string, unknown>);
}

async function startHarness(
  apiAudio = true,
  options: {
    dispatch?: ReturnType<typeof vi.fn>;
    getDryRun?: () => boolean;
    waitForAudioDrain?: () => Promise<void>;
    onLogicalTurnEnd?: () => void;
  } = {},
) {
  const sockets: FakeSocket[] = [];
  const dispatch =
    options.dispatch ?? vi.fn(async () => ({ ok: true, message: "udført" }));
  const errors: string[] = [];
  const states: string[] = [];
  const transcripts: string[] = [];
  const usages: unknown[] = [];
  const audio: ArrayBuffer[] = [];
  const responseTexts: string[] = [];
  const dispatchResults: unknown[] = [];
  const capturedTurns: unknown[] = [];
  const logicalTurnEnds: number[] = [];
  const session = createRealtimeVoiceSession({
    mintSecret: vi.fn(async () => ({ value: "ek-short", expires_at: 9_999_999_999 })),
    createSocket: (url, protocols) => {
      const socket = new FakeSocket(url, protocols);
      sockets.push(socket);
      return socket as unknown as WebSocket;
    },
    dispatch,
    getDryRun: options.getDryRun,
    waitForAudioDrain: options.waitForAudioDrain,
    onError: (error) => errors.push(error.message),
    onState: (state) => states.push(state),
    onTranscript: (text) => transcripts.push(text),
    onUsage: (usage) => usages.push(usage),
    onAudioChunk: (chunk) => audio.push(chunk),
    onResponseText: (text) => {
      responseTexts.push(text);
    },
    onDispatchResult: (result) => {
      dispatchResults.push(result);
    },
    onTurnComplete: (entry) => {
      capturedTurns.push(entry);
    },
    onLogicalTurnEnd: () => {
      logicalTurnEnds.push(Date.now());
      options.onLogicalTurnEnd?.();
    },
  });

  const waking = session.wake({ apiAudio });
  await flushTimers();
  const socket = sockets[0];
  socket.open();
  await flushTimers();
  socket.message({
    type: "session.updated",
    session: { type: "realtime", model: REALTIME_MODEL, expires_at: 9_999_999_999 },
  });
  await waking;
  return {
    session,
    socket,
    sockets,
    dispatch,
    errors,
    states,
    transcripts,
    usages,
    audio,
    responseTexts,
    dispatchResults,
    capturedTurns,
    logicalTurnEnds,
  };
}

function functionCallResponse(options: {
  responseId?: string;
  status?: string;
  name?: string;
  args?: unknown;
  callId?: string;
}) {
  return {
    type: "response.done",
    response: {
      id: options.responseId ?? "resp-1",
      status: options.status ?? "completed",
      usage: { total_tokens: 12 },
      output: [
        {
          type: "function_call",
          call_id: options.callId ?? "call-1",
          name: options.name ?? "restart_card",
          arguments: JSON.stringify(options.args ?? { card: 1 }),
        },
      ],
    },
  };
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("Realtime schema v3", () => {
  it("tools_v3_uden_confirm_cancel", () => {
    const config = buildRealtimeSessionConfig({ apiAudio: true });
    expect(config).toMatchObject({
      type: "realtime",
      model: "gpt-realtime-2.1-mini",
      output_modalities: ["audio"],
      tool_choice: "auto",
      audio: {
        input: {
          format: { type: "audio/pcm", rate: 24_000 },
          transcription: { model: "gpt-4o-transcribe", language: "da" },
          turn_detection: {
            type: "server_vad",
            threshold: 0.5,
            prefix_padding_ms: 300,
            silence_duration_ms: 800,
            create_response: true,
            interrupt_response: true,
          },
        },
      },
    });
    expect(config.tools.map((tool) => tool.name)).toEqual([
      "send_prompt",
      "new_card",
      "close_cards",
      "open_browser",
      "restart_card",
    ]);
    expect(config.tools.map((tool) => tool.name)).not.toContain("confirm");
    expect(config.tools.map((tool) => tool.name)).not.toContain("cancel");
  });

  it("session-config eksponerer præcis de 5 kommandoer", () => {
    const config = buildRealtimeSessionConfig({ apiAudio: true });
    expect(config.tools.map((tool) => tool.name).sort()).toEqual([
      "close_cards", "new_card", "open_browser", "restart_card", "send_prompt",
    ]);
  });

  it("create_required_kun_count", () => {
    expect(
      buildRealtimeSessionConfig({ apiAudio: true }).tools.find(
        (tool) => tool.name === "new_card",
      ),
    ).toMatchObject({
      parameters: {
        required: ["count"],
        properties: { count: { type: "integer", minimum: 1, maximum: 10 } },
      },
    });
  });

  it("close_cards_har_all_flag_v31", () => {
    expect(
      buildRealtimeSessionConfig({ apiAudio: true }).tools.find(
        (tool) => tool.name === "close_cards",
      ),
    ).toMatchObject({
      parameters: {
        required: ["cards"],
        properties: { all: { type: "boolean" } },
      },
    });
  });

  it("har few-shot for luk alle kort med all-flaget", () => {
    expect(REALTIME_INSTRUCTIONS).toContain('"Luk alle kort"');
    expect(REALTIME_INSTRUCTIONS).toContain('close_cards({"cards":[],"all":true})');
  });

  it("slår API-audio fra uden at fjerne tekstkanalen", () => {
    expect(buildRealtimeSessionConfig({ apiAudio: false }).output_modalities).toEqual([
      "text",
    ]);
  });

  it("har eksplicit dansk u12-few-shot for kort et → card 1", () => {
    expect(REALTIME_INSTRUCTIONS).toContain('"Genstart kort et"');
    expect(REALTIME_INSTRUCTIONS).toContain('restart_card({"card":1})');
  });
});

describe("guardRealtimeTurn", () => {
  it("accepterer konsistent dansk action og talord", () => {
    expect(
      guardRealtimeTurn("Genstart kort et", {
        name: "restart_card",
        arguments: { card: 1 },
      }),
    ).toEqual({ ok: true });
  });

  it("accepterer konsistente engelske card-talord", () => {
    expect(
      guardRealtimeTurn("Ask card one to run npm run build", {
        name: "send_prompt",
        arguments: { card: 1, text: "run npm run build" },
      }),
    ).toEqual({ ok: true });
    expect(
      guardRealtimeTurn("Close card five", {
        name: "close_cards",
        arguments: { cards: [5] },
      }),
    ).toEqual({ ok: true });
  });

  it("fejler lukket ved same-target action-konflikter", () => {
    expect(
      guardRealtimeTurn("Ask card one to run npm run build", {
        name: "restart_card",
        arguments: { card: 1 },
      }),
    ).toMatchObject({ ok: false, reason: "action_conflict" });
  });

  it("accepterer batchmål uden gentaget kort-markør", () => {
    expect(
      guardRealtimeTurn("Luk kort to og tre", {
        name: "close_cards",
        arguments: { cards: [2, 3] },
      }),
    ).toEqual({ ok: true });
  });

  it("blokerer tydelig action-konflikt", () => {
    expect(
      guardRealtimeTurn("Luk kort tre", {
        name: "restart_card",
        arguments: { card: 3 },
      }),
    ).toMatchObject({ ok: false, reason: "action_conflict" });
  });

  it("blokerer tydelig target-konflikt og sammenligner hele close-batchen", () => {
    expect(
      guardRealtimeTurn("Luk kort to og kort tre", {
        name: "close_cards",
        arguments: { cards: [2, 5] },
      }),
    ).toMatchObject({ ok: false, reason: "target_conflict" });
  });

  it("gætter ikke mål fra antal i 'luk 2 af dem'", () => {
    expect(
      guardRealtimeTurn("Luk 2 af dem", {
        name: "close_cards",
        arguments: { cards: [] },
      }),
    ).toEqual({ ok: true });
  });

  it("accepterer luk-alle med all-flag og tom kortliste", () => {
    expect(
      guardRealtimeTurn("Luk alle kort.", {
        name: "close_cards",
        arguments: { cards: [], all: true },
      }),
    ).toEqual({ ok: true });
    expect(
      guardRealtimeTurn("Close all cards", {
        name: "close_cards",
        arguments: { cards: [], all: true },
      }),
    ).toEqual({ ok: true });
    expect(
      guardRealtimeTurn("Lokal.", {
        name: "close_cards",
        arguments: { cards: [], all: true },
      }),
    ).toEqual({ ok: true });
  });

  it("blokerer all-flaget uden udtalt 'alle' eller med opdigtede numre", () => {
    expect(
      guardRealtimeTurn("Luk dem.", {
        name: "close_cards",
        arguments: { cards: [], all: true },
      }),
    ).toMatchObject({ ok: false, reason: "target_conflict" });
    expect(
      guardRealtimeTurn("Luk alle kort", {
        name: "close_cards",
        arguments: { cards: [2, 3], all: true },
      }),
    ).toMatchObject({ ok: false, reason: "target_conflict" });
    expect(
      guardRealtimeTurn("Luk kort to", {
        name: "close_cards",
        arguments: { cards: [], all: true },
      }),
    ).toMatchObject({ ok: false, reason: "target_conflict" });
  });

  it("konverterer all-flaget til close_cards-intent", () => {
    expect(
      realtimeToolCallToIntent({
        name: "close_cards",
        arguments: { cards: [], all: true },
      }),
    ).toEqual({ kind: "close_cards", cards: [], all: true });
    expect(
      realtimeToolCallToIntent({
        name: "close_cards",
        arguments: { cards: [2, 3] },
      }),
    ).toEqual({ kind: "close_cards", cards: [2, 3] });
  });

});

describe("persistent Realtime lifecycle", () => {
  it("minter kortlivet secret, åbner én GA-socket og sender session.update", async () => {
    const h = await startHarness();

    expect(h.socket.url).toBe(
      "wss://api.openai.com/v1/realtime?model=gpt-realtime-2.1-mini",
    );
    expect(h.socket.protocols).toEqual([
      "realtime",
      "openai-insecure-api-key.ek-short",
    ]);
    expect(parsedSent(h.socket)[0]).toEqual({
      type: "session.update",
      session: buildRealtimeSessionConfig({ apiAudio: true }),
    });
    expect(h.states).toContain("awake");
  });

  it("kan skifte en vågen session fra API-audio til tekst uden ny socket", async () => {
    const h = await startHarness();

    h.session.setApiAudio(false);

    expect(h.sockets).toHaveLength(1);
    expect(parsedSent(h.socket).at(-1)).toEqual({
      type: "session.update",
      session: buildRealtimeSessionConfig({ apiAudio: false }),
    });
  });

  it("append'er kontinuerlig PCM uden manual commit og stopper append i sleep", async () => {
    const h = await startHarness();
    h.session.appendAudio(new Uint8Array([0, 1, 2, 255]).buffer);
    expect(parsedSent(h.socket).at(-1)).toEqual({
      type: "input_audio_buffer.append",
      audio: "AAEC/w==",
    });
    expect(parsedSent(h.socket)).not.toContainEqual({ type: "input_audio_buffer.commit" });

    await h.session.sleep();
    expect(h.socket.closeCalls).toBe(1);
    expect(() => h.session.appendAudio(new Uint8Array([1]).buffer)).toThrow(/asleep/u);
  });

  it("venter på både completed response og final transcript uanset rækkefølge", async () => {
    const h = await startHarness();
    h.socket.message({ type: "input_audio_buffer.committed", item_id: "item-1" });
    h.socket.message({ type: "response.created", response: { id: "resp-1" } });
    h.socket.message(functionCallResponse({ args: { card: 1 } }));
    await flushTimers();
    expect(h.dispatch).not.toHaveBeenCalled();

    h.socket.message({
      type: "conversation.item.input_audio_transcription.completed",
      item_id: "item-1",
      transcript: "Genstart kort et",
      usage: { total_tokens: 4 },
    });
    await flushTimers();

    expect(h.dispatch).toHaveBeenCalledTimes(1);
    expect(h.dispatch).toHaveBeenCalledWith(
      { kind: "restart_card", card: 1 },
      "Genstart kort et",
    );
    const sent = parsedSent(h.socket);
    expect(sent).toContainEqual({
      type: "conversation.item.create",
      item: {
        type: "function_call_output",
        call_id: "call-1",
        output: JSON.stringify({ ok: true, message: "udført" }),
      },
    });
    expect(sent.at(-1)).toEqual({ type: "response.create" });
    expect(h.transcripts).toContain("Genstart kort et");
    expect(h.usages).toHaveLength(2);
    expect(h.capturedTurns).toEqual([
      expect.objectContaining({
        transcript: "Genstart kort et",
        tool: { name: "restart_card", arguments: { card: 1 } },
        resolver: { ok: true, message: "udført" },
        action_count: 0,
      }),
    ]);
  });

  it("korrelerer også når transcriptet ankommer før response.done", async () => {
    const h = await startHarness();
    h.socket.message({ type: "input_audio_buffer.committed", item_id: "item-2" });
    h.socket.message({ type: "response.created", response: { id: "resp-2" } });
    h.socket.message({
      type: "conversation.item.input_audio_transcription.completed",
      item_id: "item-2",
      transcript: "Genstart kort to",
    });
    h.socket.message(
      functionCallResponse({
        responseId: "resp-2",
        name: "restart_card",
        args: { card: 2 },
        callId: "call-2",
      }),
    );
    await flushTimers();

    expect(h.dispatch).toHaveBeenCalledWith(
      { kind: "restart_card", card: 2 },
      "Genstart kort to",
    );
  });

  it("udfører luk alle når dansk STT sammenfletter frasen til Lokal", async () => {
    const h = await startHarness();
    h.socket.message({ type: "input_audio_buffer.committed", item_id: "item-all" });
    h.socket.message({ type: "response.created", response: { id: "resp-all" } });
    h.socket.message({
      type: "conversation.item.input_audio_transcription.completed",
      item_id: "item-all",
      transcript: "Lokal.",
    });
    h.socket.message(
      functionCallResponse({
        responseId: "resp-all",
        name: "close_cards",
        args: { cards: [], all: true },
        callId: "call-all",
      }),
    );
    await flushTimers();

    expect(h.dispatch).toHaveBeenCalledWith(
      { kind: "close_cards", cards: [], all: true },
      "Lokal.",
    );
    expect(h.errors).toEqual([]);
  });

  it("udfører ingen handling ved cancelled response, API-fejl eller guard-konflikt", async () => {
    const h = await startHarness();

    h.socket.message({ type: "input_audio_buffer.committed", item_id: "item-cancel" });
    h.socket.message({ type: "response.created", response: { id: "resp-cancel" } });
    h.socket.message({
      type: "conversation.item.input_audio_transcription.completed",
      item_id: "item-cancel",
      transcript: "Genstart kort et",
    });
    h.socket.message(
      functionCallResponse({ responseId: "resp-cancel", status: "cancelled" }),
    );

    h.socket.message({ type: "input_audio_buffer.committed", item_id: "item-conflict" });
    h.socket.message({ type: "response.created", response: { id: "resp-conflict" } });
    h.socket.message({
      type: "conversation.item.input_audio_transcription.completed",
      item_id: "item-conflict",
      transcript: "Luk kort tre",
    });
    h.socket.message(
      functionCallResponse({
        responseId: "resp-conflict",
        name: "restart_card",
        args: { card: 3 },
      }),
    );
    h.socket.message({ type: "error", error: { message: "socket failed" } });
    await flushTimers();

    expect(h.dispatch).not.toHaveBeenCalled();
    expect(h.errors).toEqual(expect.arrayContaining(["socket failed"]));
    expect(h.errors.some((message) => message.includes("stemte ikke overens"))).toBe(true);
    expect(h.socket.closeCalls).toBe(1);
  });

  it("svarer guard-afviste og ugyldige calls med function_call_output", async () => {
    const h = await startHarness();

    h.socket.message({ type: "input_audio_buffer.committed", item_id: "item-conflict" });
    h.socket.message({ type: "response.created", response: { id: "resp-conflict" } });
    h.socket.message({
      type: "conversation.item.input_audio_transcription.completed",
      item_id: "item-conflict",
      transcript: "Luk kort tre",
    });
    h.socket.message(
      functionCallResponse({
        responseId: "resp-conflict",
        name: "restart_card",
        args: { card: 3 },
        callId: "call-conflict",
      }),
    );
    await flushTimers();

    expect(parsedSent(h.socket)).toContainEqual({
      type: "conversation.item.create",
      item: {
        type: "function_call_output",
        call_id: "call-conflict",
        output: JSON.stringify({
          ok: false,
          code: "action_conflict",
          message:
            "Det hørte og den valgte handling stemte ikke overens. Prøv kommandoen igen.",
        }),
      },
    });
    expect(h.dispatchResults).toContainEqual({
      ok: false,
      code: "action_conflict",
      message:
        "Det hørte og den valgte handling stemte ikke overens. Prøv kommandoen igen.",
    });
    expect(parsedSent(h.socket).at(-1)).toEqual({ type: "response.create" });
  });

  it("videresender API-audio, svar-transcript og response.done.usage til UI/telemetry", async () => {
    const h = await startHarness();
    h.socket.message({ type: "response.output_audio.delta", delta: "AQI=" });
    h.socket.message({
      type: "response.output_audio_transcript.done",
      transcript: "Kort tre er grønt",
    });
    h.socket.message({
      type: "response.done",
      response: {
        id: "assistant-response",
        status: "completed",
        usage: { total_tokens: 7 },
        output: [],
      },
    });
    await flushTimers();

    expect(new Uint8Array(h.audio[0])).toEqual(new Uint8Array([1, 2]));
    expect(h.responseTexts).toContain("Kort tre er grønt");
    expect(h.usages).toContainEqual({ total_tokens: 7 });
  });

  it("logisk_turn_end_efter_tool_rundtur", async () => {
    const h = await startHarness();
    h.socket.message({ type: "input_audio_buffer.committed", item_id: "item-turn" });
    h.socket.message({ type: "response.created", response: { id: "resp-tool" } });
    h.socket.message({
      type: "conversation.item.input_audio_transcription.completed",
      item_id: "item-turn",
      transcript: "Genstart kort et",
    });
    h.socket.message(functionCallResponse({ responseId: "resp-tool", args: { card: 1 } }));
    await flushTimers();
    expect(h.logicalTurnEnds).toHaveLength(0);
    expect(h.dispatch).toHaveBeenCalledTimes(1);

    h.socket.message({
      type: "response.done",
      response: {
        id: "resp-reply",
        status: "completed",
        output: [{ type: "message", content: [{ type: "output_text", text: "Genstartet" }] }],
      },
    });
    await flushTimers();
    expect(h.logicalTurnEnds).toHaveLength(1);
  });

  it("logisk_turn_end_ved_reject_vej", async () => {
    const h = await startHarness();
    h.socket.message({ type: "input_audio_buffer.committed", item_id: "item-reject" });
    h.socket.message({ type: "response.created", response: { id: "resp-reject" } });
    h.socket.message({
      type: "conversation.item.input_audio_transcription.completed",
      item_id: "item-reject",
      transcript: "Luk kort tre",
    });
    h.socket.message(
      functionCallResponse({
        responseId: "resp-reject",
        name: "restart_card",
        args: { card: 3 },
        callId: "call-reject",
      }),
    );
    await flushTimers();
    expect(h.logicalTurnEnds).toHaveLength(0);

    h.socket.message({
      type: "response.done",
      response: {
        id: "resp-reject-reply",
        status: "completed",
        output: [{ type: "message", content: [{ type: "output_text", text: "Kan du præcisere?" }] }],
      },
    });
    await flushTimers();
    expect(h.logicalTurnEnds).toHaveLength(1);
  });

  it("turn_end_venter_paa_audio", async () => {
    const drain = deferred<void>();
    const h = await startHarness(true, {
      waitForAudioDrain: () => drain.promise,
    });
    h.socket.message({
      type: "response.done",
      response: {
        id: "resp-audio",
        status: "completed",
        output: [{ type: "message", content: [{ type: "output_text", text: "Klar" }] }],
      },
    });
    await flushTimers();
    expect(h.logicalTurnEnds).toHaveLength(0);
    drain.resolve();
    await flushTimers();
    expect(h.logicalTurnEnds).toHaveLength(1);
  });
});
