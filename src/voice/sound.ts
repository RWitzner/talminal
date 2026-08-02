import type { StartAudioCapture } from "./ptt";

/**
 * Subtil lydfeedback for push-to-talk — port af Redaptings sound.rs
 * (spec 2026-07-06, ejer-tunet "spot on"; Talminal-addendum i
 * voice-orb-spec'en 2026-07-20).
 *
 * Lydform: to diskrete bløde toner ("du-dum") a la Wispr Flow — IKKE et
 * frekvens-glid (lyder som vandboble) og IKKE en flad tone med fades
 * (lyder som et bip). Hver tone er mørk (grundtone + oktav under), med
 * rundt anslag og blød cosinus-udklinging. Fejl = samme lave tone to
 * gange ("eh-eh") — hverken stigende (start) eller faldende (stop), så
 * den kendes i blinde.
 *
 * Motor: Web Audio i webview'en (Redapting brugte rodio i Rust fordi
 * optagelsen boede dér; her bor hele voice-laget i webview'en og kører
 * kun ved fokus). Rust-forlæggets 120 ms stille hale er bevidst udeladt:
 * den var en rodio-teardown-guard — en Web Audio-buffer spilles altid
 * færdig. Afspilning må ALDRIG blokere eller vælte voice-flowet
 * (fire-and-forget, fejl sluges).
 */
export type VoiceSound = "start" | "stop" | "error";

export const SOUND_SAMPLE_RATE = 48_000;
/** Samlet loft — håndhævet af NOTE_AMPLITUDE + overlap, testlåst. */
export const BLIP_AMPLITUDE = 0.2;

const NOTE_MS = 120;
const NOTE_OFFSET_MS = 80;
const NOTE_ATTACK_MS = 15;

// Per-tone-amplitude holdes under totalen, så overlappet (tone 1's hale +
// tone 2's anslag) aldrig overskrider BLIP_AMPLITUDE.
const NOTE_AMPLITUDE = 0.14;
const FUNDAMENTAL_MIX = 0.65;
const SUB_OCTAVE_MIX = 0.35;

const FREQ_LOW_HZ = 392;
const FREQ_HIGH_HZ = 523;
const FREQ_ERROR_HZ = 311;

const TONE_PAIRS: Record<VoiceSound, [number, number]> = {
  start: [FREQ_LOW_HZ, FREQ_HIGH_HZ],
  stop: [FREQ_HIGH_HZ, FREQ_LOW_HZ],
  error: [FREQ_ERROR_HZ, FREQ_ERROR_HZ],
};

function msToSamples(ms: number): number {
  return Math.floor((SOUND_SAMPLE_RATE * ms) / 1_000);
}

/** Raised-cosine-anslag + raised-cosine-udklinging: blød, rund og klik-fri
 * i begge ender (ingen eksponentiel decay = ingen dryp-karakter). */
function noteEnvelope(index: number, length: number): number {
  const attack = msToSamples(NOTE_ATTACK_MS);
  if (index < attack) {
    const linear = (index + 1) / attack;
    return 0.5 - 0.5 * Math.cos(Math.PI * linear);
  }
  const linear = (length - index) / (length - attack);
  return 0.5 - 0.5 * Math.cos(Math.PI * linear);
}

function addNote(buffer: Float32Array, freqHz: number, start: number): void {
  const length = msToSamples(NOTE_MS);
  for (let index = 0; index < length; index += 1) {
    const slot = start + index;
    if (slot >= buffer.length) break;
    const t = index / SOUND_SAMPLE_RATE;
    const fundamental = Math.sin(2 * Math.PI * freqHz * t);
    const subOctave = Math.sin(2 * Math.PI * freqHz * 0.5 * t);
    const voice = fundamental * FUNDAMENTAL_MIX + subOctave * SUB_OCTAVE_MIX;
    buffer[slot] += voice * NOTE_AMPLITUDE * noteEnvelope(index, length);
  }
}

/** Renderer toneparret som mono-samples. Ren funktion — unit-testet. */
export function renderVoiceSound(sound: VoiceSound): Float32Array {
  const [firstHz, secondHz] = TONE_PAIRS[sound];
  const samples = new Float32Array(msToSamples(NOTE_OFFSET_MS + NOTE_MS));
  addNote(samples, firstHz, 0);
  addNote(samples, secondHz, msToSamples(NOTE_OFFSET_MS));
  return samples;
}

export interface SoundPlayer {
  play(sound: VoiceSound): void;
  close(): Promise<void>;
}

export function createSoundPlayer(): SoundPlayer {
  let context: AudioContext | null = null;
  const buffers = new Map<VoiceSound, AudioBuffer>();

  return {
    play(sound) {
      void (async () => {
        try {
          context ??= new AudioContext({ sampleRate: SOUND_SAMPLE_RATE });
          if (context.state === "suspended") await context.resume();
          let buffer = buffers.get(sound);
          if (!buffer) {
            const samples = renderVoiceSound(sound);
            buffer = context.createBuffer(
              1,
              samples.length,
              SOUND_SAMPLE_RATE,
            );
            buffer.getChannelData(0).set(samples);
            buffers.set(sound, buffer);
          }
          const source = context.createBufferSource();
          source.buffer = buffer;
          source.connect(context.destination);
          source.start();
        } catch (error) {
          console.debug("voice.sound.playback_failed", error);
        }
      })();
    },
    async close() {
      buffers.clear();
      const current = context;
      context = null;
      if (current && current.state !== "closed") {
        try {
          await current.close();
        } catch {
          // Best effort — lyd må aldrig vælte teardown.
        }
      }
    },
  };
}

/**
 * Decorator om capture-sømmen (Redaptings mikrofon-sandheds-princip):
 * start-blip når mikrofonen reelt er åben, stop-blip når den er lukket —
 * også når stop fejler (streamen ER droppet). Fejler selve åbningen,
 * spilles ingen start-lyd (fejl-turen bærer sin egen "eh-eh" via
 * error-chokepunktet). Begge motorer deler sømmen, så begge får lydene.
 */
export function withSoundFeedback(
  start: StartAudioCapture,
  play: (sound: VoiceSound) => void,
): StartAudioCapture {
  return async (onChunk) => {
    const capture = await start(onChunk);
    play("start");
    let stopped = false;
    return {
      async stop() {
        if (stopped) {
          return capture.stop();
        }
        stopped = true;
        try {
          await capture.stop();
        } finally {
          play("stop");
        }
      },
    };
  };
}
