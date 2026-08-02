import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AudioCapture, StartAudioCapture } from "./ptt";
import {
  BLIP_AMPLITUDE,
  createSoundPlayer,
  renderVoiceSound,
  SOUND_SAMPLE_RATE,
  withSoundFeedback,
  type VoiceSound,
} from "./sound";

const ALL_SOUNDS: VoiceSound[] = ["start", "stop", "error"];

// Facit genbrugt fra Rust-forlæggets tests (Redapting sound.rs) — minus
// den stille hale, som var en rodio-teardown-guard og bevidst er udeladt.
describe("renderVoiceSound", () => {
  it("har den forventede varighed (offset + tone)", () => {
    const expected = Math.floor((SOUND_SAMPLE_RATE * (80 + 120)) / 1_000);
    for (const sound of ALL_SOUNDS) {
      expect(renderVoiceSound(sound)).toHaveLength(expected);
    }
  });

  it("er subtil men reelt hørbar", () => {
    for (const sound of ALL_SOUNDS) {
      const max = renderVoiceSound(sound).reduce(
        (acc, sample) => Math.max(acc, Math.abs(sample)),
        0,
      );
      expect(max).toBeLessThanOrEqual(BLIP_AMPLITUDE);
      expect(max).toBeGreaterThan(0.05);
    }
  });

  it("starter og slutter klik-frit (~0)", () => {
    for (const sound of ALL_SOUNDS) {
      const samples = renderVoiceSound(sound);
      expect(Math.abs(samples[0])).toBeLessThan(0.01);
      expect(Math.abs(samples[samples.length - 1])).toBeLessThan(0.01);
    }
  });

  it("alle tre lyde er forskellige", () => {
    const [start, stop, error] = ALL_SOUNDS.map((sound) => [
      ...renderVoiceSound(sound),
    ]);
    expect(start).not.toEqual(stop);
    expect(start).not.toEqual(error);
    expect(stop).not.toEqual(error);
  });
});

describe("createSoundPlayer", () => {
  class FakeAudioContext {
    static instances: FakeAudioContext[] = [];
    readonly sampleRate: number;
    state: AudioContextState = "suspended";
    destination = {} as AudioDestinationNode;
    readonly resume = vi.fn(async () => {
      this.state = "running";
    });
    readonly close = vi.fn(async () => {
      this.state = "closed";
    });
    readonly sources: Array<{
      buffer: AudioBuffer | null;
      connect: ReturnType<typeof vi.fn>;
      start: ReturnType<typeof vi.fn>;
    }> = [];

    constructor(options?: AudioContextOptions) {
      this.sampleRate = options?.sampleRate ?? 0;
      FakeAudioContext.instances.push(this);
    }

    createBuffer(
      _channels: number,
      length: number,
      _sampleRate: number,
    ): AudioBuffer {
      const channel = new Float32Array(length);
      return {
        getChannelData: () => channel,
      } as unknown as AudioBuffer;
    }

    createBufferSource(): AudioBufferSourceNode {
      const source = {
        buffer: null as AudioBuffer | null,
        connect: vi.fn(),
        start: vi.fn(),
      };
      this.sources.push(source);
      return source as unknown as AudioBufferSourceNode;
    }
  }

  beforeEach(() => {
    FakeAudioContext.instances = [];
    vi.stubGlobal(
      "AudioContext",
      FakeAudioContext as unknown as typeof AudioContext,
    );
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("afspiller fire-and-forget på en lazy, genbrugt context", async () => {
    const player = createSoundPlayer();
    expect(FakeAudioContext.instances).toHaveLength(0);

    player.play("start");
    player.play("stop");
    await Promise.resolve();
    await Promise.resolve();

    expect(FakeAudioContext.instances).toHaveLength(1);
    const context = FakeAudioContext.instances[0];
    expect(context.sampleRate).toBe(SOUND_SAMPLE_RATE);
    expect(context.sources).toHaveLength(2);
    for (const source of context.sources) {
      expect(source.connect).toHaveBeenCalledWith(context.destination);
      expect(source.start).toHaveBeenCalledOnce();
    }
    await player.close();
    expect(context.close).toHaveBeenCalledOnce();
  });

  it("sluger afspilningsfejl — play kaster aldrig", async () => {
    vi.stubGlobal(
      "AudioContext",
      class {
        constructor() {
          throw new Error("intet lydkort");
        }
      } as unknown as typeof AudioContext,
    );
    const player = createSoundPlayer();
    expect(() => player.play("error")).not.toThrow();
    await Promise.resolve();
    await player.close();
  });
});

describe("withSoundFeedback", () => {
  it("spiller start når mikrofonen er åben og stop når den lukkes", async () => {
    const played: VoiceSound[] = [];
    const capture: AudioCapture = { stop: vi.fn(async () => {}) };
    const start: StartAudioCapture = async () => capture;

    const wrapped = withSoundFeedback(start, (sound) => played.push(sound));
    const opened = await wrapped(() => {});
    expect(played).toEqual(["start"]);

    await opened.stop();
    expect(played).toEqual(["start", "stop"]);

    // Idempotent stop: ingen dobbelt stop-lyd.
    await opened.stop();
    expect(played).toEqual(["start", "stop"]);
  });

  it("spiller stop selv når stop fejler (mikrofonen ER lukket)", async () => {
    const played: VoiceSound[] = [];
    const capture: AudioCapture = {
      stop: vi.fn(async () => {
        throw new Error("device væk");
      }),
    };
    const wrapped = withSoundFeedback(
      async () => capture,
      (sound) => played.push(sound),
    );
    const opened = await wrapped(() => {});
    await expect(opened.stop()).rejects.toThrow("device væk");
    expect(played).toEqual(["start", "stop"]);
  });

  it("spiller ingen start-lyd når åbningen fejler", async () => {
    const played: VoiceSound[] = [];
    const wrapped = withSoundFeedback(
      async () => {
        throw new Error("ingen mikrofon");
      },
      (sound) => played.push(sound),
    );
    await expect(wrapped(() => {})).rejects.toThrow("ingen mikrofon");
    expect(played).toEqual([]);
  });
});
