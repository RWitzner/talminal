import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createPcmPlayer } from "./audioPlayer";

class FakeAnalyserNode {
  fftSize = 2_048;
  timeDomain: Float32Array = new Float32Array(0);
  readonly connect = vi.fn();
  readonly getFloatTimeDomainData = vi.fn((data: Float32Array) => {
    data.set(this.timeDomain.subarray(0, data.length));
  });
}

class FakeAudioContext {
  static instances: FakeAudioContext[] = [];

  readonly sampleRate: number;
  state: AudioContextState = "suspended";
  currentTime = 1;
  destination = {} as AudioDestinationNode;
  readonly analyser = new FakeAnalyserNode();
  readonly resume = vi.fn(async () => {
    this.state = "running";
  });
  readonly close = vi.fn(async () => {
    this.state = "closed";
  });
  readonly channel = new Float32Array(0);
  readonly source = {
    buffer: null as AudioBuffer | null,
    connect: vi.fn(),
    start: vi.fn((startsAt: number) => {
      const duration = this.source.buffer?.duration ?? 0;
      this.currentTime = startsAt + duration;
    }),
  };

  constructor(options?: AudioContextOptions) {
    this.sampleRate = options?.sampleRate ?? 0;
    FakeAudioContext.instances.push(this);
  }

  createBuffer(_channels: number, length: number, sampleRate: number): AudioBuffer {
    const channel = new Float32Array(length);
    Object.defineProperty(this, "channel", { value: channel });
    return {
      duration: length / sampleRate,
      getChannelData: () => channel,
    } as unknown as AudioBuffer;
  }

  createBufferSource(): AudioBufferSourceNode {
    return this.source as unknown as AudioBufferSourceNode;
  }

  createAnalyser(): AnalyserNode {
    return this.analyser as unknown as AnalyserNode;
  }
}

describe("createPcmPlayer", () => {
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

  it("enqueue converts 24 kHz s16le PCM, schedules it, and drain waits for the queue", async () => {
    const player = createPcmPlayer();
    const chunk = new ArrayBuffer(4);
    const view = new DataView(chunk);
    view.setInt16(0, -32_768, true);
    view.setInt16(2, 16_384, true);

    await player.enqueue(chunk);
    await player.drain();

    const context = FakeAudioContext.instances[0];
    expect(context.sampleRate).toBe(24_000);
    expect(context.resume).toHaveBeenCalledOnce();
    expect([...context.channel]).toEqual([-1, 0.5]);
    // Kilderne gaar gennem analyser-tappet (orbens output-niveau), som
    // selv er koblet paa destination.
    expect(context.source.connect).toHaveBeenCalledWith(context.analyser);
    expect(context.analyser.connect).toHaveBeenCalledWith(context.destination);
    expect(context.source.start).toHaveBeenCalledWith(1);
  });

  it("getOutputLevel er 0 uden context og laeser normaliseret RMS fra analyseren", async () => {
    const player = createPcmPlayer();
    expect(player.getOutputLevel()).toBe(0);

    await player.enqueue(new Uint8Array([0, 0]).buffer);
    const context = FakeAudioContext.instances[0];
    context.analyser.timeDomain = new Float32Array(
      context.analyser.fftSize,
    ).fill(0.5);
    // RMS 0.5 ligger over tale-loftet (0.30) -> fuldt udslag.
    expect(player.getOutputLevel()).toBe(1);
    expect(context.analyser.getFloatTimeDomainData).toHaveBeenCalled();

    context.analyser.timeDomain = new Float32Array(context.analyser.fftSize);
    expect(player.getOutputLevel()).toBe(0);
  });

  it("getOutputLevel falder til 0 efter close og analyseren genskabes ved naeste enqueue", async () => {
    const player = createPcmPlayer();
    await player.enqueue(new Uint8Array([0, 0]).buffer);
    const firstContext = FakeAudioContext.instances[0];
    firstContext.analyser.timeDomain = new Float32Array(
      firstContext.analyser.fftSize,
    ).fill(0.5);
    expect(player.getOutputLevel()).toBe(1);

    await player.close();
    expect(player.getOutputLevel()).toBe(0);

    await player.enqueue(new Uint8Array([0, 0]).buffer);
    const secondContext = FakeAudioContext.instances[1];
    expect(secondContext.analyser).not.toBe(firstContext.analyser);
    expect(secondContext.analyser.connect).toHaveBeenCalledWith(
      secondContext.destination,
    );
  });

  it("close stops the context and the same player lazily recreates it", async () => {
    const player = createPcmPlayer();
    const chunk = new Uint8Array([0, 0]).buffer;

    await player.enqueue(chunk);
    const firstContext = FakeAudioContext.instances[0];
    await player.close();
    expect(firstContext.close).toHaveBeenCalledOnce();

    await player.enqueue(chunk);
    expect(FakeAudioContext.instances).toHaveLength(2);
    expect(FakeAudioContext.instances[1]).not.toBe(firstContext);
  });
});
