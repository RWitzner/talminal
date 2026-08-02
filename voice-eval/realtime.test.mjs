import { readFile } from "node:fs/promises";

import { describe, expect, it } from "vitest";
import {
  buildRealtimeEvalConfig,
  evaluateRealtimeDryRunTurn,
  evaluateRealtimeResult,
  normalizeRealtimeIntent,
  passesRealtimeGate,
} from "./realtime-config.mjs";
import {
  parseUtterances,
  REALTIME_SKIP_IDS,
} from "./harness.mjs";

const utterances = await readFile(new URL("./utterances.md", import.meta.url), "utf8");

describe("realtime voice eval contract", () => {
  it("bevarer den frosne u12-fixture og låser et eksplicit dansk few-shot", () => {
    expect(utterances).toContain('| u12 | A | "Genstart kort et."');
    expect(buildRealtimeEvalConfig().instructions).toContain(
      '"Genstart kort et" -> restart_card({"card":1})',
    );
  });

  it("låser negative ambiguity-few-shots og schema v3 uden confirm", () => {
    const instructions = buildRealtimeEvalConfig().instructions;
    expect(instructions).not.toContain("-> confirm(");
    expect(instructions).toContain("Explicit numbered destructive targets execute immediately");
    expect(instructions).toContain(
      '"Luk 2 af dem" -> close_cards({"cards":[]})',
    );
  });

  it("bruger text-mode over schema v3 uden at ændre router-runneren", () => {
    const config = buildRealtimeEvalConfig();
    expect(config.output_modalities).toEqual(["text"]);
    expect(config.tools.map((tool) => tool.name)).toEqual([
      "send_prompt",
      "new_card",
      "close_cards",
      "open_browser",
      "restart_card",
    ]);
    expect(
      config.tools.find((tool) => tool.name === "new_card").parameters.required,
    ).toEqual(["count"]);
  });

  it("preserves the schema-v3 close_cards name and batch targets", () => {
    expect(
      normalizeRealtimeIntent({ name: "close_cards", arguments: { cards: [3] } }),
    ).toEqual({ kind: "close_cards", cards: [3] });
    expect(
      normalizeRealtimeIntent({ name: "close_cards", arguments: { cards: [2, 3] } }),
    ).toEqual({ kind: "close_cards", cards: [2, 3] });
  });

  it("genbruger guard + resolver og producerer capture-bevis med action_count=0", async () => {
    await expect(
      evaluateRealtimeDryRunTurn({
        transcript: "Genstart kort et",
        call: { name: "restart_card", arguments: { card: 1 } },
        focusedCard: 2,
        latencyMs: 321,
        ts: "2026-07-18T12:00:00Z",
      }),
    ).resolves.toMatchObject({
      transcript: "Genstart kort et",
      tool: { name: "restart_card", arguments: { card: 1 } },
      resolver: { ok: true, card: 1, dry_run: true, action_count: 0 },
      latency_ms: 321,
      action_count: 0,
    });

    await expect(
      evaluateRealtimeDryRunTurn({
        transcript: "Luk kort tre",
        call: { name: "restart_card", arguments: { card: 3 } },
        focusedCard: null,
        latencyMs: 100,
      }),
    ).resolves.toMatchObject({
      resolver: { ok: false, code: "action_conflict", action_count: 0 },
      action_count: 0,
    });
  });

  it("keeps the same schema-v3 call through dry-run and evaluation", async () => {
    const evaluated = await evaluateRealtimeResult({
      row: { context: "A", utterance: "Luk kort tre" },
      result: {
        toolCalls: [
          {
            name: "close_cards",
            arguments: { cards: [3] },
            rawArguments: '{"cards":[3]}',
          },
        ],
        latencyMs: 222,
      },
      ts: "2026-07-18T12:00:00Z",
    });

    expect(evaluated.result.toolCalls).toEqual([
      expect.objectContaining({
        name: "close_cards",
        arguments: { cards: [3] },
        rawArguments: '{"cards":[3]}',
      }),
    ]);
    expect(evaluated.capture).toMatchObject({
      transcript: "Luk kort tre",
      tool: { name: "close_cards", arguments: { cards: [3] } },
      resolver: {
        ok: true,
        kind: "close_cards",
        cards: [3],
        action_count: 0,
      },
      latency_ms: 222,
      action_count: 0,
    });
  });

  it("logger et no-tool svar uden at fabrikere en handling", async () => {
    const evaluated = await evaluateRealtimeResult({
      row: { context: "A", utterance: "Øhm, vent" },
      result: { toolCalls: [], latencyMs: 40 },
    });

    expect(evaluated.result.toolCalls).toEqual([]);
    expect(evaluated.capture).toMatchObject({
      tool: { name: "", arguments: {} },
      resolver: { ok: false, code: "no_tool_call", action_count: 0 },
      action_count: 0,
    });
  });

  it("classifies from Markdown sections rather than utterance IDs", () => {
    const rows = parseUtterances(`### Gyldige kommandoer (uanset ID)
| ID | Ktx | Utterance | Sprog | Forventet intent | Mål-kort |
|---|---|---|---|---|---|
| u30 | A | "Status på kort et." | da | \`query_status { card: 1 }\` | 1 |

### Bevidst tvetydige — SKAL give HUD-fejl
| ID | Ktx | Utterance | Sprog | Forventet resultat | Forventet reason |
|---|---|---|---|---|---|
| u01 | B | "Luk kortet." | da | HUD-fejl | \`no_target\` |`);

    expect(rows.map(({ id, classification }) => ({ id, classification }))).toEqual([
      { id: "u30", classification: "valid" },
      { id: "u01", classification: "ambiguous" },
    ]);
  });

  it("keeps 17 valid + 17 ambiguous single-intent rows after the chain filter", () => {
    // Kæde-rækkerne (multi-spec §10.2) er pipeline-only og filtreres fra i
    // realtime-sporet — tallene gælder EFTER REALTIME_SKIP_IDS-filteret.
    const rows = parseUtterances(utterances).filter(
      (row) => !REALTIME_SKIP_IDS.has(row.id),
    );
    expect(rows.filter((row) => row.classification === "valid")).toHaveLength(17);
    expect(rows.filter((row) => row.classification === "ambiguous")).toHaveLength(17);
  });

  it("keeps every valid fixture after the chain filter inside the 5-tool schema", () => {
    const toolNames = new Set(
      buildRealtimeEvalConfig().tools.map((tool) => tool.name),
    );
    const unsupportedFixtures = parseUtterances(utterances)
      .filter((row) => !REALTIME_SKIP_IDS.has(row.id))
      .filter((row) => row.classification === "valid")
      .filter((row) => !toolNames.has(row.expectedKind))
      .map(({ id, expectedKind }) => ({ id, expectedKind }));

    expect(unsupportedFixtures).toEqual([]);
  });

  it("resolves creates to the project root in dry-run captures", async () => {
    for (const [transcript, count] of [
      ["Åbn 4 terminaler", 4],
      ["Ny terminal", 1],
      ["Åbn tre kort mere", 3],
    ]) {
      const capture = await evaluateRealtimeDryRunTurn({
        transcript,
        call: { name: "new_card", arguments: { count } },
        focusedCard: null,
        latencyMs: 10,
      });
      expect(capture).toMatchObject({
        tool: { name: "new_card", arguments: { count } },
        resolver: {
          ok: true,
          kind: "new_card",
          cwd: "C:\\projekter\\demo",
          action_count: 0,
        },
        action_count: 0,
      });
      expect(capture.tool.arguments).not.toHaveProperty("cwd_hint");
    }
  });

  it("enforces intent ≥90 %, target ≥95 %, and 17/17 ambiguity", () => {
    expect(
      passesRealtimeGate({
        intentAccuracy: 0.9,
        targetAccuracy: 0.95,
        ambiguousRejected: 17,
        ambiguousTotal: 17,
      }),
    ).toBe(true);
    expect(
      passesRealtimeGate({
        intentAccuracy: 0.89,
        targetAccuracy: 1,
        ambiguousRejected: 17,
        ambiguousTotal: 17,
      }),
    ).toBe(false);
    expect(
      passesRealtimeGate({
        intentAccuracy: 1,
        targetAccuracy: 0.94,
        ambiguousRejected: 17,
        ambiguousTotal: 17,
      }),
    ).toBe(false);
    expect(
      passesRealtimeGate({
        intentAccuracy: 1,
        targetAccuracy: 1,
        ambiguousRejected: 16,
        ambiguousTotal: 17,
      }),
    ).toBe(false);
    expect(
      passesRealtimeGate({
        intentAccuracy: 1,
        targetAccuracy: 1,
        ambiguousRejected: 17,
        ambiguousTotal: 18,
      }),
    ).toBe(false);
  });
});
