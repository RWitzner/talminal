// Klip-afspiller — pipeline-motorens ENESTE svar-stemme (v4-spec §5).
// audioKey → pre-genereret klip via PcmPlayer. Manglende klip eller
// audioKey null = stilhed (HUD-teksten er svaret) — ALDRIG TTS-fallback.
import { createPcmPlayer, type PcmPlayer } from "./audioPlayer";
import type { Reply } from "./replies";
import type { TtsPlayback } from "./tts";
import { getActiveVoiceTrace, markPerf } from "../perfTrace";

const SILENT_PLAYBACK: TtsPlayback = {
  firstAudio: Promise.resolve(),
  done: Promise.resolve(),
  stop() {},
};

export function createClipTts(deps: {
  assets: Map<string, ArrayBuffer>;
  player?: PcmPlayer;
}): { speak(reply: Reply): TtsPlayback } {
  const player = deps.player ?? createPcmPlayer();
  return {
    speak(reply) {
      const perfTrace = PERF_ENABLED ? getActiveVoiceTrace() : null;
      const clip =
        reply.audioKey === null ? undefined : deps.assets.get(reply.audioKey);
      if (clip === undefined) {
        if (PERF_ENABLED) {
          markPerf(perfTrace, "voice.clip.lookup", {
            audio_key: reply.audioKey,
            hit: false,
            asset_count: deps.assets.size,
          });
        }
        if (reply.audioKey !== null) console.warn("voice.clips.miss", reply.audioKey);
        return SILENT_PLAYBACK;
      }
      if (PERF_ENABLED) {
        markPerf(perfTrace, "voice.clip.lookup", {
          audio_key: reply.audioKey,
          hit: true,
          asset_count: deps.assets.size,
          pcm_bytes: clip.byteLength,
        });
      }

      let stopped = false;
      let resolveFirst!: () => void;
      let rejectFirst!: (reason?: unknown) => void;
      let resolveDone!: () => void;
      let rejectDone!: (reason?: unknown) => void;
      const firstAudio = new Promise<void>((resolve, reject) => {
        resolveFirst = resolve;
        rejectFirst = reject;
      });
      const done = new Promise<void>((resolve, reject) => {
        resolveDone = resolve;
        rejectDone = reject;
      });

      void (async () => {
        try {
          if (stopped) return;
          await player.enqueue(clip);
          if (stopped) return;
          resolveFirst();
          await player.drain();
          if (stopped) return;
          resolveDone();
        } catch (error) {
          if (stopped) return;
          rejectFirst(error);
          rejectDone(error);
        }
      })();

      return {
        firstAudio,
        done,
        stop() {
          if (stopped) return;
          stopped = true;
          rejectFirst(new Error("TTS-afspilning blev stoppet"));
          void Promise.resolve()
            .then(() => player.close())
            .then(
              () => resolveDone(),
              () => resolveDone(),
            );
        },
      };
    },
  };
}
