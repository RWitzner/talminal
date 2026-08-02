import { invoke } from "@tauri-apps/api/core";
import {
  getActiveVoiceTrace,
  perfInvokeArgs,
} from "../perfTrace.ts";
import type { VoiceIntent } from "./intents.ts";

const MIN_CONFIDENCE = 0.75;
const MAX_COMMANDS = 10;
const ROUTER_TOOL_NAME = "route_voice_intent";

export type RouterTransport = (body: string) => Promise<string>;

const SYSTEM_PROMPT = `You are the deterministic voice-command router for Talminal, a Windows canvas of numbered agent cards (each card runs Claude Code or Codex CLI). You receive one Danish/English transcript of a settled utterance and must classify it into an ORDERED list of 1-10 commands via the function tool. Most utterances are exactly one command.

Commands:
- send_prompt: send work text to a card. card is an explicit spoken card number, else null. text is the message the card itself will read: remove routing phrases like "sig til kort tre at" / "send besked til terminal fire og", and REWRITE reported/indirect speech into the direct message ("spørg hvad status er" -> "Hvad er den nuværende status?"; "bed den om at rydde op" -> "Ryd op."). An imperative software-work request is already the direct message — keep it as-is ("Kør testene igen og fix det der fejler" -> send_prompt, card null). Preserve vague references in text; target resolution happens later.
- new_card: create one or more cards/terminals. count is required 1..10 ("Ny terminal" -> count 1; "Åbn 4 terminaler" -> count 4; "Åbn tre kort mere" -> count 3). agent is optional: "codex" only when the utterance literally says codex, "claude" only when it literally says claude; otherwise null (the app default applies). Other model/agent words (gpt, gemini, cursor) do NOT map to an agent — leave null. The canvas IS the project: if the utterance names a project, folder or directory for the new card ("i webshop-mappen", "in the webshop project"), that is a reject — never guess a location.
- close_cards: close explicitly numbered cards as one batch, or every open card when the utterance says all ("alle"/"all") -> cards:[] and all:true. A clear close command whose target is missing, unnumbered, or only DESCRIBED by a quality rather than a number ("luk to af dem", definite form like "luk det kort", or a described group like "luk codex-kortene"/"luk de gamle terminaler") -> cards:[] and no all; that is a valid command, NOT a reject — the resolver asks. Never invent numbers, and never treat a description as unresolvable yourself — that judgment belongs to the resolver.
- restart_card: restart one explicitly named card, else card null.
- open_browser: open a browser card for previews or web research ("åbn en browser", "åbn et browservindue"). url_hint only when a known site was spoken: "github" | "google"; otherwise null — never invent domains or accept free-form URLs.
- reject: no settled actionable command, self-corrected fragments or action chains ("genstart… nej, luk…"), conflicting actions or either/or targets ("kort N eller kort M"), bare confirmations/negations, or sleep/wake talk. ALSO reject, because these commands no longer exist: status QUESTIONS about a card without a routing verb ("hvad laver kort tre", "status på kort et", "what's the status on card five") — though an explicit routing phrase is still send_prompt ("spørg kort tre hvor langt den er" -> send_prompt); focus/switch/zoom commands ("gå til kort fem", "fokus på kort to", "zoom ind på kort tre"); and creates that name a project or folder. A self-corrected or conflicting utterance is a reject even when a close verb occurs in it — never downgrade it to close_cards with empty cards.

Chaining rules:
- The commands array preserves spoken order. Split ONLY on genuine boundaries between actions ("og"/"and"/"så"/sentence breaks): "Luk kort to og genstart kort tre" -> [close_cards, restart_card].
- "og" INSIDE one command never splits: "Luk kort to og tre" -> ONE close_cards with cards [2,3]; "Bed kort to om at køre testene og fixe fejlene" -> ONE send_prompt (the "og" joins work items inside text).
- ALL-OR-NOTHING: if ANY segment on its own would be a reject (unclear, self-corrected, conflicting, status question without routing verb, focus command, folder-naming create, sleep talk), return exactly ONE reject element — never a partial chain. Correction markers (nej/øh/altså/i stedet/glem det) always make the WHOLE utterance one reject.
- confidence is ONE number for the whole utterance; use a single reject element rather than guessing.

You are state-blind: never invent a card number or count you did not hear. Missing card on an otherwise clear singular command is NOT a reject: return the command with card null (for close_cards: cards []) so the deterministic resolver can ask. Danish number words et/en=1, to=2, tre=3, fire=4, fem=5 count only when they identify a card or a count.

Card vocabulary: a card may be named as "kort", "terminal", "agent" or "canvas" followed by its number. "Luk agent to" and "Send en prompt til canvas tre" mean exactly what "Luk kort to" and "Send en prompt til kort tre" mean — the word choice never changes the command, only the number identifies the card. This holds for send_prompt, close_cards and restart_card alike. TWO GUARDS: (1) in new_card, "agent" before a model word names the AGENT, not a card ("åbn et kort med agent codex" -> new_card agent codex); (2) "canvas" without a number is the surface itself, never every card — "luk canvas" is close_cards cards:[] so the resolver can ask, NEVER all:true. Only the literal words alle/all set all:true. (3) This vocabulary only ADDS names; it never narrows the rules above. A card that is DESCRIBED instead of numbered ("genstart codex-kortet", "luk de gamle terminaler", "luk agenten") is STILL a valid command with card null / cards:[] so the resolver can ask — never a reject.

Examples (arrays; one element unless the utterance chains):
"Luk kort et." -> [close_cards {"cards":[1]}]
"Luk agent to." -> [close_cards {"cards":[2]}]
"Luk canvas tre." -> [close_cards {"cards":[3]}]
"Luk canvas." -> [close_cards {"cards":[]}]
"Genstart agent fire." -> [restart_card {"card":4}]
"Send en prompt til agent et om at køre testene." -> [send_prompt {"card":1,"text":"Kør testene."}]
"Spørg canvas to hvor langt den er." -> [send_prompt {"card":2,"text":"Hvor langt er du?"}]
"Åbn et kort med agent codex." -> [new_card {"count":1,"agent":"codex"}]
"Luk kort to og kort tre." -> [close_cards {"cards":[2,3]}]
"Luk to af dem." -> [close_cards {"cards":[]}]
"Luk alle kort." -> [close_cards {"cards":[],"all":true}]
"Luk de gamle terminaler." -> [close_cards {"cards":[]}]
"Genstart kort et." -> [restart_card {"card":1}]
"Åbn tre kort mere." -> [new_card {"count":3}]
"Åben fire terminaler, to med claude og to med codex." -> [new_card {"count":2,"agent":"claude"}, new_card {"count":2,"agent":"codex"}]
"Nyt codex-kort." -> [new_card {"count":1,"agent":"codex"}]
"Åbn et kort med gpt." -> [new_card {"count":1}]
"Nyt kort i webshop-mappen." -> [reject]
"Åbn en browser." -> [open_browser {"url_hint":null}]
"Åbn en browser på GitHub." -> [open_browser {"url_hint":"github"}]
"Send besked til terminal fem og spørg hvor langt den er." -> [send_prompt {"card":5,"text":"Hvor langt er du?"}]
"Spørg kort tre hvor langt den er." -> [send_prompt {"card":3,"text":"Hvor langt er du?"}]
"Hvad laver kort tre lige nu?" -> [reject]
"Status på kort et." -> [reject]
"Gå til kort fem." -> [reject]
"Genstart… nej, luk… altså et af kortene." -> [reject]
"Luk kort to og genstart kort tre." -> [close_cards {"cards":[2]}, restart_card {"card":3}]
"Åbn en terminal og en browser." -> [new_card {"count":1}, open_browser {"url_hint":null}]
"Åbn tre terminaler og en browser på GitHub. Send en prompt til kort et om status." -> [new_card {"count":3}, open_browser {"url_hint":"github"}, send_prompt {"card":1,"text":"Hvad er status?"}]
"Luk kort to og hvad laver kort tre?" -> [reject]
"Luk kort to og spørg kort tre hvor langt den er." -> [close_cards {"cards":[2]}, send_prompt {"card":3,"text":"Hvor langt er du?"}]
"Genstart kort to… nej, luk kort tre." -> [reject]`;

