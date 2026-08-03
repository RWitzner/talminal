import { afterEach, describe, expect, it, vi } from "vitest";
import type { DispatchResult } from "./dispatch";
import type { DryRunResult } from "./dryRun";
import type { VoiceIntent } from "./intents";
import { createPipelineVoiceSession, type PipelineUiState } from "./pipeline";
import type { Reply } from "./replies";
import type { SttClient } from "./stt";
import type { TtsPlayback } from "./tts";
import { deferred, flushTimers } from "../testHelpers";



function fakeStt(final: Promise<string> | string = "Genstart kort tre"): SttClient {
  return {
    start: vi.fn(async () => undefined),
    pushAudio: vi.fn(),
    onPartial: vi.fn(),
    stop: vi.fn(async () => final),
    abort: vi.fn(),
  };
}

function resolvedPlayback(): TtsPlayback {
  return {
    firstAudio: Promise.resolve(),
    done: Promise.resolve(),
    stop: vi.fn(),
  };
}

const restartIntent: VoiceIntent = { kind: "restart_card", card: 3 };
const dispatchResult: DispatchResult = {
  ok: true,
  kind: "restart_card",
  card: 3,
  message: "Kort 3 blev genstartet",
};

afterEach(() => {
  vi.restoreAllMocks();
});

function makeHarness(options: {
  stts?: SttClient[];
  route?: (text: string) => Promise<VoiceIntent[] | null>;
  dispatch?: (intent: VoiceIntent, text?: string) => Promise<DispatchResult>;
  dryRunDispatch?: (
    intent: VoiceIntent,
    text?: string,
  ) => Promise<DryRunResult>;
  dryRun?: boolean;
  speak?: (reply: Reply) => TtsPlayback;
} = {}) {
  const stts = options.stts ?? [fakeStt()];
  let sttIndex = 0;
  const states: PipelineUiState[] = [];
  const transcripts: string[] = [];
  const toolCalls: Array<{
    name: string;
    arguments: Record<string, unknown>;
  }> = [];
  const results: Array<DispatchResult | DryRunResult> = [];
  const responses: string[] = [];
  const errors: Error[] = [];
  const latencies: number[] = [];
  const captures: unknown[] = [];
  const route = vi.fn(
    options.route ?? (async () => [restartIntent]),
  );
  const dispatch = vi.fn(
    options.dispatch ?? (async () => dispatchResult),
  );
  const dryRunDispatch = vi.fn(options.dryRunDispatch);
  const speak = vi.fn(options.speak ?? resolvedPlayback);
  const warm = vi.fn();
  const startCapture = vi.fn(async () => ({
    stop: vi.fn(async () => undefined),
  }));
  const session = createPipelineVoiceSession({
    stt: () => stts[Math.min(sttIndex++, stts.length - 1)],
    route,
    dispatch,
    ...(options.dryRunDispatch ? { dryRunDispatch } : {}),
    getDryRun: () => options.dryRun ?? false,
    speak,
    startCapture,
    onState: (state) => states.push(state),
    onTranscript: (text) => transcripts.push(text),
    onToolCall: (call) => toolCalls.push(call),
    onDispatchResult: (result) => results.push(result),
    onResponseText: (text) => responses.push(text),
    onError: (error) => errors.push(error),
    onLatency: (ms) => latencies.push(ms),
    onTurnComplete: (entry) => {
      captures.push(entry);
    },
    warm,
  });
  return {
    session,
    stts,
    states,
    transcripts,
    toolCalls,
    results,
    responses,
    errors,
    latencies,
    captures,
    route,
    dispatch,
    dryRunDispatch,
    speak,
    startCapture,
    warm,
  };
}

async function runTurn(h: ReturnType<typeof makeHarness>) {
  h.session.press();
  await flushTimers();
  h.session.release();
  await flushTimers();
  await flushTimers();
}

