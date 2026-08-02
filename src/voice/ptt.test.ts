/**
 * @vitest-environment happy-dom
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import fixtures from "../../hotkey-grammar.fixtures.json";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import {
  createPtt,
  createSessionToggle,
  createStreamingPcm16Resampler,
  isKeyboardBinding,
  matchesAccelerator,
  parseAccelerator,
  registerPttKey,
  registerWakeKey,
  resampleToPcm16,
  type AudioCapture,
  type PttState,
  type SessionToggleState,
} from "./ptt";
import { type SttClient } from "./stt";

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

async function flush() {
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
}

function makeStt(overrides: Partial<SttClient> = {}): SttClient {
  return {
    start: vi.fn(async () => undefined),
    pushAudio: vi.fn(),
    onPartial: vi.fn(),
    stop: vi.fn(async () => "luk kort tre"),
    abort: vi.fn(),
    ...overrides,
  };
}

function makeCapture(): AudioCapture & { stop: ReturnType<typeof vi.fn> } {
  return {
    stop: vi.fn(async () => undefined),
  };
}

function makeHarness(stt = makeStt(), capture = makeCapture()) {
  const states: PttState[] = [];
  const finals: string[] = [];
  const errors: Error[] = [];
  const startCapture = vi.fn(async () => capture);
  const ptt = createPtt({
    stt,
    onState: (state) => states.push(state),
    onFinal: (text) => finals.push(text),
    onError: (error) => errors.push(error),
    startCapture,
  });
  return { ptt, stt, capture, startCapture, states, finals, errors };
}

describe("createPtt", () => {
  it("går press → listening → release → finalizing → idle og leverer final tekst", async () => {
    const h = makeHarness();

    h.ptt.press();
    expect(h.states).toEqual(["listening"]);
    await flush();
    expect(h.startCapture).toHaveBeenCalledTimes(1);

    h.ptt.release();
    expect(h.states).toEqual(["listening", "finalizing"]);
    await flush();

    expect(h.capture.stop).toHaveBeenCalledTimes(1);
    expect(h.stt.stop).toHaveBeenCalledTimes(1);
    expect(h.stt.abort).not.toHaveBeenCalled();
    expect(h.finals).toEqual(["luk kort tre"]);
    expect(h.states).toEqual(["listening", "finalizing", "idle"]);
  });

  it("returnerer til idle uden final tekst når STT start fejler", async () => {
    const partial = vi.fn();
    const stt = makeStt({
      start: vi.fn(async () => {
        throw new Error("start failed");
      }),
      onPartial: vi.fn((cb) => cb("ufuldstændig tekst")),
    });
    stt.onPartial(partial);
    const h = makeHarness(stt);

    h.ptt.press();
    await flush();

    expect(h.startCapture).toHaveBeenCalledTimes(1);
    expect(h.capture.stop).toHaveBeenCalledTimes(1);
    expect(h.finals).toEqual([]);
    expect(h.errors.map((error) => error.message)).toEqual(["start failed"]);
    expect(h.states).toEqual(["listening", "idle"]);
  });

  it("returnerer til idle uden partial som final når STT stop fejler", async () => {
    const stt = makeStt({
      onPartial: vi.fn((cb) => cb("må ikke blive final")),
      stop: vi.fn(async () => {
        throw new Error("stop failed");
      }),
    });
    const h = makeHarness(stt);

    h.ptt.press();
    await flush();
    h.ptt.release();
    await flush();

    expect(h.finals).toEqual([]);
    expect(h.errors.map((error) => error.message)).toEqual(["stop failed"]);
    expect(h.states).toEqual(["listening", "finalizing", "idle"]);
  });

  it("ignorerer overlap-press mens der lyttes og press under finalizing", async () => {
    const stop = deferred<string>();
    const stt = makeStt({ stop: vi.fn(() => stop.promise) });
    const h = makeHarness(stt);

    h.ptt.press();
    h.ptt.press();
    await flush();
    expect(stt.start).toHaveBeenCalledTimes(1);

    h.ptt.release();
    h.ptt.press();
    expect(h.states).toEqual(["listening", "finalizing"]);
    expect(stt.start).toHaveBeenCalledTimes(1);

    stop.resolve("færdig");
    await flush();
    expect(h.finals).toEqual(["færdig"]);
    expect(h.states.at(-1)).toBe("idle");
  });

  it("starter capture straks og buffer lyd mens async STT-start afventer", async () => {
    const start = deferred<void>();
    const stt = makeStt({ start: vi.fn(() => start.promise) });
    let emit!: (chunk: ArrayBuffer) => void;
    const states: PttState[] = [];
    const ptt = createPtt({
      stt,
      onState: (state) => states.push(state),
      onFinal: vi.fn(),
      startCapture: vi.fn(async (onChunk) => {
        emit = onChunk;
        return makeCapture();
      }),
    });

    ptt.press();
    await flush();
    emit(new Uint8Array([1, 2]).buffer);
    expect(stt.pushAudio).not.toHaveBeenCalled();

    start.resolve();
    await flush();
    expect(stt.pushAudio).toHaveBeenCalledWith(new Uint8Array([1, 2]).buffer);
    expect(states).toEqual(["listening"]);
  });

  it("håndterer release mens async start stadig afventer og rydder tidlig capture", async () => {
    const start = deferred<void>();
    const stt = makeStt({ start: vi.fn(() => start.promise) });
    const h = makeHarness(stt);

    h.ptt.press();
    h.ptt.release();
    expect(h.states).toEqual(["listening", "finalizing"]);

    start.resolve();
    await flush();

    expect(h.startCapture).toHaveBeenCalledTimes(1);
    expect(h.capture.stop).toHaveBeenCalledTimes(1);
    expect(stt.stop).toHaveBeenCalledTimes(1);
    expect(h.finals).toEqual(["luk kort tre"]);
    expect(h.states.at(-1)).toBe("idle");
  });

  it("rydder capture og returnerer idle når browser-capture fejler", async () => {
    const stt = makeStt();
    const states: PttState[] = [];
    const finals: string[] = [];
    const errors: Error[] = [];
    const ptt = createPtt({
      stt,
      onState: (state) => states.push(state),
      onFinal: (text) => finals.push(text),
      onError: (error) => errors.push(error),
      startCapture: vi.fn(async () => {
        throw new Error("permission denied");
      }),
    });

    ptt.press();
    await flush();

    expect(stt.stop).toHaveBeenCalledTimes(1);
    expect(finals).toEqual([]);
    expect(errors.map((error) => error.message)).toEqual(["permission denied"]);
    expect(states).toEqual(["listening", "idle"]);
  });

  it("rydder capture og returnerer idle hvis audio-push afslører disconnect", async () => {
    const stt = makeStt({
      pushAudio: vi.fn(() => {
        throw new Error("socket closed");
      }),
    });
    const capture = makeCapture();
    let emit!: (chunk: ArrayBuffer) => void;
    const states: PttState[] = [];
    const finals: string[] = [];
    const errors: Error[] = [];
    const ptt = createPtt({
      stt,
      onState: (state) => states.push(state),
      onFinal: (text) => finals.push(text),
      onError: (error) => errors.push(error),
      startCapture: vi.fn(async (onChunk) => {
        emit = onChunk;
        return capture;
      }),
    });

    ptt.press();
    await flush();
    emit(new Uint8Array([1]).buffer);
    await flush();

    expect(capture.stop).toHaveBeenCalledTimes(1);
    expect(stt.stop).toHaveBeenCalledTimes(1);
    expect(finals).toEqual([]);
    expect(errors.map((error) => error.message)).toEqual(["socket closed"]);
    expect(states).toEqual(["listening", "idle"]);
  });

  it("rapporterer capture-cleanup fejl én gang og undertrykker final", async () => {
    const capture = makeCapture();
    capture.stop.mockRejectedValue(new Error("capture cleanup failed"));
    const h = makeHarness(makeStt(), capture);

    h.ptt.press();
    await flush();
    h.ptt.release();
    await flush();

    expect(h.stt.stop).toHaveBeenCalledTimes(1);
    expect(h.stt.abort).not.toHaveBeenCalled();
    expect(h.finals).toEqual([]);
    expect(h.errors.map((error) => error.message)).toEqual([
      "capture cleanup failed",
    ]);
    expect(h.states.at(-1)).toBe("idle");
  });

  it("bevarer primær fejl når cleanup også fejler", async () => {
    const capture = makeCapture();
    capture.stop.mockRejectedValue(new Error("secondary cleanup"));
    const stt = makeStt({
      start: vi.fn(async () => {
        throw "primary start failure";
      }),
    });
    const h = makeHarness(stt, capture);

    h.ptt.press();
    await flush();

    expect(h.finals).toEqual([]);
    expect(h.errors).toHaveLength(1);
    expect(h.errors[0]).toBeInstanceOf(Error);
    expect(h.errors[0].message).toBe("primary start failure");
    expect(h.states.at(-1)).toBe("idle");
  });

  it("cancel_from_listening_suppresses_final", async () => {
    const h = makeHarness();

    h.ptt.press();
    await flush();
    h.ptt.cancel();
    await flush();

    expect(h.capture.stop).toHaveBeenCalledTimes(1);
    expect(h.stt.abort).toHaveBeenCalledTimes(1);
    expect(h.stt.stop).not.toHaveBeenCalled();
    expect(h.finals).toEqual([]);
    expect(h.states.at(-1)).toBe("idle");
  });

  it("cancel_from_finalizing_suppresses_late_final", async () => {
    const stop = deferred<string>();
    const h = makeHarness(makeStt({ stop: vi.fn(() => stop.promise) }));

    h.ptt.press();
    await flush();
    h.ptt.release();
    h.ptt.cancel();
    expect(h.states.at(-1)).toBe("idle");

    stop.resolve("for sent");
    await flush();
    expect(h.finals).toEqual([]);
    expect(h.states.at(-1)).toBe("idle");
  });

  it("cancel_is_idempotent", async () => {
    const h = makeHarness();

    h.ptt.press();
    await flush();
    h.ptt.cancel();
    h.ptt.cancel();
    await flush();

    expect(h.capture.stop).toHaveBeenCalledTimes(1);
    expect(h.stt.abort).toHaveBeenCalledTimes(1);
    expect(h.stt.stop).not.toHaveBeenCalled();
    expect(h.states.filter((state) => state === "idle")).toHaveLength(1);
  });

  it("press_after_cancel_starts_fresh_cycle", async () => {
    const h = makeHarness();

    h.ptt.press();
    await flush();
    h.ptt.cancel();
    await flush();
    h.ptt.press();
    await flush();
    h.ptt.release();
    await flush();

    expect(h.stt.start).toHaveBeenCalledTimes(2);
    expect(h.finals).toEqual(["luk kort tre"]);
    expect(h.states.at(-1)).toBe("idle");
  });
});

describe("resampleToPcm16", () => {
  it("nedprøver 48 kHz mono float til 24 kHz PCM16", () => {
    const pcm = resampleToPcm16(new Float32Array([0, 0.5, 1, -0.5]), 48_000);
    expect(Array.from(pcm)).toEqual([0, 32767]);
  });

  it("opprøver 16 kHz til 24 kHz med lineær interpolation", () => {
    const pcm = resampleToPcm16(new Float32Array([0, 1, 0]), 16_000);
    expect(Array.from(pcm)).toEqual([0, 21845, 21845, 0]);
  });

  function continuousReference(input: Float32Array, sourceRate: number) {
    const output: number[] = [];
    for (let outputIndex = 0; ; outputIndex += 1) {
      const position = (outputIndex * sourceRate) / 24_000;
      const lower = Math.floor(position);
      if (lower + 1 >= input.length) break;
      const fraction = position - lower;
      const sample =
        input[lower] * (1 - fraction) + input[lower + 1] * fraction;
      const clamped = Math.max(-1, Math.min(1, sample));
      output.push(Math.round(clamped * (clamped < 0 ? 32_768 : 32_767)));
    }
    return output;
  }

  for (const sourceRate of [16_000, 48_000]) {
    it(`matcher kontinuerlig reference over chunk-grænse ved ${sourceRate} Hz`, () => {
      const input = new Float32Array([0, 0.2, 0.7, -0.4, -0.8, 0.1, 0.9]);
      const resampler = createStreamingPcm16Resampler(sourceRate);
      const first = resampler.push(input.subarray(0, 3));
      const second = resampler.push(input.subarray(3));

      expect([...first, ...second]).toEqual(
        continuousReference(input, sourceRate),
      );
    });
  }

  it("bevarer fase uden drift ved ikke-integral 44,1 kHz rate", () => {
    const input = Float32Array.from({ length: 101 }, (_, index) =>
      Math.sin(index / 7),
    );
    const resampler = createStreamingPcm16Resampler(44_100);
    const chunks = [
      resampler.push(input.subarray(0, 17)),
      resampler.push(input.subarray(17, 46)),
      resampler.push(input.subarray(46)),
    ];

    expect(chunks.flatMap((chunk) => Array.from(chunk))).toEqual(
      continuousReference(input, 44_100),
    );
  });
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});


function keyEvent(
  overrides: Partial<KeyboardEvent> & { key: string; code: string },
): KeyboardEvent {
  return {
    key: overrides.key,
    code: overrides.code,
    ctrlKey: overrides.ctrlKey ?? false,
    metaKey: overrides.metaKey ?? false,
    shiftKey: overrides.shiftKey ?? false,
    altKey: overrides.altKey ?? false,
    repeat: overrides.repeat ?? false,
    preventDefault: overrides.preventDefault ?? vi.fn(),
    stopPropagation: overrides.stopPropagation ?? vi.fn(),
  } as unknown as KeyboardEvent;
}

function dispatchKey(
  type: "keydown" | "keyup",
  init: KeyboardEventInit & { key: string; code: string },
) {
  const event = new KeyboardEvent(type, {
    bubbles: true,
    cancelable: true,
    ...init,
  });
  document.dispatchEvent(event);
  return event;
}

describe("wake-key", () => {
  it("matcher den delte grammatik-fixture", () => {
    for (const fixture of fixtures.valid) {
      expect(parseAccelerator(fixture.accel)).toEqual({
        ctrl: fixture.ctrl,
        shift: fixture.shift,
        alt: fixture.alt,
        code: fixture.code,
      });
    }
    for (const accel of fixtures.invalid) {
      expect(() => parseAccelerator(accel)).toThrow();
    }
  });

  it("matchesAccelerator_ctrl_shift_space", () => {
    expect(
      matchesAccelerator(
        keyEvent({ key: " ", code: "Space", ctrlKey: true, shiftKey: true }),
        "CmdOrCtrl+Shift+Space",
      ),
    ).toBe(true);
  });

  it("matchesAccelerator_fuld_grammatik", () => {
    expect(matchesAccelerator(keyEvent({ key: "a", code: "KeyA", ctrlKey: true }), "Ctrl+A")).toBe(true);
    expect(matchesAccelerator(keyEvent({ key: "5", code: "Digit5", altKey: true }), "Alt+5")).toBe(true);
    expect(matchesAccelerator(keyEvent({ key: "F5", code: "F5", shiftKey: true }), "Shift+F5")).toBe(true);
    expect(
      matchesAccelerator(
        keyEvent({ key: " ", code: "Space", metaKey: true, shiftKey: true }),
        "CmdOrCtrl+Shift+Space",
      ),
    ).toBe(true);
  });

  it("matchesAccelerator_afviser_delmatch", () => {
    // Manglende KRAEVET modifier afviser fortsat.
    expect(
      matchesAccelerator(keyEvent({ key: " ", code: "Space", ctrlKey: true }), "CmdOrCtrl+Shift+Space"),
    ).toBe(false);
    expect(
      matchesAccelerator(keyEvent({ key: " ", code: "Space", shiftKey: true }), "CmdOrCtrl+Shift+Space"),
    ).toBe(false);
  });

  it("matchesAccelerator_ekstra_modifier_diskvalificerer_aldrig", () => {
    // Fantom-modifier-vaernet (2026-07-20, subset-semantik som wake_hotkey.rs
    // og redapting-forlaegget): en stuck/fantom Alt-bit i OS'ets modifier-
    // tilstand maa aldrig goere hotkeyen tavst doed.
    expect(
      matchesAccelerator(
        keyEvent({ key: "a", code: "KeyA", ctrlKey: true, shiftKey: true }),
        "Ctrl+A",
      ),
    ).toBe(true);
    expect(
      matchesAccelerator(
        keyEvent({ key: " ", code: "Space", ctrlKey: true, shiftKey: true, altKey: true }),
        "CmdOrCtrl+Shift+Space",
      ),
    ).toBe(true);
  });

  it("registerWakeKey_uparsebar_accel_kaster", () => {
    expect(() => registerWakeKey("NotAKey", () => {})).toThrow(/Uparsebar/u);
    expect(() => registerWakeKey("Ctrl+", () => {})).toThrow(/Uparsebar/u);
  });

  it("registerWakeKey_ignorerer_event_repeat", () => {
    const onWake = vi.fn();
    const dispose = registerWakeKey("Ctrl+Shift+Space", onWake);
    const repeatEvent = new KeyboardEvent("keydown", {
      key: " ",
      code: "Space",
      ctrlKey: true,
      shiftKey: true,
      repeat: true,
      bubbles: true,
      cancelable: true,
    });
    document.dispatchEvent(repeatEvent);
    // Repeats vaekker ikke, men SKAL stadig interceptes — et matchet tryk
    // maa aldrig naa terminalen, heller ikke som repeat.
    expect(onWake).not.toHaveBeenCalled();
    expect(repeatEvent.defaultPrevented).toBe(true);
    dispose();
  });

  it("registerWakeKey_fyrer_og_preventDefaulter", () => {
    const onWake = vi.fn();
    const dispose = registerWakeKey("Ctrl+Shift+Space", onWake);
    const event = new KeyboardEvent("keydown", {
      key: " ",
      code: "Space",
      ctrlKey: true,
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    });
    const prevented = !document.dispatchEvent(event);
    expect(onWake).toHaveBeenCalledTimes(1);
    expect(prevented || event.defaultPrevented).toBe(true);
    dispose();
  });

  it("registerWakeKey_capture_fase_stopper_propagation", () => {
    const onWake = vi.fn();
    const childSeen = vi.fn();
    const dispose = registerWakeKey("Ctrl+K", onWake);
    const child = document.createElement("div");
    document.body.appendChild(child);
    child.addEventListener("keydown", childSeen);
    child.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "k",
        code: "KeyK",
        ctrlKey: true,
        bubbles: true,
        cancelable: true,
      }),
    );
    expect(onWake).toHaveBeenCalledTimes(1);
    expect(childSeen).not.toHaveBeenCalled();
    child.remove();
    dispose();
  });

  it("dispose_fjerner_listener", () => {
    const onWake = vi.fn();
    const dispose = registerWakeKey("Ctrl+Shift+Space", onWake);
    dispose();
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: " ",
        code: "Space",
        ctrlKey: true,
        shiftKey: true,
        bubbles: true,
      }),
    );
    expect(onWake).not.toHaveBeenCalled();
  });

  it("registerPttKey_press_release_pair", () => {
    const onPress = vi.fn();
    const onRelease = vi.fn();
    const dispose = registerPttKey("Ctrl+Shift+Space", { onPress, onRelease });
    dispatchKey("keydown", {
      key: " ",
      code: "Space",
      ctrlKey: true,
      shiftKey: true,
    });
    dispatchKey("keyup", {
      key: " ",
      code: "Space",
      ctrlKey: true,
      shiftKey: true,
    });
    expect(onPress).toHaveBeenCalledTimes(1);
    expect(onRelease).toHaveBeenCalledTimes(1);
    dispose();
  });

  it("registerPttKey_ignores_repeat", () => {
    const onPress = vi.fn();
    const onRelease = vi.fn();
    const dispose = registerPttKey("Ctrl+Shift+Space", { onPress, onRelease });
    dispatchKey("keydown", {
      key: " ",
      code: "Space",
      ctrlKey: true,
      shiftKey: true,
      repeat: true,
    });
    expect(onPress).not.toHaveBeenCalled();
    expect(onRelease).not.toHaveBeenCalled();
    dispose();
  });

  it("registerPttKey_release_fires_despite_modifier_loss", () => {
    const onPress = vi.fn();
    const onRelease = vi.fn();
    const dispose = registerPttKey("Ctrl+Shift+Space", { onPress, onRelease });
    dispatchKey("keydown", {
      key: " ",
      code: "Space",
      ctrlKey: true,
      shiftKey: true,
    });
    dispatchKey("keyup", { key: " ", code: "Space" });
    expect(onRelease).toHaveBeenCalledTimes(1);
    dispose();
  });

  it("registerPttKey_blur_releases_active_press", () => {
    const onPress = vi.fn();
    const onRelease = vi.fn();
    const dispose = registerPttKey("Ctrl+Shift+Space", { onPress, onRelease });
    dispatchKey("keydown", {
      key: " ",
      code: "Space",
      ctrlKey: true,
      shiftKey: true,
    });
    window.dispatchEvent(new Event("blur"));
    expect(onRelease).toHaveBeenCalledTimes(1);
    window.dispatchEvent(new Event("blur"));
    expect(onRelease).toHaveBeenCalledTimes(1);
    dispose();
  });

  it("supprimerer ikke mens hoejre Alt er nede", () => {
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        code: "AltRight",
        altKey: true,
        bubbles: true,
      }),
    );
    const event = new KeyboardEvent("keydown", {
      code: "Digit8",
      key: "[",
      ctrlKey: true,
      altKey: true,
    });
    expect(matchesAccelerator(event, "Ctrl+Digit8")).toBe(false);
    window.dispatchEvent(new Event("blur"));
  });

  it("bruger fysisk hoejre Alt og ikke AltGraph", () => {
    const event = new KeyboardEvent("keydown", {
      code: "Digit8",
      key: "[",
      ctrlKey: true,
      altKey: true,
    });
    expect(matchesAccelerator(event, "Ctrl+Digit8")).toBe(true);
  });

  it("muse-token matcher aldrig et KeyboardEvent", () => {
    expect(matchesAccelerator(keyEvent({ key: "q", code: "KeyQ" }), "Mouse4")).toBe(false);
  });

  it("skelner mus fra tastatur", () => {
    expect(isKeyboardBinding("Ctrl+Shift+Space")).toBe(true);
    expect(isKeyboardBinding("F9")).toBe(true);
    expect(isKeyboardBinding("Mouse4")).toBe(false);
  });
});

describe("session-toggle", () => {
  it("skifter asleep → waking → awake → sleeping → asleep", async () => {
    const states: SessionToggleState[] = [];
    const wake = vi.fn(async () => undefined);
    const sleep = vi.fn(async () => undefined);
    const toggle = createSessionToggle({
      wake,
      sleep,
      stopCapture: vi.fn(),
      onState: (state) => states.push(state),
    });

    await toggle.toggle();
    await toggle.toggle();

    expect(wake).toHaveBeenCalledTimes(1);
    expect(sleep).toHaveBeenCalledTimes(1);
    expect(states).toEqual(["waking", "awake", "sleeping", "asleep"]);
    expect(toggle.state()).toBe("asleep");
  });

  it("synkroniserer ekstern go_to_sleep så næste hotkey vækker igen", async () => {
    const wake = vi.fn(async () => undefined);
    const toggle = createSessionToggle({
      wake,
      sleep: vi.fn(async () => undefined),
      stopCapture: vi.fn(),
      onState: vi.fn(),
    });

    await toggle.toggle();
    toggle.markAsleep();
    await toggle.toggle();

    expect(wake).toHaveBeenCalledTimes(2);
    expect(toggle.state()).toBe("awake");
  });

  it("returnerer sikkert til asleep når wake fejler", async () => {
    const errors: Error[] = [];
    const states: SessionToggleState[] = [];
    const toggle = createSessionToggle({
      wake: vi.fn(async () => {
        throw new Error("microphone denied");
      }),
      sleep: vi.fn(async () => undefined),
      stopCapture: vi.fn(),
      onState: (state) => states.push(state),
      onError: (error) => errors.push(error),
    });

    await toggle.toggle();

    expect(states).toEqual(["waking", "asleep"]);
    expect(errors.map((error) => error.message)).toEqual(["microphone denied"]);
    expect(toggle.state()).toBe("asleep");
  });

  it("blur_i_awake_stopper_capture_og_draener", async () => {
    const stopCapture = vi.fn();
    const states: SessionToggleState[] = [];
    const toggle = createSessionToggle({
      wake: vi.fn(async () => undefined),
      sleep: vi.fn(async () => undefined),
      stopCapture,
      onState: (state) => states.push(state),
    });
    await toggle.toggle();
    toggle.notifyBlur();
    expect(stopCapture).toHaveBeenCalledTimes(1);
    expect(toggle.state()).toBe("draining");
    expect(states.at(-1)).toBe("draining");
  });

  it("turn_done_i_draining_sover", async () => {
    const sleep = vi.fn(async () => undefined);
    const toggle = createSessionToggle({
      wake: vi.fn(async () => undefined),
      sleep,
      stopCapture: vi.fn(),
      onState: vi.fn(),
    });
    await toggle.toggle();
    toggle.notifyBlur();
    toggle.notifyTurnDone();
    await flush();
    expect(sleep).toHaveBeenCalledTimes(1);
    expect(toggle.state()).toBe("asleep");
  });

  it("draen_timeout_30s_tvangssover", async () => {
    vi.useFakeTimers();
    const sleep = vi.fn(async () => undefined);
    const toggle = createSessionToggle({
      wake: vi.fn(async () => undefined),
      sleep,
      stopCapture: vi.fn(),
      onState: vi.fn(),
      setTimeout,
      clearTimeout,
    });
    await toggle.toggle();
    toggle.notifyBlur();
    await vi.advanceTimersByTimeAsync(30_000);
    expect(sleep).toHaveBeenCalledTimes(1);
    expect(toggle.state()).toBe("asleep");
  });

  it("blur_i_waking_aborterer_generation_og_capture_starter_aldrig", async () => {
    const wakeGate = deferred<void>();
    const startCapture = vi.fn(async () => undefined);
    const sleep = vi.fn(async () => undefined);
    const toggle = createSessionToggle({
      wake: vi.fn(() => wakeGate.promise),
      startCapture,
      sleep,
      stopCapture: vi.fn(),
      onState: vi.fn(),
    });
    const waking = toggle.toggle();
    expect(toggle.state()).toBe("waking");
    toggle.notifyBlur();
    wakeGate.resolve();
    await waking;
    await flush();
    expect(startCapture).not.toHaveBeenCalled();
    expect(sleep).toHaveBeenCalledTimes(1);
    expect(toggle.state()).toBe("asleep");
  });

  it("blur_i_asleep_noop", () => {
    const stopCapture = vi.fn();
    const toggle = createSessionToggle({
      wake: vi.fn(async () => undefined),
      sleep: vi.fn(async () => undefined),
      stopCapture,
      onState: vi.fn(),
    });
    toggle.notifyBlur();
    expect(stopCapture).not.toHaveBeenCalled();
    expect(toggle.state()).toBe("asleep");
  });

  it("toggle_i_draining_afbryder_draenet_og_genoptager_capture", async () => {
    // Fix doede tryk (2026-07-20): foer var toggle-i-draining et tavst no-op
    // — op til 30 s doedt vindue efter blur. Nu betyder trykket "jeg vil tale
    // igen": draen-timeren annulleres, capture genoptages, sessionen forbliver
    // vaagen (wake genkaldes IKKE — den var aldrig sovet).
    vi.useFakeTimers();
    const wake = vi.fn(async () => undefined);
    const sleep = vi.fn(async () => undefined);
    const startCapture = vi.fn(async () => undefined);
    const states: SessionToggleState[] = [];
    const toggle = createSessionToggle({
      wake,
      sleep,
      startCapture,
      stopCapture: vi.fn(),
      onState: (state) => states.push(state),
      setTimeout,
      clearTimeout,
    });
    await toggle.toggle();
    toggle.notifyBlur();
    await toggle.toggle();
    expect(wake).toHaveBeenCalledTimes(1);
    expect(startCapture).toHaveBeenCalledTimes(2);
    expect(toggle.state()).toBe("awake");
    expect(states).toEqual(["waking", "awake", "draining", "waking", "awake"]);
    // Draen-timeren ER annulleret: 30 s senere sover vi stadig ikke.
    await vi.advanceTimersByTimeAsync(30_000);
    expect(sleep).not.toHaveBeenCalled();
    expect(toggle.state()).toBe("awake");
  });

  it("turn_done_efter_genoptaget_draen_sover_ikke", async () => {
    // Den logiske tur der udloeste draenet maa ikke sove sessionen efter at
    // brugeren har genoptaget den.
    const sleep = vi.fn(async () => undefined);
    const toggle = createSessionToggle({
      wake: vi.fn(async () => undefined),
      sleep,
      startCapture: vi.fn(async () => undefined),
      stopCapture: vi.fn(),
      onState: vi.fn(),
    });
    await toggle.toggle();
    toggle.notifyBlur();
    await toggle.toggle();
    toggle.notifyTurnDone();
    await flush();
    expect(sleep).not.toHaveBeenCalled();
    expect(toggle.state()).toBe("awake");
  });

  it("genoptaget_draen_med_capture_fejl_sover_sikkert", async () => {
    const errors: Error[] = [];
    const sleep = vi.fn(async () => undefined);
    const stopCapture = vi.fn();
    let captureCalls = 0;
    const startCapture = vi.fn(async () => {
      captureCalls += 1;
      if (captureCalls === 2) throw new Error("mic gone");
    });
    const toggle = createSessionToggle({
      wake: vi.fn(async () => undefined),
      sleep,
      startCapture,
      stopCapture,
      onState: vi.fn(),
      onError: (error) => errors.push(error),
    });
    await toggle.toggle();
    toggle.notifyBlur();
    await toggle.toggle();
    expect(errors.map((error) => error.message)).toEqual(["mic gone"]);
    // Sessionen var vaagen under draenet — fejlvejen skal sove den RIGTIGT
    // (deps.sleep), ikke bare stemple asleep.
    expect(sleep).toHaveBeenCalledTimes(1);
    expect(toggle.state()).toBe("asleep");
  });

  it("blur_under_genoptagelse_aborterer_generation_og_sover", async () => {
    const captureGate = deferred<void>();
    const sleep = vi.fn(async () => undefined);
    const stopCapture = vi.fn();
    let captureCalls = 0;
    const startCapture = vi.fn(() => {
      captureCalls += 1;
      return captureCalls === 2 ? captureGate.promise : Promise.resolve();
    });
    const toggle = createSessionToggle({
      wake: vi.fn(async () => undefined),
      sleep,
      startCapture,
      stopCapture,
      onState: vi.fn(),
    });
    await toggle.toggle();
    toggle.notifyBlur();
    const resuming = toggle.toggle();
    expect(toggle.state()).toBe("waking");
    // Nyt blur MIDT i genoptagelsen: generation bumpes, capture skal stoppes
    // og sessionen soves naar den langsomme startCapture endelig lander.
    toggle.notifyBlur();
    captureGate.resolve();
    await resuming;
    await flush();
    expect(stopCapture).toHaveBeenCalledTimes(2);
    expect(sleep).toHaveBeenCalledTimes(1);
    expect(toggle.state()).toBe("asleep");
  });
});