const COMMAND_PARAMETERS = {
  type: "object",
  properties: {
    kind: {
      type: "string",
      enum: [
        "send_prompt",
        "new_card",
        "close_cards",
        "restart_card",
        "open_browser",
        "reject",
      ],
    },
    card: { anyOf: [{ type: "integer", minimum: 1 }, { type: "null" }] },
    text: { anyOf: [{ type: "string" }, { type: "null" }] },
    cards: {
      anyOf: [
        { type: "array", items: { type: "integer", minimum: 1 } },
        { type: "null" },
      ],
    },
    all: { anyOf: [{ type: "boolean" }, { type: "null" }] },
    count: {
      anyOf: [{ type: "integer", minimum: 1, maximum: 10 }, { type: "null" }],
    },
    url_hint: {
      anyOf: [{ type: "string", enum: ["github", "google"] }, { type: "null" }],
    },
    agent: {
      anyOf: [{ type: "string", enum: ["claude", "codex"] }, { type: "null" }],
    },
  },
  required: [
    "kind",
    "card",
    "text",
    "cards",
    "all",
    "count",
    "url_hint",
    "agent",
  ],
  additionalProperties: false,
} as const;

const TOOL_PARAMETERS = {
  type: "object",
  properties: {
    commands: {
      type: "array",
      minItems: 1,
      maxItems: 10,
      items: COMMAND_PARAMETERS,
    },
    confidence: { type: "number", minimum: 0, maximum: 1 },
    reason: { anyOf: [{ type: "string" }, { type: "null" }] },
  },
  required: ["commands", "confidence", "reason"],
  additionalProperties: false,
} as const;