// Kæde-test-hjælpere (Task 3): spejler filens stub-mønster — route stubbes
// til at returnere det angivne array, speak samler audioKeys.
function fakeSttFinal(final: string): () => SttClient {
  return () => fakeStt(final);
}

function collectSpeak(spoken: string[]): (reply: Reply) => TtsPlayback {
  return (reply) => {
    spoken.push(reply.audioKey ?? reply.text);
    return resolvedPlayback();
  };
}

const stubStartCapture = async () => ({ stop: async () => undefined });

function minimalDeps() {
  return {
    dispatch: vi.fn(
      async (intent: VoiceIntent): Promise<DispatchResult> => ({
        ok: true,
        kind: intent.kind,
        message: "ok",
      }),
    ),
    speak: resolvedPlayback,
    startCapture: stubStartCapture,
  };
}

function sessionWithRoute(
  intents: VoiceIntent[],
  overrides: {
    dispatch?: (intent: VoiceIntent, text?: string) => Promise<DispatchResult>;
    speak?: (reply: Reply) => TtsPlayback;
    onChainResult?(
      commands: Array<{
        intent: VoiceIntent;
        result: DispatchResult | DryRunResult;
      }>,
    ): void;
    onTurnComplete?(entry: unknown): void;
  } = {},
) {
  return createPipelineVoiceSession({
    stt: fakeSttFinal("Genstart kort tre"),
    route: async () => intents,
    dispatch:
      overrides.dispatch ??
      (async (intent: VoiceIntent) => ({
        ok: true,
        kind: intent.kind,
        message: "ok",
      })),
    speak: overrides.speak ?? resolvedPlayback,
    startCapture: stubStartCapture,
    ...(overrides.onChainResult
      ? { onChainResult: overrides.onChainResult }
      : {}),
    ...(overrides.onTurnComplete
      ? { onTurnComplete: overrides.onTurnComplete }
      : {}),
  });
}

