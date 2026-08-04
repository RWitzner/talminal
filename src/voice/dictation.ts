// Dikterings-turen: mikrofon -> STT -> ind i det fokuserede korts composer.
//
// Det er PTT-turen med de tre sidste led skaaret af. Hvor
// `createPipelineVoiceSession` sender transskriptet gennem router-modellen,
// dispatcher en intent og svarer med et lydklip, goer denne det modsatte af
// at fortolke: den afleverer ordene raat. Optage-primitiven er den SAMME
// (`createPtt`), saa mikrofon, resampling og STT-streamen har ét sted at fejle
// for begge veje.
//
// Ingen router betyder ogsaa ingen afvisning: alt hvad STT'en hoerer, lander.
// Det er hele pointen — det er brugeren selv der laeser korrektur, i kortets
// eget tekstfelt, foer der bliver trykket Enter.

import { asError } from "./errors";
import { createPtt, startBrowserCapture, type StartAudioCapture } from "./ptt";
import type { SttClient } from "./stt";

export type DictationUiState = "idle" | "listening" | "finalizing" | "inserting";

export interface DictationSession {
  press(): void;
  release(): void;
  cancel(): void;
  state(): DictationUiState;
}

export function createDictationSession(deps: {
  stt: () => SttClient;
  /** Hvor ordene lander. Kaster den, ryger fejlen videre til `onError`. */
  insert(text: string): Promise<void>;
  startCapture?: StartAudioCapture;
  onState?(state: DictationUiState): void;
  /** Partials undervejs og den endelige tekst — HUD'en viser den. */
  onTranscript?(text: string): void;
  /** Tur uden ord. Ikke en fejl: mikrofonen kan bare have hoert stilhed. */
  onEmpty?(): void;
  onError?(error: Error): void;
  /** TLS-opvarmning ved tryk, praecis som pipelinen gør det. */
  warm?(): void;
  maxUtteranceMs?: number;
}): DictationSession {
  const startCapture = deps.startCapture ?? startBrowserCapture;

  let currentState: DictationUiState = "idle";
  let generation = 0;
  let currentPtt: ReturnType<typeof createPtt> | null = null;
  let maxDurationTimer: ReturnType<typeof setTimeout> | null = null;

  function isCurrent(token: number): boolean {
    return token === generation;
  }

  function transition(next: DictationUiState) {
    if (currentState === next) return;
    currentState = next;
    deps.onState?.(next);
  }

  function clearMaxTimer() {
    if (maxDurationTimer === null) return;
    clearTimeout(maxDurationTimer);
    maxDurationTimer = null;
  }

  function cancelCurrent() {
    clearMaxTimer();
    // Bumpet FOER cancel(): en igangvaerende insert-promise der resolver
    // bagefter, skal ikke kunne skrive i kortet efter afbrydelsen.
    generation += 1;
    currentPtt?.cancel();
    currentPtt = null;
    transition("idle");
  }

  async function deliver(token: number, transcript: string) {
    if (!isCurrent(token)) return;
    const text = transcript.trim();
    deps.onTranscript?.(text);
    if (!isCurrent(token)) return;

    if (text === "") {
      deps.onEmpty?.();
      transition("idle");
      return;
    }

    transition("inserting");
    try {
      await deps.insert(text);
    } catch (error) {
      if (isCurrent(token)) deps.onError?.(asError(error));
    }
    if (isCurrent(token)) transition("idle");
  }

  function startTurn() {
    const token = ++generation;
    transition("listening");

    try {
      const sttClient = deps.stt();
      let partial = "";
      sttClient.onPartial((delta) => {
        if (!isCurrent(token) || currentState !== "listening") return;
        partial += delta;
        deps.onTranscript?.(partial);
      });
      currentPtt = createPtt({
        stt: sttClient,
        startCapture,
        onState: () => undefined,
        onFinal: (transcript) => {
          if (isCurrent(token)) void deliver(token, transcript);
        },
        onError: (error) => {
          if (!isCurrent(token)) return;
          deps.onError?.(error);
          transition("idle");
        },
      });
      currentPtt.press();
      if (deps.maxUtteranceMs !== undefined) {
        const limitMs = deps.maxUtteranceMs;
        maxDurationTimer = setTimeout(() => {
          deps.onError?.(
            new Error(
              `Dikteringen nåede grænsen på ${Math.round(limitMs / 1000)} sekunder`,
            ),
          );
          cancelCurrent();
        }, limitMs);
      }
    } catch (error) {
      if (isCurrent(token)) {
        deps.onError?.(asError(error));
        transition("idle");
      }
    }
  }

  return {
    press() {
      // Et tryk midt i en tur betyder "jeg vil sige noget andet": den gamle
      // kasseres frem for at to ture kappes om den samme composer.
      if (currentState !== "idle") cancelCurrent();
      deps.warm?.();
      startTurn();
    },

    release() {
      if (currentState !== "listening") return;
      clearMaxTimer();
      transition("finalizing");
      currentPtt?.release();
    },

    cancel() {
      cancelCurrent();
    },

    state() {
      return currentState;
    },
  };
}
