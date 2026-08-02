import type { SttClient } from "./stt";
import { getActiveVoiceTrace, markPerf } from "../perfTrace";
import { describeMicrophoneFailure, MicrophoneError } from "./micError";

export type PttState = "idle" | "listening" | "finalizing";
export type SessionToggleState =
  | "asleep"
  | "waking"
  | "awake"
  | "draining"
  | "sleeping";

export interface SessionToggle {
  toggle(): Promise<void>;
  notifyBlur(): void;
  notifyFocus(): void;
  notifyTurnDone(): void;
  markAsleep(): void;
  state(): SessionToggleState;
}

type ParsedAccelerator = {
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
  code: string;
};

const MOUSE_CODES = ["Mouse1", "Mouse2", "Mouse3", "Mouse4", "Mouse5"];

const KEY_CODES = [
  ...Array.from({ length: 26 }, (_, i) => `Key${String.fromCharCode(65 + i)}`),
  ...Array.from({ length: 10 }, (_, i) => `Digit${i}`),
  ...Array.from({ length: 12 }, (_, i) => `F${i + 1}`),
  "Space", "Escape", "Enter", "Tab", "Backspace",
  "ArrowUp", "ArrowLeft", "ArrowRight", "ArrowDown",
  "Insert", "Delete", "Home", "End", "PageUp", "PageDown",
  "Minus", "Equal", "BracketLeft", "BracketRight", "Backslash",
  "Semicolon", "Quote", "Backquote", "Comma", "Period", "Slash",
  "IntlBackslash",
];

const CODE_BY_LOWER = new Map(
  [...KEY_CODES, ...MOUSE_CODES].map((code) => [code.toLowerCase(), code]),
);

function allowsBare(code: string): boolean {
  return MOUSE_CODES.includes(code) || /^F([1-9]|1[0-2])$/u.test(code);
}

function migrateV1Token(token: string): string | null {
  if (token.length !== 1) return null;
  if (/^[a-z]$/u.test(token)) return `key${token}`;
  if (/^[0-9]$/u.test(token)) return `digit${token}`;
  return null;
}

export function parseAccelerator(accel: string): ParsedAccelerator {
  const parts = accel.split("+").map((part) => part.trim()).filter(Boolean);
  if (parts.length === 0) {
    throw new Error(`Uparsebar accelerator: ${accel}`);
  }
  let ctrl = false;
  let shift = false;
  let alt = false;
  let code: string | null = null;
  for (const part of parts) {
    const token = part.toLocaleLowerCase();
    if (token === "cmdorctrl" || token === "ctrl" || token === "control") {
      ctrl = true;
      continue;
    }
    if (token === "shift") {
      shift = true;
      continue;
    }
    if (token === "alt" || token === "option") {
      alt = true;
      continue;
    }
    if (code !== null) {
      throw new Error(`Uparsebar accelerator: ${accel}`);
    }
    const resolved =
      CODE_BY_LOWER.get(token) ??
      CODE_BY_LOWER.get(migrateV1Token(token) ?? "\0") ??
      null;
    if (resolved === null) throw new Error(`Uparsebar accelerator: ${accel}`);
    code = resolved;
  }
  if (code === null) {
    throw new Error(`Uparsebar accelerator: ${accel}`);
  }
  if (!ctrl && !shift && !alt && !allowsBare(code)) {
    throw new Error(
      `${accel} kræver mindst én modifier — kun F1-F12 og musetaster må stå alene`,
    );
  }
  return { ctrl, shift, alt, code };
}

export function isKeyboardBinding(accel: string): boolean {
  return !MOUSE_CODES.includes(parseAccelerator(accel).code);
}

let rightAltDown = false;
let trackingInstalled = false;

function ensureRightAltTracking(): void {
  if (trackingInstalled || typeof document === "undefined") return;
  trackingInstalled = true;
  document.addEventListener(
    "keydown",
    (event) => {
      if (event.code === "AltRight") rightAltDown = true;
    },
    true,
  );
  document.addEventListener(
    "keyup",
    (event) => {
      if (event.code === "AltRight") rightAltDown = false;
    },
    true,
  );
  window.addEventListener("blur", () => {
    rightAltDown = false;
  });
}

