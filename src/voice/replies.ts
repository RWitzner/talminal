import type { DispatchResult } from "./dispatch";
import type { DryRunResult } from "./dryRun";
import type { VoiceIntent } from "./intents";

export type ActionOrigin = "voice" | "typed" | "system";

export interface Reply {
  text: string;
  audioKey: string | null;
}

export type TurnOutcome =
  | { kind: "dispatched"; intent: VoiceIntent; result: DispatchResult }
  | { kind: "dispatched_chain"; intents: VoiceIntent[]; results: DispatchResult[] }
  | { kind: "dry_run"; intent: VoiceIntent; result: DryRunResult }
  | { kind: "dry_run_chain"; intents: VoiceIntent[]; results: DryRunResult[] }
  | { kind: "router_reject" }
  | { kind: "stt_empty" }
  | { kind: "stt_error"; message: string };

export type ReplySource = (outcome: TurnOutcome) => Reply;

// Manuskript v2 (v4-spec §4 + multi-spec §6, LÅST af ejer): svaret ekkoer
// HANDLINGEN, skærmen viser MÅLET — talte svar bærer aldrig numre; HUD'en
// beholder detaljerne. Kæder ≥ 2 taler "Udført" (alle ok) eller det
// generiske fejl-klip. Nøgler = audioKey/filnavne 1:1 (slug af teksten —
// testen håndhæver parret).
export const CLIP_TEXTS = {
  sendt: "Sendt",
  "kort-aabnet": "Kort åbnet",
  "kort-lukket": "Kort lukket",
  "alle-lukket": "Alle lukket",
  "kort-genstartet": "Kort genstartet",
  "browser-aabnet": "Browser åbnet",
  udfoert: "Udført",
  "det-fangede-jeg-ikke": "Det fangede jeg ikke",
  "ingen-lyd-fanget-proev-igen": "Ingen lyd fanget — prøv igen",
  "kortet-findes-ikke": "Kortet findes ikke",
  "sig-det-med-et-kortnummer": "Sig det med et kortnummer",
  "ingen-kort-aabne": "Ingen kort åbne",
  "det-kort-er-en-browser": "Det kort er en browser",
  "noget-gik-galt-se-skaermen": "Noget gik galt — se skærmen",
} as const;

export type ClipKey = keyof typeof CLIP_TEXTS;

function clipReply(key: ClipKey): Reply {
  return { text: CLIP_TEXTS[key], audioKey: key };
}

const SUCCESS_CLIPS: Record<VoiceIntent["kind"], ClipKey> = {
  send_prompt: "sendt",
  new_card: "kort-aabnet",
  close_cards: "kort-lukket",
  restart_card: "kort-genstartet",
  open_browser: "browser-aabnet",
};

function failureClip(code: string): ClipKey {
  switch (code) {
    case "no_such_card":
      return "kortet-findes-ikke";
    case "no_target":
    case "ambiguous_focus":
      return "sig-det-med-et-kortnummer";
    case "no_cards":
      return "ingen-kort-aabne";
    case "browser_card":
      return "det-kort-er-en-browser";
    case "router_reject":
      return "det-fangede-jeg-ikke";
    default:
      // dispatch_exception/dry_run_exception/invalid_count/ukendte koder:
      // detaljerne bor i HUD'en, talen er den generiske fejl (v4-spec §6.2).
      return "noget-gik-galt-se-skaermen";
  }
}

export const templateReplySource: ReplySource = (outcome) => {
  switch (outcome.kind) {
    case "dispatched":
      if (!outcome.result.ok) return clipReply(failureClip(outcome.result.code));
      return outcome.intent.kind === "close_cards" && outcome.intent.all === true
        ? clipReply("alle-lukket")
        : clipReply(SUCCESS_CLIPS[outcome.intent.kind]);
    case "dispatched_chain":
      // Multi-spec §6: alle ok → "Udført"; mindst én fejl → generisk fejl.
      // HUD'ens kæde-resultatliste skelner pr. kommando (Task 7).
      return outcome.results.every((result) => result.ok)
        ? clipReply("udfoert")
        : clipReply("noget-gik-galt-se-skaermen");
    case "dry_run":
      // Dry-run taler ikke (v4-spec §5): HUD-teksten er svaret, ingen audio.
      return { text: outcome.result.message, audioKey: null };
    case "dry_run_chain":
      return {
        text: `Dry-run: kæde med ${outcome.intents.length} kommandoer resolveret; ingen handling udført`,
        audioKey: null,
      };
    case "router_reject":
      return clipReply("det-fangede-jeg-ikke");
    case "stt_empty":
      return clipReply("ingen-lyd-fanget-proev-igen");
    case "stt_error":
      return clipReply("noget-gik-galt-se-skaermen");
  }
};