describe("createPipelineVoiceSession", () => {
  it("rydder HUD'en ved turens start når ruten ikke har partials", async () => {
    const onTurnStart = vi.fn();
    const session = createPipelineVoiceSession({
      ...minimalDeps(),
      stt: () => fakeStt(),
      routeHasPartials: () => false,
      onTurnStart,
    });
    session.press();
    await flushTimers();
    expect(onTurnStart).toHaveBeenCalledTimes(1);
  });

  it("rydder IKKE ved turens start når ruten har partials", async () => {
    const onTurnStart = vi.fn();
    const session = createPipelineVoiceSession({
      ...minimalDeps(),
      stt: () => fakeStt(),
      routeHasPartials: () => true,
      onTurnStart,
    });
    session.press();
    await flushTimers();
    expect(onTurnStart).not.toHaveBeenCalled();
  });

  it("læser partial-støtten ved HVER tur", async () => {
    let hasPartials = true;
    const onTurnStart = vi.fn();
    const session = createPipelineVoiceSession({
      ...minimalDeps(),
      stt: () => fakeStt(),
      routeHasPartials: () => hasPartials,
      onTurnStart,
    });
    session.press();
    await flushTimers();
    session.cancel();
    hasPartials = false;
    session.press();
    await flushTimers();
    expect(onTurnStart).toHaveBeenCalledTimes(1);
  });

  it("afbryder optagelsen ved max-varighed", async () => {
    vi.useFakeTimers();
    const stt = fakeStt();
    const session = createPipelineVoiceSession({
      ...minimalDeps(),
      stt: () => stt,
      routeHasPartials: () => false,
      maxUtteranceMs: 1_000,
    });
    session.press();
    await vi.advanceTimersByTimeAsync(1_200);
    expect(session.state()).toBe("idle");
    expect(stt.abort).toHaveBeenCalled();
    vi.useRealTimers();
  });

  it("full_turn_dispatches_and_speaks_template", async () => {
    const h = makeHarness();
    await runTurn(h);

    expect(h.route).toHaveBeenCalledWith("Genstart kort tre");
    expect(h.dispatch).toHaveBeenCalledWith(
      restartIntent,
      "Genstart kort tre",
    );
    expect(h.responses).toEqual(["Kort genstartet"]);
    expect(h.speak).toHaveBeenCalledWith({
      text: "Kort genstartet",
      audioKey: "kort-genstartet",
    });
    expect(h.session.state()).toBe("idle");
  });

  it("router_reject_speaks_reject_template_without_dispatch", async () => {
    const h = makeHarness({ route: async () => null });
    await runTurn(h);

    expect(h.dispatch).not.toHaveBeenCalled();
    expect(h.responses).toEqual(["Det fangede jeg ikke"]);
    expect(h.captures).toEqual([]);
  });

  it("router_transport_error_speaks_reject", async () => {
    const h = makeHarness({
      route: async () => {
        throw new Error("router transport nede");
      },
    });
    await runTurn(h);

    expect(h.errors.map((error) => error.message)).toContain(
      "router transport nede",
    );
    expect(h.responses).toEqual(["Det fangede jeg ikke"]);
    expect(h.dispatch).not.toHaveBeenCalled();
    expect(h.captures).toEqual([]);
  });

  // Transcript-guarderne er fjernet fra pipelinen (ejer-beslutning
  // 2026-07-19): regex-ordbogen false-positivede paa synonymer ("Luk
  // terminal to og tre" kraevede ordet "kort"). Denne test laaser at
  // router-output nu dispatches uden transcript-sammenligning — routeren
  // er prompt-laast mod opfundne numre og resolveren er fail-closed.
  it("router_intent_dispatches_without_transcript_guard", async () => {
    const h = makeHarness({
      stts: [fakeStt("Luk terminal to og tre")],
      route: async () => [{ kind: "close_cards", cards: [2, 3] }],
      dispatch: async () => ({
        ok: true,
        kind: "close_cards",
        message: "Kort 2, 3 blev lukket",
      }),
    });
    await runTurn(h);

    expect(h.dispatch).toHaveBeenCalledWith(
      { kind: "close_cards", cards: [2, 3] },
      "Luk terminal to og tre",
    );
  });

  it("count_guard_allows_spoken_count", async () => {
    const intent: VoiceIntent = {
      kind: "new_card",
      count: 4,
    };
    const result: DispatchResult = {
      ok: true,
      kind: "new_card",
      message: "4 kort oprettet",
    };
    const h = makeHarness({
      stts: [fakeStt("Åbn fire terminaler")],
      route: async () => [intent],
      dispatch: async () => result,
    });
    await runTurn(h);

    expect(h.dispatch).toHaveBeenCalledWith(intent, "Åbn fire terminaler");
  });

  it("open_browser_intent_sender_url_hint_som_toolcall_arguments_og_taler_statisk_svar", async () => {
    const intent: VoiceIntent = { kind: "open_browser", url_hint: "github" };
    const result: DispatchResult = {
      ok: true,
      kind: "open_browser",
      card: 6,
      message: "Browser åbnet som kort 6",
    };
    const h = makeHarness({
      stts: [fakeStt("Åbn en browser på GitHub")],
      route: async () => [intent],
      dispatch: async () => result,
    });
    await runTurn(h);

    expect(h.toolCalls).toEqual([
      { name: "open_browser", arguments: { url_hint: "github" } },
    ]);
    expect(h.dispatch).toHaveBeenCalledWith(intent, "Åbn en browser på GitHub");
    expect(h.responses).toEqual(["Browser åbnet"]);
    expect(h.speak).toHaveBeenCalledWith({
      text: "Browser åbnet",
      audioKey: "browser-aabnet",
    });
  });

  it("dry_run_uses_dry_run_dispatch_and_captures_full_result", async () => {
    const result: DryRunResult = {
      ok: true,
      kind: "restart_card",
      card: 3,
      message: "Dry-run: restart_card",
      dry_run: true,
      action_count: 0,
    };
    const h = makeHarness({
      dryRun: true,
      dryRunDispatch: async () => result,
    });
    await runTurn(h);

    expect(h.dispatch).not.toHaveBeenCalled();
    expect(h.dryRunDispatch).toHaveBeenCalledWith(
      restartIntent,
      "Genstart kort tre",
    );
    expect(h.captures).toHaveLength(1);
    expect((h.captures[0] as { resolver: unknown }).resolver).toBe(result);
  });

  it("dispatch_exception_wraps_result_and_captures", async () => {
    const h = makeHarness({
      dispatch: async () => {
        throw new Error("dispatch eksploderede");
      },
    });
    await runTurn(h);

    const wrapped = {
      ok: false,
      code: "dispatch_exception",
      message: "dispatch eksploderede",
    };
    expect(h.errors.map((error) => error.message)).toContain(
      "dispatch eksploderede",
    );
    expect(h.results).toEqual([wrapped]);
    expect(h.responses).toEqual(["Noget gik galt — se skærmen"]);
    expect(h.captures).toHaveLength(1);
    expect((h.captures[0] as { resolver: unknown }).resolver).toEqual(wrapped);
  });

  it("dry_run_exception_wraps_result_and_captures", async () => {
    const h = makeHarness({
      dryRun: true,
      dryRunDispatch: async () => {
        throw new Error("dry-run eksploderede");
      },
    });
    await runTurn(h);

    // Wrapperen skal være et ÆGTE DryRunResult (dry_run/action_count), så
    // App.tsx-persist-gaten ikke tavst skipper capturen når exceptionen
    // står først i en kæde.
    const wrapped = {
      ok: false,
      code: "dry_run_exception",
      message: "dry-run eksploderede",
      dry_run: true,
      action_count: 0,
    };
    expect(h.errors.map((error) => error.message)).toContain(
      "dry-run eksploderede",
    );
    expect(h.results).toEqual([wrapped]);
    // Kæde-loopet (Task 3) ruter ALLE dry-run-ture til dry_run-outcomet —
    // også exceptions: dry-run taler ikke (v4-spec §5), HUD-teksten er
    // exception-beskeden. Før talte denne kant det generiske fejl-klip.
    expect(h.responses).toEqual(["dry-run eksploderede"]);
    expect(h.captures).toHaveLength(1);
    expect((h.captures[0] as { resolver: unknown }).resolver).toEqual(wrapped);
  });

  it("barge_in_stops_playback_and_starts_new_turn", async () => {
    const done = deferred<void>();
    const playback: TtsPlayback = {
      firstAudio: Promise.resolve(),
      done: done.promise,
      stop: vi.fn(() => done.resolve()),
    };
    const h = makeHarness({
      stts: [fakeStt(), fakeStt("Genstart kort tre")],
      speak: () => playback,
    });
    h.session.press();
    await flushTimers();
    h.session.release();
    await flushTimers();
    expect(h.session.state()).toBe("speaking");

    h.session.press();
    expect(playback.stop).toHaveBeenCalledTimes(1);
    expect(h.session.state()).toBe("listening");
    expect(h.stts[1].start).toHaveBeenCalledTimes(1);
  });

  it("press_during_processing_cancels_pending_turn", async () => {
    const routed = deferred<VoiceIntent[] | null>();
    const h = makeHarness({
      stts: [fakeStt(), fakeStt()],
      route: () => routed.promise,
    });
    h.session.press();
    await flushTimers();
    h.session.release();
    await flushTimers();
    expect(h.session.state()).toBe("processing");

    h.session.press();
    routed.resolve([restartIntent]);
    await flushTimers();
    expect(h.session.state()).toBe("listening");
    expect(h.dispatch).not.toHaveBeenCalled();
  });

  it("press_while_listening_is_noop", async () => {
    const h = makeHarness();
    h.session.press();
    h.session.press();
    await flushTimers();

    expect(h.stts[0].start).toHaveBeenCalledTimes(1);
    expect(h.startCapture).toHaveBeenCalledTimes(1);
    expect(h.states).toEqual(["listening"]);
  });

  it("press_warms_voice_connections_once_per_turn_start", async () => {
    const h = makeHarness();
    h.session.press();
    h.session.press();
    await flushTimers();

    // Kun det tur-startende tryk varmer; listening-no-op'et gør ikke.
    expect(h.warm).toHaveBeenCalledTimes(1);
  });

  it("release_outside_listening_is_noop", async () => {
    const h = makeHarness();
    h.session.release();
    h.session.press();
    await flushTimers();
    h.session.release();
    h.session.release();
    await flushTimers();

    expect(h.stts[0].stop).toHaveBeenCalledTimes(1);
    expect(h.states.filter((state) => state === "finalizing")).toHaveLength(1);
  });

  it("stt_partials_forwarded_accumulated_to_onTranscript", async () => {
    let emitPartial!: (delta: string) => void;
    const stt = fakeStt("Genstart kort tre");
    stt.onPartial = vi.fn((callback) => {
      emitPartial = callback;
    });
    const h = makeHarness({ stts: [stt] });

    h.session.press();
    emitPartial("Genstart ");
    emitPartial("kort ");
    emitPartial("tre");
    expect(h.transcripts).toEqual([
      "Genstart ",
      "Genstart kort ",
      "Genstart kort tre",
    ]);

    h.session.release();
    await flushTimers();
    expect(h.transcripts.at(-1)).toBe("Genstart kort tre");
  });

  it("partials_from_stale_generation_dropped", () => {
    let emitFirstPartial!: (delta: string) => void;
    const first = fakeStt();
    first.onPartial = vi.fn((callback) => {
      emitFirstPartial = callback;
    });
    const h = makeHarness({ stts: [first, fakeStt()] });

    h.session.press();
    h.session.cancel();
    h.session.press();
    emitFirstPartial("for gammel");

    expect(h.transcripts).toEqual([]);
    expect(h.session.state()).toBe("listening");
  });

  it("late_stt_final_after_cancel_is_dropped", async () => {
    const final = deferred<string>();
    const h = makeHarness({ stts: [fakeStt(final.promise)] });
    h.session.press();
    await flushTimers();
    h.session.release();
    h.session.cancel();
    final.resolve("Genstart kort tre");
    await flushTimers();

    expect(h.transcripts).toEqual([]);
    expect(h.route).not.toHaveBeenCalled();
    expect(h.session.state()).toBe("idle");
  });

  it("late_router_result_after_new_press_is_dropped", async () => {
    const routed = deferred<VoiceIntent[] | null>();
    const h = makeHarness({
      stts: [fakeStt(), fakeStt()],
      route: () => routed.promise,
    });
    h.session.press();
    await flushTimers();
    h.session.release();
    await flushTimers();
    h.session.press();
    routed.resolve([restartIntent]);
    await flushTimers();

    expect(h.dispatch).not.toHaveBeenCalled();
    expect(h.session.state()).toBe("listening");
  });

  it("late_dispatch_result_after_stop_is_dropped", async () => {
    const dispatched = deferred<DispatchResult>();
    const h = makeHarness({ dispatch: () => dispatched.promise });
    h.session.press();
    await flushTimers();
    h.session.release();
    await flushTimers();
    await h.session.stop();
    dispatched.resolve(dispatchResult);
    await flushTimers();

    expect(h.results).toEqual([]);
    expect(h.responses).toEqual([]);
    expect(h.captures).toEqual([]);
    expect(h.session.state()).toBe("idle");
  });

  it("late_tts_completion_after_barge_in_is_dropped", async () => {
    const done = deferred<void>();
    const playback: TtsPlayback = {
      firstAudio: Promise.resolve(),
      done: done.promise,
      stop: vi.fn(),
    };
    const h = makeHarness({
      stts: [fakeStt(), fakeStt()],
      speak: () => playback,
    });
    h.session.press();
    await flushTimers();
    h.session.release();
    await flushTimers();
    h.session.press();
    done.resolve();
    await flushTimers();

    expect(h.captures).toEqual([]);
    expect(h.session.state()).toBe("listening");
  });

  it("stop_during_each_phase_lands_idle", async () => {
    for (const phase of [
      "listening",
      "finalizing",
      "processing",
      "speaking",
    ] as const) {
      const final = deferred<string>();
      const routed = deferred<VoiceIntent[] | null>();
      const done = deferred<void>();
      const playback: TtsPlayback = {
        firstAudio: Promise.resolve(),
        done: done.promise,
        stop: vi.fn(() => done.resolve()),
      };
      const h = makeHarness({
        stts: [fakeStt(phase === "finalizing" ? final.promise : "Genstart kort tre")],
        route: phase === "processing" ? () => routed.promise : async () => [restartIntent],
        speak: () => playback,
      });
      h.session.press();
      await flushTimers();
      if (phase !== "listening") {
        h.session.release();
        await flushTimers();
      }
      if (phase === "speaking") await flushTimers();

      await h.session.stop();
      expect(h.session.state(), phase).toBe("idle");
      final.resolve("Genstart kort tre");
      routed.resolve([restartIntent]);
      done.resolve();
    }
  });

  it("stt_error_speaks_error_template", async () => {
    const stt = fakeStt();
    stt.start = vi.fn(async () => {
      throw new Error("ingen nøgle");
    });
    const h = makeHarness({ stts: [stt] });
    h.session.press();
    await flushTimers();
    await flushTimers();

    expect(h.responses).toEqual(["Noget gik galt — se skærmen"]);
    expect(h.errors.map((error) => error.message)).toContain("ingen nøgle");
    expect(h.captures).toEqual([]);
  });

  it("tts_failure_still_completes_turn_with_hud_text", async () => {
    const failed = Promise.reject(new Error("tts nede"));
    void failed.catch(() => undefined);
    const h = makeHarness({
      speak: () => ({
        firstAudio: failed,
        done: failed,
        stop: vi.fn(),
      }),
    });
    await runTurn(h);

    expect(h.responses).toEqual(["Kort genstartet"]);
    expect(h.errors.map((error) => error.message)).toContain("tts nede");
    expect(h.session.state()).toBe("idle");
  });

  it("tts_failure_captures_elapsed_latency_not_zero", async () => {
    const firstAudio = deferred<void>();
    const done = deferred<void>();
    let now = 2_000;
    vi.spyOn(performance, "now").mockImplementation(() => now);
    const h = makeHarness({
      speak: () => ({
        firstAudio: firstAudio.promise,
        done: done.promise,
        stop: vi.fn(),
      }),
    });

    h.session.press();
    await flushTimers();
    h.session.release();
    await flushTimers();
    now = 2_460;
    firstAudio.reject(new Error("tts fejlede før lyd"));
    done.reject(new Error("tts fejlede før lyd"));
    await flushTimers();

    expect(h.latencies).toEqual([]);
    expect(h.captures).toHaveLength(1);
    expect((h.captures[0] as { latency_ms: number }).latency_ms).toBe(460);
  });

  it("capture_entry_carries_full_resolver_and_origin", async () => {
    const h = makeHarness();
    await runTurn(h);

    expect(h.captures).toHaveLength(1);
    expect(h.captures[0]).toMatchObject({
      transcript: "Genstart kort tre",
      tool: {
        name: "restart_card",
        arguments: { card: 3 },
      },
      resolver: dispatchResult,
      action_count: 0,
      origin: "voice",
    });
    expect((h.captures[0] as { ts: string }).ts).not.toBe("");
  });

  it("latency_measured_release_to_first_audio", async () => {
    const firstAudio = deferred<void>();
    const done = deferred<void>();
    let now = 1_000;
    vi.spyOn(performance, "now").mockImplementation(() => now);
    const h = makeHarness({
      speak: () => ({
        firstAudio: firstAudio.promise,
        done: done.promise,
        stop: vi.fn(() => done.resolve()),
      }),
    });
    h.session.press();
    await flushTimers();
    h.session.release();
    await flushTimers();
    now = 1_275;
    firstAudio.resolve();
    done.resolve();
    await flushTimers();

    expect(h.latencies).toEqual([275]);
    expect(h.captures).toHaveLength(1);
    expect((h.captures[0] as { latency_ms: number }).latency_ms).toBe(275);
  });

  it("tom STT-final giver stt_empty-klippet uden router-kald", async () => {
    const route = vi.fn();
    const spoken: string[] = [];
    const session = createPipelineVoiceSession({
      stt: fakeSttFinal("   "),
      route,
      dispatch: vi.fn(),
      speak: collectSpeak(spoken),
      startCapture: stubStartCapture,
    });
    session.press();
    session.release();
    await vi.waitFor(() =>
      expect(spoken).toEqual(["ingen-lyd-fanget-proev-igen"]),
    );
    expect(route).not.toHaveBeenCalled();
  });

  it("en 2-kæde udføres sekventielt og taler Udført", async () => {
    const order: string[] = [];
    const dispatch = vi.fn(
      async (intent: VoiceIntent): Promise<DispatchResult> => {
        order.push(intent.kind);
        return { ok: true, kind: intent.kind, message: "ok" };
      },
    );
    const spoken: string[] = [];
    const session = sessionWithRoute(
      [{ kind: "new_card", count: 1 }, { kind: "open_browser", url_hint: null }],
      { dispatch, speak: collectSpeak(spoken) },
    );
    session.press();
    session.release();
    await vi.waitFor(() => expect(spoken).toEqual(["udfoert"]));
    expect(order).toEqual(["new_card", "open_browser"]);
  });

  it("fortsæt-ved-fejl: fejlet led stopper ikke resten, og kæden taler fejl-klippet", async () => {
    const dispatch = vi
      .fn<(intent: VoiceIntent, text?: string) => Promise<DispatchResult>>()
      .mockResolvedValueOnce({
        ok: false,
        code: "no_cards",
        message: "Ingen åbne kort at lukke",
      })
      .mockResolvedValueOnce({ ok: true, kind: "new_card", message: "ok" });
    const spoken: string[] = [];
    const session = sessionWithRoute(
      [{ kind: "close_cards", cards: [], all: true }, { kind: "new_card", count: 3 }],
      { dispatch, speak: collectSpeak(spoken) },
    );
    session.press();
    session.release();
    await vi.waitFor(() => expect(spoken).toEqual(["noget-gik-galt-se-skaermen"]));
    expect(dispatch).toHaveBeenCalledTimes(2);
  });

  it("fokus-fallback er slået fra i kæder: card:null-send_prompt dispatches ikke", async () => {
    const dispatch = vi.fn(
      async (intent: VoiceIntent): Promise<DispatchResult> => ({
        ok: true,
        kind: intent.kind,
        message: "ok",
      }),
    );
    const chainResults: unknown[] = [];
    const session = sessionWithRoute(
      [
        { kind: "new_card", count: 1 },
        { kind: "send_prompt", card: null, text: "kør testene" },
      ],
      { dispatch, onChainResult: (commands) => void chainResults.push(commands) },
    );
    session.press();
    session.release();
    await vi.waitFor(() => expect(chainResults).toHaveLength(1));
    expect(dispatch).toHaveBeenCalledTimes(1); // kun new_card
    expect(
      (chainResults[0] as Array<{ result: { ok: boolean; code?: string } }>)[1]
        .result,
    ).toMatchObject({
      ok: false,
      code: "no_target",
    });
  });

  it("kæde-capture bærer commands/command_count og første kommando i tool/resolver", async () => {
    const captures: unknown[] = [];
    const session = sessionWithRoute(
      [{ kind: "new_card", count: 1 }, { kind: "open_browser", url_hint: null }],
      { onTurnComplete: (entry) => void captures.push(entry) },
    );
    session.press();
    session.release();
    await vi.waitFor(() => expect(captures).toHaveLength(1));
    expect(captures[0]).toMatchObject({
      action_count: 0, // FROSSEN invariant
      command_count: 2,
      tool: { name: "new_card" },
    });
    expect((captures[0] as { commands: unknown[] }).commands).toHaveLength(2);
  });

  it("blocked kæde-led udløser stadig onToolCall, så HUD'ens tool/resolver-par matcher", async () => {
    const h = makeHarness({
      route: async () => [
        { kind: "new_card", count: 1 },
        { kind: "send_prompt", card: null, text: "kør testene" },
      ],
    });
    await runTurn(h);

    expect(h.toolCalls).toEqual([
      { name: "new_card", arguments: { count: 1 } },
      { name: "send_prompt", arguments: { card: null, text: "kør testene" } },
    ]);
  });

  it("onToolCall-telemetrien medsender agent naar intentet har et eksplicit agent-valg (T7)", async () => {
    const h = makeHarness({
      route: async () => [{ kind: "new_card", count: 1, agent: "codex" }],
    });
    await runTurn(h);

    expect(h.toolCalls).toEqual([
      { name: "new_card", arguments: { count: 1, agent: "codex" } },
    ]);
  });

  it("onToolCall-telemetrien har ingen agent-noegle uden et eksplicit agent-valg — ikke bare en undefined-vaerdi (T7)", async () => {
    const h = makeHarness({
      route: async () => [{ kind: "new_card", count: 1 }],
    });
    await runTurn(h);

    expect(h.toolCalls).toHaveLength(1);
    // toEqual/toMatchObject ignorerer undefined-vaerdi-noegler (ville bestaa
    // uanset om agent er udeladt ELLER spredt ubetinget som undefined) —
    // toHaveProperty skelner reelt mellem fravaer og en tilstedevaerende
    // undefined-vaerdi, saa denne test fanger en regression til ubetinget spread.
    expect(h.toolCalls[0].arguments).not.toHaveProperty("agent");
  });

  it("dry-run-kæde: blocked første led capturer ægte DryRunResult (persist-gate-felterne)", async () => {
    const h = makeHarness({
      dryRun: true,
      route: async () => [
        { kind: "send_prompt", card: null, text: "kør testene" },
        { kind: "new_card", count: 1 },
      ],
      dryRunDispatch: async (intent) => ({
        ok: true,
        kind: intent.kind,
        message: "Dry-run: ok",
        dry_run: true,
        action_count: 0,
      }),
    });
    await runTurn(h);

    expect(h.dryRunDispatch).toHaveBeenCalledTimes(1); // kun new_card
    expect(h.captures).toHaveLength(1);
    expect((h.captures[0] as { resolver: unknown }).resolver).toEqual({
      ok: false,
      code: "no_target",
      message: "Kommandoen mangler et kortnummer",
      dry_run: true,
      action_count: 0,
    });
  });

  it("én-kommando-ytring capturer BYTE-kompatibelt (ingen commands/command_count)", async () => {
    const captures: Array<Record<string, unknown>> = [];
    const session = sessionWithRoute(
      [{ kind: "restart_card", card: 2 }],
      { onTurnComplete: (entry) => void captures.push(entry as never) },
    );
    session.press();
    session.release();
    await vi.waitFor(() => expect(captures).toHaveLength(1));
    expect("commands" in captures[0]).toBe(false);
    expect("command_count" in captures[0]).toBe(false);
  });
});