export function isRightAltDown(): boolean {
  return rightAltDown;
}

export function matchesAccelerator(e: KeyboardEvent, accel: string): boolean {
  ensureRightAltTracking();
  const parsed = parseAccelerator(accel);
  if (MOUSE_CODES.includes(parsed.code)) return false;
  if (rightAltDown) return false;
  const ctrlPressed = e.ctrlKey || e.metaKey;
  if (parsed.ctrl && !ctrlPressed) return false;
  if (parsed.shift && !e.shiftKey) return false;
  if (parsed.alt && !e.altKey) return false;
  if (!parsed.ctrl && !parsed.shift && !parsed.alt) {
    if (ctrlPressed || e.shiftKey || e.altKey) return false;
  }
  return e.code === parsed.code;
}

export function registerWakeKey(accel: string, onWake: () => void): () => void {
  parseAccelerator(accel);
  ensureRightAltTracking();
  const handler = (event: KeyboardEvent) => {
    if (!matchesAccelerator(event, accel)) return;
    // Matchede tryk interceptes ALTID (arbitrations-invariant: vaektasten
    // naar aldrig terminalen) — men repeats vaekker ikke (M8-oscillation).
    event.preventDefault();
    event.stopPropagation();
    if (event.repeat) return;
    onWake();
  };
  document.addEventListener("keydown", handler, true);
  return () => document.removeEventListener("keydown", handler, true);
}

export function registerPttKey(
  accel: string,
  handlers: { onPress(): void; onRelease(): void },
): () => void {
  const parsed = parseAccelerator(accel);
  ensureRightAltTracking();
  let active = false;

  const onKeyDown = (event: KeyboardEvent) => {
    if (!matchesAccelerator(event, accel)) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.repeat || active) return;
    active = true;
    handlers.onPress();
  };

  const onKeyUp = (event: KeyboardEvent) => {
    if (!active || event.code !== parsed.code) return;
    event.preventDefault();
    event.stopPropagation();
    active = false;
    handlers.onRelease();
  };

  const onBlur = () => {
    if (!active) return;
    active = false;
    handlers.onRelease();
  };

  document.addEventListener("keydown", onKeyDown, true);
  document.addEventListener("keyup", onKeyUp, true);
  window.addEventListener("blur", onBlur);
  return () => {
    active = false;
    document.removeEventListener("keydown", onKeyDown, true);
    document.removeEventListener("keyup", onKeyUp, true);
    window.removeEventListener("blur", onBlur);
  };
}

