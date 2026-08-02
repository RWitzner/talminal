import { Channel, invoke } from "@tauri-apps/api/core";
import { createPcmPlayer, type PcmPlayer } from "./audioPlayer";
import type { Reply } from "./replies";

export const TTS_MODEL = "gpt-4o-mini-tts";
export const TTS_VOICE = "nova";
export const TTS_INSTRUCTIONS =
  "Tal naturligt dansk, kort og neutralt, i roligt tempo. Udtal tal som danske talord.";

export interface TtsPlayback {
  firstAudio: Promise<void>;
  done: Promise<void>;
  stop(): void;
}

type TtsTransport = (
  body: string,
  onChunk: (base64Chunk: string) => void,
) => Promise<void>;

interface OpenAiTtsDependencies {
  transport?: TtsTransport;
  player?: PcmPlayer;
  assets?: Map<string, ArrayBuffer>;
}

function decodeBase64(value: string): ArrayBuffer {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes.buffer;
}

function defaultTransport(
  body: string,
  onChunk: (base64Chunk: string) => void,
): Promise<void> {
  const channel = new Channel<string>();
  channel.onmessage = onChunk;
  return invoke<void>("tts_speech_stream", { body, onChunk: channel });
}

export function createOpenAiTts(
  deps: OpenAiTtsDependencies = {},
): { speak(reply: Reply): TtsPlayback } {
  const transport = deps.transport ?? defaultTransport;
  const player = deps.player ?? createPcmPlayer();
  const assets = deps.assets ?? new Map<string, ArrayBuffer>();
  let playerReady: Promise<void> = Promise.resolve();

  return {
    speak(reply) {
      if (!reply.text.trim()) {
        return {
          firstAudio: Promise.resolve(),
          done: Promise.resolve(),
          stop() {},
        };
      }

      let stopped = false;
      let firstAudioSettled = false;
      let doneSettled = false;
      let resolveFirstAudio!: () => void;
      let rejectFirstAudio!: (reason?: unknown) => void;
      let resolveDone!: () => void;
      let rejectDone!: (reason?: unknown) => void;

      const firstAudio = new Promise<void>((resolve, reject) => {
        resolveFirstAudio = resolve;
        rejectFirstAudio = reject;
      });
      const done = new Promise<void>((resolve, reject) => {
        resolveDone = resolve;
        rejectDone = reject;
      });

      const settleFirstAudio = {
        resolve() {
          if (firstAudioSettled) return;
          firstAudioSettled = true;
          resolveFirstAudio();
        },
        reject(error: unknown) {
          if (firstAudioSettled) return;
          firstAudioSettled = true;
          rejectFirstAudio(error);
        },
      };
      const settleDone = {
        resolve() {
          if (doneSettled) return;
          doneSettled = true;
          resolveDone();
        },
        reject(error: unknown) {
          if (doneSettled) return;
          doneSettled = true;
          rejectDone(error);
        },
      };

      const playback: TtsPlayback = {
        firstAudio,
        done,
        stop() {
          if (stopped || doneSettled) return;
          stopped = true;
          settleFirstAudio.reject(new Error("TTS-afspilning blev stoppet"));
          playerReady = Promise.resolve()
            .then(() => player.close())
            .then(
              () => undefined,
              () => undefined,
            );
          void playerReady.then(settleDone.resolve);
        },
      };

      void (async () => {
        try {
          const cached =
            reply.audioKey === null ? undefined : assets.get(reply.audioKey);
          if (cached !== undefined) {
            if (stopped) return;
            await playerReady;
            if (stopped) return;
            await player.enqueue(cached);
            if (stopped) return;
            settleFirstAudio.resolve();
          } else {
            // Chunks skal koees i ankomstraekkefoelge selv om enqueue er
            // async — kaeden serialiserer, og firstAudio er foerste chunk
            // koeet (source.start), ikke stream-slut.
            let chain: Promise<void> = Promise.resolve();
            await transport(
              JSON.stringify({
                model: TTS_MODEL,
                voice: TTS_VOICE,
                input: reply.text,
                response_format: "pcm",
                instructions: TTS_INSTRUCTIONS,
              }),
              (base64Chunk) => {
                if (stopped) return;
                const audio = decodeBase64(base64Chunk);
                chain = chain.then(async () => {
                  if (stopped) return;
                  await playerReady;
                  if (stopped) return;
                  await player.enqueue(audio);
                  if (stopped) return;
                  settleFirstAudio.resolve();
                });
              },
            );
            await chain;
          }
          if (stopped) return;

          await player.drain();
          if (stopped) return;
          settleFirstAudio.resolve();
          settleDone.resolve();
        } catch (error) {
          if (stopped) return;
          settleFirstAudio.reject(error);
          settleDone.reject(error);
        }
      })();

      return playback;
    },
  };
}
