import type { StartAudioCapture } from "./ptt";

/**
 * Audio-niveau-matematik for voice-orben (spec 2026-07-20 §5).
 *
 * Normaliseringskurven er porteret verbatim fra Redaptings voice_recorder.rs
 * (støjgulv/loft + sqrt), så orbens følelse matcher companion-forlægget:
 * RMS under gulvet er stilhed (0), RMS over loftet er fuldt udslag (1), og
 * sqrt-kurven lægger normal tale (RMS ~0,05-0,20) i midterbåndet.
 */
export const LEVEL_NOISE_FLOOR_RMS = 0.015;
export const LEVEL_SPEECH_CEILING_RMS = 0.3;

/** RMS af lille-endian PCM16 (samme i16::MAX-normalisering som Rust). */
export function rmsOfPcm16(chunk: ArrayBuffer): number {
  const samples = Math.floor(chunk.byteLength / 2);
  if (samples === 0) return 0;
  const view = new DataView(chunk);
  let sumSquares = 0;
  for (let index = 0; index < samples; index += 1) {
    const normalized = view.getInt16(index * 2, true) / 32_767;
    sumSquares += normalized * normalized;
  }
  return Math.sqrt(sumSquares / samples);
}

export function normalizeLevel(rms: number): number {
  const span = LEVEL_SPEECH_CEILING_RMS - LEVEL_NOISE_FLOOR_RMS;
  const linear = Math.min(
    1,
    Math.max(0, (rms - LEVEL_NOISE_FLOOR_RMS) / span),
  );
  return Math.sqrt(linear);
}

/**
 * Decorator om capture-sømmen: chunks der alligevel flyder (24 kHz PCM16 fra
 * startBrowserCapture) producerer et niveau som ren sideeffekt. Chunken
 * videresendes byte-uændret — metering må aldrig ændre STT-vejen.
 */
export function withLevelMeter(
  start: StartAudioCapture,
  onLevel: (level: number) => void,
): StartAudioCapture {
  return (onChunk) =>
    start((chunk) => {
      onLevel(normalizeLevel(rmsOfPcm16(chunk)));
      onChunk(chunk);
    });
}