export function createSessionToggle(deps: {
  wake(): Promise<void>;
  sleep(): Promise<void>;
  stopCapture(): void;
  startCapture?: () => Promise<void>;
  onState(state: SessionToggleState): void;
  onError?(error: Error): void;
  drainTimeoutMs?: number;
  setTimeout?: typeof setTimeout;
  clearTimeout?: typeof clearTimeout;
}): SessionToggle {
  let current: SessionToggleState = "asleep";
  let wakeGeneration = 0;
  let drainTimer: ReturnType<typeof setTimeout> | null = null;
  const schedule = deps.setTimeout ?? setTimeout;
  const clear = deps.clearTimeout ?? clearTimeout;
  const drainTimeoutMs = deps.drainTimeoutMs ?? 30_000;

  function transition(next: SessionToggleState) {
    if (current === next) return;
    current = next;
    deps.onState(next);
  }

  function clearDrainTimer() {
    if (drainTimer !== null) {
      clear(drainTimer);
      drainTimer = null;
    }
  }

  async function sleepNow() {
    clearDrainTimer();
    if (current === "asleep") return;
    transition("sleeping");
    try {
      await deps.sleep();
      transition("asleep");
    } catch (error) {
      deps.onError?.(asError(error));
      transition("asleep");
    }
  }

  return {
    async toggle() {
      if (current === "waking" || current === "sleeping") {
        // Transiente tilstande: trykket ignoreres bevidst, men aldrig tavst
        // (doede-tryk-diagnostik, 2026-07-20) — HUD'en viser tilstanden.
        console.debug(`voice.toggle.ignored_in_${current}`);
        return;
      }
      if (current === "draining") {
        // Fix doede tryk (2026-07-20): et tryk under draenet betyder "jeg vil
        // tale igen". Sessionen er stadig vaagen — kun mikrofonen er stoppet —
        // saa draenet afbrydes og capture genoptages i stedet for at trykket
        // sluges tavst (op til 30 s doedt vindue foer timeouten sov os).
        clearDrainTimer();
        const generation = ++wakeGeneration;
        transition("waking");
        try {
          if (deps.startCapture) {
            await deps.startCapture();
            if (generation !== wakeGeneration) {
              deps.stopCapture();
              await sleepNow();
              return;
            }
          }
          transition("awake");
        } catch (error) {
          deps.onError?.(asError(error));
          deps.stopCapture();
          await sleepNow();
        }
        return;
      }
      if (current === "asleep") {
        const generation = ++wakeGeneration;
        transition("waking");
        try {
          await deps.wake();
          if (generation !== wakeGeneration) {
            await sleepNow();
            return;
          }
          if (deps.startCapture) {
            await deps.startCapture();
            if (generation !== wakeGeneration) {
              deps.stopCapture();
              await sleepNow();
              return;
            }
          }
          transition("awake");
        } catch (error) {
          deps.onError?.(asError(error));
          deps.stopCapture();
          clearDrainTimer();
          transition("asleep");
        }
        return;
      }
      if (current === "awake") {
        await sleepNow();
      }
    },
    notifyBlur() {
      if (current === "waking") {
        wakeGeneration += 1;
        return;
      }
      if (current !== "awake") return;
      deps.stopCapture();
      transition("draining");
      clearDrainTimer();
      drainTimer = schedule(() => {
        drainTimer = null;
        if (current === "draining") void sleepNow();
      }, drainTimeoutMs);
    },
    notifyFocus() {
      // Fokus genstarter aldrig voice automatisk.
    },
    notifyTurnDone() {
      if (current !== "draining") return;
      void sleepNow();
    },
    markAsleep() {
      clearDrainTimer();
      transition("asleep");
    },
    state() {
      return current;
    },
  };
}

export interface AudioCapture {
  stop(): Promise<void>;
}

export type StartAudioCapture = (
  onChunk: (chunk: ArrayBuffer) => void,
) => Promise<AudioCapture>;

const TARGET_SAMPLE_RATE = 24_000;

function toPcm16(sample: number): number {
  const clamped = Math.max(-1, Math.min(1, sample));
  return Math.round(clamped * (clamped < 0 ? 32_768 : 32_767));
}

/** Resample mono Web Audio floats to signed little-endian PCM16 at 24 kHz. */
export function resampleToPcm16(
  input: Float32Array,
  sourceRate: number,
): Int16Array {
  if (sourceRate <= 0) throw new Error("Invalid source sample rate");
  if (input.length === 0) return new Int16Array();

  const outputLength = Math.max(
    1,
    Math.floor((input.length * TARGET_SAMPLE_RATE) / sourceRate),
  );
  const output = new Int16Array(outputLength);
  const sourceStep = sourceRate / TARGET_SAMPLE_RATE;

  for (let i = 0; i < outputLength; i += 1) {
    const sourcePosition = Math.min(i * sourceStep, input.length - 1);
    const lower = Math.floor(sourcePosition);
    const upper = Math.min(lower + 1, input.length - 1);
    const fraction = sourcePosition - lower;
    const sample =
      input[lower] * (1 - fraction) + input[upper] * fraction;
    output[i] = toPcm16(sample);
  }

  return output;
}

