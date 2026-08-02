import { describe, expect, it, vi } from "vitest";
import {
  LEVEL_NOISE_FLOOR_RMS,
  LEVEL_SPEECH_CEILING_RMS,
  normalizeLevel,
  rmsOfPcm16,
  withLevelMeter,
} from "./orbLevel";
import type { AudioCapture, StartAudioCapture } from "./ptt";

function pcm16Of(values: number[]): ArrayBuffer {
  const chunk = new ArrayBuffer(values.length * 2);
  const view = new DataView(chunk);
  values.forEach((value, index) => view.setInt16(index * 2, value, true));
  return chunk;
}

// Facit genbrugt fra Rust-forlæggets tests (voice_recorder.rs).
describe("rmsOfPcm16", () => {
  it("tom og stilhed er nul", () => {
    expect(rmsOfPcm16(new ArrayBuffer(0))).toBe(0);
    expect(rmsOfPcm16(pcm16Of(Array<number>(4_800).fill(0)))).toBe(0);
  });

  it("fuldskala-firkantbølge er ~1", () => {
    const samples = Array.from({ length: 4_800 }, (_, index) =>
      index % 2 === 0 ? 32_767 : -32_767,
    );
    expect(rmsOfPcm16(pcm16Of(samples))).toBeCloseTo(1, 3);
  });

  it("halvskala-firkantbølge er ~0.5", () => {
    const samples = Array.from({ length: 4_800 }, (_, index) =>
      index % 2 === 0 ? 16_384 : -16_384,
    );
    expect(rmsOfPcm16(pcm16Of(samples))).toBeCloseTo(0.5, 2);
  });

  it("en ulige trailing byte ignoreres", () => {
    const chunk = new ArrayBuffer(3);
    new DataView(chunk).setInt16(0, 16_384, true);
    expect(rmsOfPcm16(chunk)).toBeCloseTo(0.5, 2);
  });
});

describe("normalizeLevel", () => {
  it("clamper stilhed og fuld skala", () => {
    expect(normalizeLevel(0)).toBe(0);
    expect(normalizeLevel(LEVEL_NOISE_FLOOR_RMS * 0.5)).toBe(0);
    expect(normalizeLevel(LEVEL_SPEECH_CEILING_RMS)).toBe(1);
    expect(normalizeLevel(0.707)).toBe(1);
  });

  it("lægger normal tale (RMS 0.05-0.20) i midterbåndet", () => {
    const soft = normalizeLevel(0.05);
    const loud = normalizeLevel(0.2);
    expect(soft).toBeGreaterThan(0.25);
    expect(loud).toBeLessThan(0.9);
    expect(loud).toBeGreaterThan(soft);
  });
});

describe("withLevelMeter", () => {
  it("videresender chunks byte-uændret og emitter normaliserede niveauer", async () => {
    const forwarded: ArrayBuffer[] = [];
    const capture: AudioCapture = { stop: vi.fn(async () => {}) };
    let innerOnChunk: ((chunk: ArrayBuffer) => void) | null = null;
    const start: StartAudioCapture = async (onChunk) => {
      innerOnChunk = onChunk;
      return capture;
    };
    const levels: number[] = [];

    const metered = withLevelMeter(start, (level) => levels.push(level));
    const opened = await metered((chunk) => forwarded.push(chunk));

    const chunk = pcm16Of(Array<number>(100).fill(16_384));
    innerOnChunk!(chunk);

    expect(forwarded).toEqual([chunk]);
    // Konstant halvskala: RMS ~0.5 ligger over tale-loftet -> fuldt udslag.
    expect(levels).toEqual([1]);
    expect(opened).toBe(capture);
  });
});
