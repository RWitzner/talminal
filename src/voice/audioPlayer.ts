import { normalizeLevel } from "./orbLevel";
import {
  flushPerfTrace,
  getActiveVoiceTrace,
  markPerf,
  markPerfOnce,
} from "../perfTrace";

export interface PcmPlayer {
  enqueue(chunk: ArrayBuffer): Promise<void>;
  drain(): Promise<void>;
  close(): Promise<void>;
  getOutputLevel(): number;
}

export function createPcmPlayer(): PcmPlayer {
  let context: AudioContext | null = null;
  let analyser: AnalyserNode | null = null;
  let analyserData: Float32Array<ArrayBuffer> | null = null;
  let queuedUntil = 0;
  // In-flight enqueues (resume() kan vaere pending) skal synliggoeres for
  // drain() — ellers kan turn-end lukke sessionen FOER lyden er koeet.
  const pending = new Set<Promise<void>>();

  async function enqueueChunk(chunk: ArrayBuffer): Promise<void> {
    if (chunk.byteLength < 2) return;
    const perfTrace = PERF_ENABLED ? getActiveVoiceTrace() : null;
    const enqueueStarted = PERF_ENABLED ? performance.now() : 0;
    const cold = context === null;
    if (PERF_ENABLED) {
      markPerf(perfTrace, "voice.clip.enqueue.begin", {
        pcm_bytes: chunk.byteLength,
        audio_context_cold: cold,
      });
    }
    if (!context) {
      context = new AudioContext({ sampleRate: 24_000 });
      // Voice-orbens output-niveau tappes fra selve audio-grafen: chunks
      // skeduleres frem i tiden (source.start(startsAt)), saa RMS ved
      // enqueue-tid ville pulsere FOER lyden er hoerbar.
      analyser = context.createAnalyser();
      analyser.fftSize = 1_024;
      analyser.connect(context.destination);
      analyserData = new Float32Array(analyser.fftSize);
      if (PERF_ENABLED) {
        markPerf(perfTrace, "voice.clip.audio_context.created", {
          sample_rate: context.sampleRate,
          state: context.state,
        });
      }
    }
    if (context.state === "suspended") {
      const resumeStarted = PERF_ENABLED ? performance.now() : 0;
      await context.resume();
      if (PERF_ENABLED) {
        markPerf(perfTrace, "voice.clip.audio_context.resumed", {
          duration_ms: performance.now() - resumeStarted,
        });
      }
    }
    const conversionStarted = PERF_ENABLED ? performance.now() : 0;
    const samples = Math.floor(chunk.byteLength / 2);
    const audio = context.createBuffer(1, samples, 24_000);
    const channel = audio.getChannelData(0);
    const view = new DataView(chunk);
    for (let index = 0; index < samples; index += 1) {
      channel[index] = view.getInt16(index * 2, true) / 32_768;
    }
    const source = context.createBufferSource();
    source.buffer = audio;
    source.connect(analyser ?? context.destination);
    const startsAt = Math.max(context.currentTime, queuedUntil);
    source.start(startsAt);
    queuedUntil = startsAt + audio.duration;
    if (PERF_ENABLED && perfTrace) {
      const conversionMs = performance.now() - conversionStarted;
      const scheduledDelayMs =
        Math.max(0, startsAt - context.currentTime) * 1_000;
      const timingContext = context as AudioContext & {
        outputLatency?: number;
        getOutputTimestamp?: () => {
          contextTime: number;
          performanceTime: number;
        };
      };
      const outputTimestamp = timingContext.getOutputTimestamp?.();
      const outputPerformanceTime = outputTimestamp?.performanceTime;
      const outputContextTime = outputTimestamp?.contextTime;
      const hasOutputTimestamp =
        typeof outputPerformanceTime === "number" &&
        outputPerformanceTime > 0 &&
        typeof outputContextTime === "number";
      const estimatedOutputPerformanceMs = hasOutputTimestamp
        ? outputPerformanceTime + (startsAt - outputContextTime) * 1_000
        : performance.now() +
          scheduledDelayMs +
          (context.baseLatency + (timingContext.outputLatency ?? 0)) * 1_000;
      markPerf(perfTrace, "voice.clip.source_scheduled", {
        audio_context_cold: cold,
        conversion_ms: conversionMs,
        enqueue_ms: performance.now() - enqueueStarted,
        scheduled_delay_ms: scheduledDelayMs,
        base_latency_ms: context.baseLatency * 1_000,
        output_latency_ms: (timingContext.outputLatency ?? 0) * 1_000,
        estimated_output_performance_ms: estimatedOutputPerformanceMs,
        estimated_output_epoch_ms:
          performance.timeOrigin + estimatedOutputPerformanceMs,
        output_timestamp_supported: hasOutputTimestamp,
        duration_ms: audio.duration * 1_000,
      });
      // A new WebView2 AudioContext often reports 0/0 in source.start()'s
      // task. Retry after render-start to estimate the first hardware frame.
      const scheduleFrame = globalThis.requestAnimationFrame?.bind(globalThis);
      if (!scheduleFrame) return;
      const sampleHardwareOutputEstimate = (attempt: number) => {
        const sample = timingContext.getOutputTimestamp?.();
        if (
          typeof sample?.performanceTime === "number" &&
          sample.performanceTime > 0 &&
          typeof sample.contextTime === "number"
        ) {
          const outputPerformanceMs =
            sample.performanceTime +
            (startsAt - sample.contextTime) * 1_000;
          markPerf(perfTrace, "voice.clip.hardware_output_estimate", {
            attempt,
            estimated_output_performance_ms: outputPerformanceMs,
            estimated_output_epoch_ms:
              performance.timeOrigin + outputPerformanceMs,
            sampled_context_time: sample.contextTime,
            sampled_performance_time: sample.performanceTime,
            base_latency_ms: timingContext.baseLatency * 1_000,
            output_latency_ms: (timingContext.outputLatency ?? 0) * 1_000,
          });
          void flushPerfTrace(perfTrace);
          return;
        }
        if (attempt < 30) {
          scheduleFrame(() => sampleHardwareOutputEstimate(attempt + 1));
        }
      };
      scheduleFrame(() => sampleHardwareOutputEstimate(1));
    }
  }

  return {
    enqueue(chunk) {
      const task = enqueueChunk(chunk);
      pending.add(task);
      task.then(
        () => pending.delete(task),
        () => pending.delete(task),
      );
      return task;
    },
    async drain() {
      for (;;) {
        while (pending.size > 0) {
          await Promise.allSettled([...pending]);
        }
        if (!context || context.state === "closed") {
          queuedUntil = 0;
          return;
        }
        const remainingMs = Math.max(0, (queuedUntil - context.currentTime) * 1_000);
        if (remainingMs <= 0 && pending.size === 0) break;
        if (remainingMs > 0) {
          await new Promise<void>((resolve) => setTimeout(resolve, remainingMs + 16));
        }
      }
      queuedUntil = Math.max(queuedUntil, context.currentTime);
    },
    async close() {
      queuedUntil = 0;
      if (context && context.state !== "closed") await context.close();
      context = null;
      analyser = null;
      analyserData = null;
    },
    getOutputLevel() {
      if (
        !context ||
        context.state === "closed" ||
        !analyser ||
        !analyserData
      ) {
        return 0;
      }
      analyser.getFloatTimeDomainData(analyserData);
      let sumSquares = 0;
      for (let index = 0; index < analyserData.length; index += 1) {
        sumSquares += analyserData[index] * analyserData[index];
      }
      const level = normalizeLevel(Math.sqrt(sumSquares / analyserData.length));
      if (PERF_ENABLED && level > 0.001) {
        const trace = getActiveVoiceTrace();
        if (
          markPerfOnce(
            trace,
            "voice.output_detected",
            "voice.clip.graph_output_detected_proxy",
            {
              level,
              base_latency_ms: context.baseLatency * 1_000,
              output_latency_ms:
                ((context as AudioContext & { outputLatency?: number })
                  .outputLatency ?? 0) * 1_000,
            },
          )
        ) {
          void flushPerfTrace(trace);
        }
      }
      return level;
    },
  };
}