/** Preserve interpolation phase across Web Audio callback boundaries. */
export function createStreamingPcm16Resampler(sourceRate: number): {
  push(input: Float32Array): Int16Array;
} {
  if (sourceRate <= 0) throw new Error("Invalid source sample rate");

  let pending = new Float32Array();
  let sampleOffset = 0;
  let nextOutputIndex = 0;

  return {
    push(input) {
      if (input.length === 0) return new Int16Array();

      const samples = new Float32Array(pending.length + input.length);
      samples.set(pending);
      samples.set(input, pending.length);

      const output: number[] = [];
      const sampleEnd = sampleOffset + samples.length;
      while (true) {
        const position =
          (nextOutputIndex * sourceRate) / TARGET_SAMPLE_RATE;
        const lower = Math.floor(position);
        if (lower + 1 >= sampleEnd) break;

        const localPosition = position - sampleOffset;
        const localLower = Math.floor(localPosition);
        const fraction = localPosition - localLower;
        const sample =
          samples[localLower] * (1 - fraction) +
          samples[localLower + 1] * fraction;
        output.push(toPcm16(sample));
        nextOutputIndex += 1;
      }

      const nextPosition =
        (nextOutputIndex * sourceRate) / TARGET_SAMPLE_RATE;
      const consumed = Math.max(0, Math.floor(nextPosition) - sampleOffset);
      pending = samples.slice(consumed);
      sampleOffset += consumed;
      return Int16Array.from(output);
    },
  };
}

/** Browser-only microphone capture. No browser APIs are touched until called. */
export async function startBrowserCapture(
  onChunk: (chunk: ArrayBuffer) => void,
): Promise<AudioCapture> {
  if (!navigator.mediaDevices?.getUserMedia) {
    throw new Error(
      "Mikrofon-adgang findes ikke i denne visning. Genstart Talminal.",
    );
  }

  // getUserMedia er det ENESTE sted mikrofon-adgang kan afvises, og fejlen
  // herfra vises ordret for brugeren. Den oversaettes derfor med det samme —
  // opstroems ved den ved ikke laengere hvad DOMException-navnet betoed.
  let stream: MediaStream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        channelCount: 1,
        sampleRate: { ideal: TARGET_SAMPLE_RATE },
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
      },
    });
  } catch (error) {
    throw new MicrophoneError(describeMicrophoneFailure(error), error);
  }

  let context: AudioContext | null = null;
  let source: MediaStreamAudioSourceNode | null = null;
  let processor: ScriptProcessorNode | null = null;
  let stopped = false;

  async function stop() {
    if (stopped) return;
    stopped = true;
    if (processor) {
      processor.onaudioprocess = null;
      processor.disconnect();
    }
    source?.disconnect();
    for (const track of stream.getTracks()) track.stop();
    if (context && context.state !== "closed") await context.close();
  }

  try {
    context = new AudioContext({ sampleRate: TARGET_SAMPLE_RATE });
    const resampler = createStreamingPcm16Resampler(context.sampleRate);
    source = context.createMediaStreamSource(stream);
    processor = context.createScriptProcessor(4096, 1, 1);
    processor.onaudioprocess = (event) => {
      if (stopped) return;
      const floats = event.inputBuffer.getChannelData(0);
      const pcm = resampler.push(floats);
      if (pcm.length === 0) return;
      const chunk = new Uint8Array(pcm.byteLength);
      chunk.set(new Uint8Array(pcm.buffer, pcm.byteOffset, pcm.byteLength));
      onChunk(chunk.buffer);
    };
    source.connect(processor);
    processor.connect(context.destination);
    if (context.state === "suspended") await context.resume();
    return { stop };
  } catch (error) {
    await stop();
    throw error;
  }
}

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}

