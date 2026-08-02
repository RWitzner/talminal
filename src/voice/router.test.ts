import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { routeVoiceTranscript, type RouterTransport } from "./router";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

const ROUTER_TOOL = "route_voice_intent";
const REQUIRED_FIELDS = ["commands", "confidence", "reason"];
const REQUIRED_COMMAND_FIELDS = [
  "kind",
  "card",
  "text",
  "cards",
  "all",
  "count",
  "url_hint",
  "agent",
];

function toolResponse(argumentsValue: unknown): string {
  const argumentsText =
    typeof argumentsValue === "string"
      ? argumentsValue
      : JSON.stringify(argumentsValue);
  return JSON.stringify({
    choices: [
      {
        message: {
          tool_calls: [
            {
              type: "function",
              function: {
                name: ROUTER_TOOL,
                arguments: argumentsText,
              },
            },
          ],
        },
      },
    ],
  });
}

function commandElement(overrides: Record<string, unknown> = {}) {
  return {
    kind: "reject",
    card: null,
    text: null,
    cards: null,
    all: null,
    count: null,
    url_hint: null,
    agent: null,
    ...overrides,
  };
}

function routerInput(
  commands: Array<Record<string, unknown>>,
  overrides: Record<string, unknown> = {},
) {
  return {
    commands,
    confidence: 0.97,
    reason: null,
    ...overrides,
  };
}

function fakeTransport(argumentsValue: unknown): RouterTransport {
  return vi.fn(async () => toolResponse(argumentsValue));
}