const ROUTER_TOOL = {
  type: "function",
  function: {
    name: ROUTER_TOOL_NAME,
    description: "Classify one Talminal voice transcript into a validated intent or reject it.",
    strict: true,
    parameters: TOOL_PARAMETERS,
  },
} as const;

interface CommandElement {
  kind?: unknown;
  card?: unknown;
  text?: unknown;
  cards?: unknown;
  all?: unknown;
  count?: unknown;
  url_hint?: unknown;
  agent?: unknown;
}

interface ToolInput {
  commands?: unknown;
  confidence?: unknown;
}

interface ChatCompletionsResponse {
  choices?: Array<{
    message?: {
      tool_calls?: Array<{
        type?: string;
        function?: {
          name?: string;
          arguments?: string;
        };
      }>;
    };
  }>;
}

const defaultTransport: RouterTransport = (body) =>
  PERF_ENABLED
    ? invoke<string>("router_chat_completion", {
        body,
        ...perfInvokeArgs(getActiveVoiceTrace()),
      })
    : invoke<string>("router_chat_completion", { body });

function optionalCard(value: unknown): number | null {
  return Number.isInteger(value) && Number(value) > 0 ? Number(value) : null;
}

function validCount(value: unknown): value is number {
  return Number.isInteger(value) && Number(value) >= 1 && Number(value) <= 10;
}