export function createPtt(deps: {
  stt: SttClient;
  onState: (s: PttState) => void;
  onFinal: (text: string) => void;
  onError?: (error: Error) => void;
  startCapture?: StartAudioCapture;
}): { press(): void; release(): void; cancel(): void } {
  const startCapture = deps.startCapture ?? startBrowserCapture;
  let state: PttState = "idle";
  let generation = 0;
  let ready: Promise<void> | null = null;
  let capture: AudioCapture | null = null;
  let sttStarted = false;
  let sttReady = false;
  let bufferedAudio: ArrayBuffer[] = [];
  let failing = false;
  let errorReported = false;

  function transition(next: PttState) {
    state = next;
    deps.onState(next);
  }

  function reportError(error: unknown) {
    if (errorReported) return;
    errorReported = true;
    deps.onError?.(asError(error));
  }

  async function stopCapture(): Promise<Error | null> {
    const current = capture;
    capture = null;
    try {
      await current?.stop();
      return null;
    } catch (error) {
      return asError(error);
    }
  }

  async function stopStt(): Promise<
    { text: string | null; error: Error | null }
  > {
    if (!sttStarted) return { text: null, error: null };
    sttStarted = false;
    try {
      return { text: await deps.stt.stop(), error: null };
    } catch (error) {
      return { text: null, error: asError(error) };
    }
  }

  function returnIdle(token: number) {
    if (generation === token && state !== "idle") transition("idle");
  }

  async function failCycle(token: number, primaryError: unknown) {
    if (generation !== token || state === "idle" || failing) return;
    failing = true;
    state = "finalizing";
    bufferedAudio = [];
    const captureError = await stopCapture();
    const { error: sttError } = await stopStt();
    if (generation !== token) {
      failing = false;
      return;
    }
    reportError(primaryError ?? captureError ?? sttError);
    returnIdle(token);
    failing = false;
  }

  return {
    press() {
      if (state !== "idle") return;

      const token = ++generation;
      sttStarted = false;
      sttReady = false;
      bufferedAudio = [];
      capture = null;
      failing = false;
      errorReported = false;
      transition("listening");

      const captureReady = startCapture((chunk) => {
        if (generation !== token || state !== "listening") return;
        if (!sttReady) {
          bufferedAudio.push(chunk);
          return;
        }
        try {
          deps.stt.pushAudio(chunk);
        } catch (error) {
          void failCycle(token, error);
        }
      }).then(async (openedCapture) => {
        if (generation === token && state !== "idle") {
          capture = openedCapture;
        } else {
          await openedCapture.stop();
        }
      });

      const sttReadyPromise = (async () => {
        await deps.stt.start();
        sttStarted = true;
        sttReady = true;

        if (generation !== token || state === "idle") {
          await stopStt();
          return;
        }

        const queued = bufferedAudio;
        bufferedAudio = [];
        for (const chunk of queued) deps.stt.pushAudio(chunk);
      })();

      ready = Promise.allSettled([captureReady, sttReadyPromise]).then(
        (results) => {
          const failure = results.find(
            (result): result is PromiseRejectedResult =>
              result.status === "rejected",
          );
          if (failure) throw failure.reason;
        },
      );

      void ready.catch((error) => {
        if (generation === token && state === "listening") {
          void failCycle(token, error);
        }
      });
    },

    release() {
      if (state !== "listening") return;

      const token = generation;
      transition("finalizing");
      const currentReady = ready;

      void (async () => {
        const perfTrace = PERF_ENABLED ? getActiveVoiceTrace() : null;
        let setupError: Error | null = null;
        const setupStarted = PERF_ENABLED ? performance.now() : 0;
        try {
          await currentReady;
        } catch (error) {
          setupError = asError(error);
        }
        if (PERF_ENABLED) {
          markPerf(perfTrace, "voice.release.residual_setup.end", {
            duration_ms: performance.now() - setupStarted,
            ok: setupError === null,
          });
        }

        const captureStarted = PERF_ENABLED ? performance.now() : 0;
        const captureError = await stopCapture();
        if (PERF_ENABLED) {
          markPerf(perfTrace, "voice.capture.stop.end", {
            duration_ms: performance.now() - captureStarted,
            ok: captureError === null,
          });
        }
        const sttStartedAt = PERF_ENABLED ? performance.now() : 0;
        const { text, error: sttError } = await stopStt();
        if (PERF_ENABLED) {
          markPerf(perfTrace, "voice.stt.stop.end", {
            duration_ms: performance.now() - sttStartedAt,
            ok: sttError === null,
            transcript_chars: text?.length ?? null,
          });
        }
        const error = setupError ?? captureError ?? sttError;

        if (generation !== token) return;
        if (error) reportError(error);
        else if (text !== null) deps.onFinal(text);

        bufferedAudio = [];
        returnIdle(token);
      })();
    },

    cancel() {
      generation += 1;
      const currentCapture = capture;
      const shouldStopStt = sttStarted;
      capture = null;
      sttStarted = false;
      sttReady = false;
      bufferedAudio = [];
      ready = null;
      failing = false;
      if (state !== "idle") transition("idle");

      void currentCapture?.stop().catch(() => undefined);
      if (shouldStopStt) {
        deps.stt.abort();
      }
    },
  };
}