describe("routeVoiceTranscript", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new Error("Router tests must not call the network");
      }),
    );
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("parser én kommando som array med ét element", async () => {
    const intents = await routeVoiceTranscript("Åbn tre kort mere.", {
      transport: fakeTransport(
        routerInput(
          [commandElement({ kind: "new_card", count: 3 })],
          { confidence: 0.97 },
        ),
      ),
    });
    expect(intents).toEqual([{ kind: "new_card", count: 3 }]);
  });

  it("parser en 2-kæde i talt rækkefølge", async () => {
    const intents = await routeVoiceTranscript("Luk kort to og genstart kort tre.", {
      transport: fakeTransport(
        routerInput(
          [
            commandElement({ kind: "close_cards", cards: [2] }),
            commandElement({ kind: "restart_card", card: 3 }),
          ],
          { confidence: 0.96 },
        ),
      ),
    });
    expect(intents).toEqual([
      { kind: "close_cards", cards: [2] },
      { kind: "restart_card", card: 3 },
    ]);
  });

  it("ét reject-element forgifter hele kæden (alt-eller-intet)", async () => {
    const intents = await routeVoiceTranscript("Luk kort to og hvad laver kort tre?", {
      transport: fakeTransport(
        routerInput(
          [
            commandElement({ kind: "close_cards", cards: [2] }),
            commandElement({ kind: "reject" }),
          ],
          { confidence: 0.9, reason: "status question" },
        ),
      ),
    });
    expect(intents).toBeNull();
  });

  it("samlet oprettelses-budget > 10 afviser hele kæden", async () => {
    const intents = await routeVoiceTranscript("Åbn ti kort og åbn tre mere.", {
      transport: fakeTransport(
        routerInput(
          [
            commandElement({ kind: "new_card", count: 10 }),
            commandElement({ kind: "new_card", count: 3 }),
          ],
          { confidence: 0.95 },
        ),
      ),
    });
    expect(intents).toBeNull();
  });

  it("fjernede kinds er ugyldige elementer", async () => {
    const intents = await routeVoiceTranscript("Hvad laver kort tre?", {
      transport: fakeTransport(
        routerInput(
          [commandElement({ kind: "query_status", card: 3 })],
          { confidence: 0.95 },
        ),
      ),
    });
    expect(intents).toBeNull();
  });

  it("routes_close_cards_batch_with_all_flag", async () => {
    await expect(
      routeVoiceTranscript("Luk kort to og kort tre", {
        transport: fakeTransport(
          routerInput([
            commandElement({ kind: "close_cards", cards: [2, 0, 3, 1.5] }),
          ]),
        ),
      }),
    ).resolves.toEqual([{ kind: "close_cards", cards: [2, 3] }]);

    await expect(
      routeVoiceTranscript("Luk alle kort", {
        transport: fakeTransport(
          routerInput([
            commandElement({ kind: "close_cards", cards: [], all: true }),
          ]),
        ),
      }),
    ).resolves.toEqual([{ kind: "close_cards", cards: [], all: true }]);
  });

  it("close_cards_all_with_numbers_is_parse_reject", async () => {
    await expect(
      routeVoiceTranscript("Luk alle kort og kort to", {
        transport: fakeTransport(
          routerInput([
            commandElement({ kind: "close_cards", cards: [2], all: true }),
          ]),
        ),
      }),
    ).resolves.toBeNull();
  });

  it("new_card_requires_count", async () => {
    await expect(
      routeVoiceTranscript("Åbn en terminal", {
        transport: fakeTransport(
          routerInput([commandElement({ kind: "new_card", count: null })]),
        ),
      }),
    ).resolves.toBeNull();
  });

  it("parses new_card with agent codex", async () => {
    await expect(
      routeVoiceTranscript("Nyt codex-kort.", {
        transport: fakeTransport(
          routerInput([
            commandElement({ kind: "new_card", count: 2, agent: "codex" }),
          ]),
        ),
      }),
    ).resolves.toEqual([{ kind: "new_card", count: 2, agent: "codex" }]);
  });

  it("tolererer en helt FRAVAERENDE agent-noegle", async () => {
    // strict-skemaet kraever `agent`, men et modelsvar der udelader noeglen
    // giver `undefined` — ikke `null`. Den maa degradere til "ingen agent"
    // (default_agent-vejen), ikke forgifte hele kaeden (slut-review fix 3).
    await expect(
      routeVoiceTranscript("Nyt kort.", {
        transport: fakeTransport(
          routerInput([
            {
              kind: "new_card",
              card: null,
              text: null,
              cards: null,
              all: null,
              count: 1,
              url_hint: null,
            },
          ]),
        ),
      }),
    ).resolves.toEqual([{ kind: "new_card", count: 1 }]);
  });

  it("keeps all-or-nothing on malformed agent TYPE", async () => {
    // type-brud (tal i stedet for streng/null) -> hele kæden null
    // (router.ts:198-202-princippet: ét ugyldigt element forgifter alt).
    await expect(
      routeVoiceTranscript("Nyt codex-kort.", {
        transport: fakeTransport(
          routerInput([
            commandElement({ kind: "new_card", count: 1, agent: 7 }),
          ]),
        ),
      }),
    ).resolves.toBeNull();
  });

  it("send_prompt_strips_routing_phrase", async () => {
    await expect(
      routeVoiceTranscript("Sig til kort tre at køre testene", {
        transport: fakeTransport(
          routerInput([
            commandElement({ kind: "send_prompt", card: 3, text: " Kør testene " }),
          ]),
        ),
      }),
    ).resolves.toEqual([
      {
        kind: "send_prompt",
        card: 3,
        text: "Kør testene",
      },
    ]);
  });

  it("parser_open_browser_med_url_hint", async () => {
    await expect(
      routeVoiceTranscript("Åbn en browser på GitHub.", {
        transport: fakeTransport(
          routerInput([
            commandElement({ kind: "open_browser", url_hint: "github" }),
          ]),
        ),
      }),
    ).resolves.toEqual([{ kind: "open_browser", url_hint: "github" }]);
  });

  it("open_browser_uden_kendt_site_giver_null_url_hint", async () => {
    await expect(
      routeVoiceTranscript("Åbn en browser.", {
        transport: fakeTransport(
          routerInput([
            commandElement({ kind: "open_browser", url_hint: null }),
          ]),
        ),
      }),
    ).resolves.toEqual([{ kind: "open_browser", url_hint: null }]);

    await expect(
      routeVoiceTranscript("Åbn en browser på et sted jeg selv finder på.", {
        transport: fakeTransport(
          routerInput([
            commandElement({
              kind: "open_browser",
              url_hint: "https://evil.example.com",
            }),
          ]),
        ),
      }),
    ).resolves.toEqual([{ kind: "open_browser", url_hint: null }]);
  });

  it("low_confidence_returns_null", async () => {
    await expect(
      routeVoiceTranscript("Genstart kort to", {
        transport: fakeTransport(
          routerInput(
            [commandElement({ kind: "restart_card", card: 2 })],
            { confidence: 0.74 },
          ),
        ),
      }),
    ).resolves.toBeNull();
  });

  it("reject_kind_returns_null", async () => {
    await expect(
      routeVoiceTranscript("Nej, glem det", {
        transport: fakeTransport(
          routerInput([commandElement()], { reason: "bare negation" }),
        ),
      }),
    ).resolves.toBeNull();
  });

  it("unknown_kind_returns_null", async () => {
    for (const kind of [
      "close_card",
      "focus_card",
      "query_status",
      "go_to_sleep",
      "new_card_in_project",
      "confirm",
      "cancel",
      "other",
    ]) {
      await expect(
        routeVoiceTranscript(kind, {
          transport: fakeTransport(routerInput([commandElement({ kind })])),
        }),
      ).resolves.toBeNull();
    }
  });

  it("tom_eller_for_lang_kaede_returnerer_null", async () => {
    await expect(
      routeVoiceTranscript("Luk kort to", {
        transport: fakeTransport(routerInput([])),
      }),
    ).resolves.toBeNull();

    await expect(
      routeVoiceTranscript("Genstart alting mange gange", {
        transport: fakeTransport(
          routerInput(
            Array.from({ length: 11 }, () =>
              commandElement({ kind: "restart_card", card: 1 }),
            ),
          ),
        ),
      }),
    ).resolves.toBeNull();
  });

  it("default_transport_invokes_router_chat_completion", async () => {
    invokeMock.mockResolvedValue(
      toolResponse(
        routerInput([commandElement({ kind: "restart_card", card: 4 })]),
      ),
    );

    await expect(routeVoiceTranscript("Genstart kort fire")).resolves.toEqual([
      { kind: "restart_card", card: 4 },
    ]);

    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(invokeMock).toHaveBeenCalledWith("router_chat_completion", {
      body: expect.any(String),
    });
    const body = JSON.parse(invokeMock.mock.calls[0][1].body);
    expect(body).not.toHaveProperty("model");
    expect(body).not.toHaveProperty("providerOptions");
    expect(body.max_completion_tokens).toBe(1024);
    expect(body.messages[1]).toEqual({
      role: "user",
      content: "Genstart kort fire",
    });
    expect(body.messages[0].content).toContain(
      "count only when they identify a card or a count",
    );
    expect(body.messages[0].content).toContain('"Åbn tre kort mere" -> count 3');
    expect(body.messages[0].content).toContain(
      "never invent domains or accept free-form URLs",
    );
    expect(body.messages[0].content).toContain(
      '"Åbn en browser på GitHub." -> [open_browser {"url_hint":"github"}]',
    );
    expect(body.messages[0].content).toContain(
      '"Åbn en browser." -> [open_browser {"url_hint":null}]',
    );
    expect(body.messages[0].content).toContain(
      '"Luk kort to og genstart kort tre." -> [close_cards {"cards":[2]}, restart_card {"card":3}]',
    );
    expect(body.messages[0].content).toContain("ALL-OR-NOTHING");
    expect(body.messages[0].content).toContain(
      "each card runs Claude Code or Codex CLI",
    );
    expect(body.messages[0].content).toContain(
      'agent is optional: "codex" only when the utterance literally says codex',
    );
    expect(body.messages[0].content).toContain(
      '"Nyt codex-kort." -> [new_card {"count":1,"agent":"codex"}]',
    );
    // Alias-ordforraadet: et kort kan hedde kort/terminal/agent/canvas + nummer.
    // Reglen kan kun verificeres LIVE (mod en rigtig model), saa det eneste en
    // offline-test kan gøre er at forhindre at den forsvinder ud af prompten
    // igen — inkl. de to guards, som er der hvor aliasserne kan goere skade.
    expect(body.messages[0].content).toContain(
      'a card may be named as "kort", "terminal", "agent" or "canvas" followed by its number',
    );
    expect(body.messages[0].content).toContain(
      '"agent" before a model word names the AGENT, not a card',
    );
    expect(body.messages[0].content).toContain(
      '"canvas" without a number is the surface itself, never every card',
    );
    expect(body.messages[0].content).toContain(
      '"Luk agent to." -> [close_cards {"cards":[2]}]',
    );
    expect(body.messages[0].content).toContain(
      '"Luk canvas." -> [close_cards {"cards":[]}]',
    );

    const tool = body.tools[0].function;
    expect(tool).toMatchObject({
      name: ROUTER_TOOL,
      strict: true,
      parameters: {
        type: "object",
        additionalProperties: false,
        required: REQUIRED_FIELDS,
      },
    });
    expect(tool.parameters.properties.commands).toMatchObject({
      type: "array",
      minItems: 1,
      maxItems: 10,
    });
    const command = tool.parameters.properties.commands.items;
    expect(command).toMatchObject({
      type: "object",
      additionalProperties: false,
      required: REQUIRED_COMMAND_FIELDS,
    });
    expect(command.properties.kind.enum).toEqual([
      "send_prompt",
      "new_card",
      "close_cards",
      "restart_card",
      "open_browser",
      "reject",
    ]);
    expect(command.properties.count).toEqual({
      anyOf: [
        { type: "integer", minimum: 1, maximum: 10 },
        { type: "null" },
      ],
    });
    expect(command.properties.url_hint).toEqual({
      anyOf: [
        { type: "string", enum: ["github", "google"] },
        { type: "null" },
      ],
    });
    expect(command.properties.agent).toEqual({
      anyOf: [
        { type: "string", enum: ["claude", "codex"] },
        { type: "null" },
      ],
    });
  });

  it("transport_error_propagates", async () => {
    const failure = new Error("Rust proxy failed");
    const transport: RouterTransport = vi.fn(async () => {
      throw failure;
    });

    await expect(
      routeVoiceTranscript("Kør testene", { transport }),
    ).rejects.toBe(failure);
  });
});