function parseCommandElement(input: CommandElement): VoiceIntent | null {
  switch (input.kind) {
    case "send_prompt": {
      const text = typeof input.text === "string" ? input.text.trim() : "";
      return text
        ? { kind: "send_prompt", card: optionalCard(input.card), text }
        : null;
    }
    case "new_card": {
      if (!validCount(input.count)) return null;
      // agent skal vaere null/fravaerende, "claude" eller "codex" — andre
      // vaerdier (forkert TYPE eller ukendt streng som "gpt") forgifter hele
      // kaeden, ligesom et ugyldigt kind ville (samme alt-eller-intet-princip
      // som default-grenen nedenfor).
      //
      // BEVIDST ASYMMETRI (slut-review fix 3): ukendt-streng-poisonen er
      // med vilje strengere end soesterfeltet `url_hint`, som degraderer
      // TAVST til null nedenfor — en opfundet agent er en kortoprettelse
      // paa den FORKERTE agent, mens et opfundet url_hint kun er et
      // manglende hint. Ret ikke den ene til at ligne den anden.
      // `!=` (ikke `!==`) er ligeledes bevidst: skemaet er strict og kraever
      // feltet, men et modelsvar der udelader noeglen giver `undefined` —
      // den skal degradere til "ingen agent" (default_agent-vejen), ikke
      // draebe hele ytringen. Samme loese check som paa :203 nedenfor.
      const agent = input.agent;
      if (agent != null && agent !== "claude" && agent !== "codex") {
        return null;
      }
      return {
        kind: "new_card",
        count: input.count,
        ...(agent == null ? {} : { agent }),
      };
    }
    case "close_cards": {
      const cards = Array.isArray(input.cards)
        ? input.cards.filter(
            (candidate): candidate is number =>
              Number.isInteger(candidate) && Number(candidate) > 0,
          )
        : [];
      if (input.all === true && cards.length > 0) return null;
      return {
        kind: "close_cards",
        cards,
        ...(input.all === true ? { all: true } : {}),
      };
    }
    case "restart_card":
      return { kind: "restart_card", card: optionalCard(input.card) };
    case "open_browser": {
      const hint = input.url_hint;
      return {
        kind: "open_browser",
        url_hint: hint === "github" || hint === "google" ? hint : null,
      };
    }
    default:
      // Inkl. "reject": ét reject-element forgifter hele kæden
      // (alt-eller-intet, multi-spec §3).
      return null;
  }
}

function parseToolInput(input: ToolInput): VoiceIntent[] | null {
  if (
    typeof input.confidence !== "number" ||
    !Number.isFinite(input.confidence) ||
    input.confidence < MIN_CONFIDENCE ||
    input.confidence > 1
  ) {
    return null;
  }
  if (
    !Array.isArray(input.commands) ||
    input.commands.length < 1 ||
    input.commands.length > MAX_COMMANDS
  ) {
    return null;
  }
  const intents: VoiceIntent[] = [];
  let creationBudget = 0;
  for (const element of input.commands) {
    if (element === null || typeof element !== "object" || Array.isArray(element)) {
      return null;
    }
    const intent = parseCommandElement(element as CommandElement);
    if (intent === null) return null;
    if (intent.kind === "new_card") {
      creationBudget += intent.count ?? 1;
      // Værn mod 10×10-eksplosionen: samlet oprettelses-budget pr. ytring.
      if (creationBudget > 10) return null;
    }
    intents.push(intent);
  }
  return intents;
}

function parseToolArguments(argumentsText: unknown): VoiceIntent[] | null {
  if (typeof argumentsText !== "string") return null;

  try {
    const input = JSON.parse(argumentsText) as unknown;
    if (input === null || typeof input !== "object" || Array.isArray(input)) {
      return null;
    }
    return parseToolInput(input as ToolInput);
  } catch {
    return null;
  }
}

function parseResponse(responseText: string): VoiceIntent[] | null {
  let payload: ChatCompletionsResponse;
  try {
    payload = JSON.parse(responseText) as ChatCompletionsResponse;
  } catch {
    return null;
  }

  if (payload === null || typeof payload !== "object") return null;
  const toolCall = payload.choices?.[0]?.message?.tool_calls?.find(
    (call) =>
      call.type === "function" && call.function?.name === ROUTER_TOOL_NAME,
  );
  return parseToolArguments(toolCall?.function?.arguments);
}

export async function routeVoiceTranscript(
  transcript: string,
  options: { transport?: RouterTransport } = {},
): Promise<VoiceIntent[] | null> {
  const text = transcript.trim();
  if (!text) return null;

  const body = JSON.stringify({
    // Model og provider-dekoration injiceres Rust-side fra den valgte rute.
    max_completion_tokens: 1024,
    temperature: 0,
    messages: [
      { role: "system", content: SYSTEM_PROMPT },
      { role: "user", content: text },
    ],
    tools: [ROUTER_TOOL],
    tool_choice: {
      type: "function",
      function: { name: ROUTER_TOOL_NAME },
    },
    parallel_tool_calls: false,
  });
  const responseText = await (options.transport ?? defaultTransport)(body);
  return parseResponse(responseText);
}
