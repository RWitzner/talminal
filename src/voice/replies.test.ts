import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { VoiceIntent } from "./intents";
import { CLIP_TEXTS, templateReplySource, type TurnOutcome } from "./replies";

const ok = (kind: string, extra: Record<string, unknown> = {}) =>
  ({ ok: true, kind, message: "HUD-detalje med numre", ...extra }) as never;
const fail = (code: string) =>
  ({ ok: false, code, message: "HUD-fejldetalje" }) as never;

describe("manuskript v2 — kvitteringer (spec §4, LÅST)", () => {
  const cases: Array<[TurnOutcome, string]> = [
    [{ kind: "dispatched", intent: { kind: "send_prompt", card: 2, text: "x" }, result: ok("send_prompt") }, "sendt"],
    [{ kind: "dispatched", intent: { kind: "new_card", count: 3 }, result: ok("new_card") }, "kort-aabnet"],
    [{ kind: "dispatched", intent: { kind: "close_cards", cards: [1, 4] }, result: ok("close_cards") }, "kort-lukket"],
    [{ kind: "dispatched", intent: { kind: "close_cards", cards: [], all: true }, result: ok("close_cards") }, "alle-lukket"],
    [{ kind: "dispatched", intent: { kind: "restart_card", card: 2 }, result: ok("restart_card") }, "kort-genstartet"],
    [{ kind: "dispatched", intent: { kind: "open_browser", url_hint: null }, result: ok("open_browser") }, "browser-aabnet"],
  ];
  for (const [outcome, expectedKey] of cases) {
    it(`${JSON.stringify(outcome.kind)} → ${expectedKey}`, () => {
      const reply = templateReplySource(outcome);
      expect(reply.audioKey).toBe(expectedKey);
      expect(reply.text).toBe(CLIP_TEXTS[expectedKey as keyof typeof CLIP_TEXTS]);
    });
  }
});

describe("kæder (multi-spec §6)", () => {
  const intents = [
    { kind: "new_card", count: 1 },
    { kind: "open_browser", url_hint: null },
  ] as never[];
  it("alle ok → Udført", () => {
    const reply = templateReplySource({
      kind: "dispatched_chain",
      intents,
      results: [ok("new_card"), ok("open_browser")] as never[],
    });
    expect(reply).toEqual({ text: "Udført", audioKey: "udfoert" });
  });
  it("mindst én fejl → generisk fejl-klip", () => {
    const reply = templateReplySource({
      kind: "dispatched_chain",
      intents,
      results: [fail("no_cards"), ok("open_browser")] as never[],
    });
    expect(reply.audioKey).toBe("noget-gik-galt-se-skaermen");
  });
  it("dry_run_chain taler ikke", () => {
    const reply = templateReplySource({
      kind: "dry_run_chain",
      intents,
      results: [
        { ok: true, kind: "new_card", message: "Dry-run", dry_run: true, action_count: 0 },
        { ok: true, kind: "open_browser", message: "Dry-run", dry_run: true, action_count: 0 },
      ] as never[],
    });
    expect(reply.audioKey).toBeNull();
  });
});

describe("manuskript v2 — fejl-mapping", () => {
  const intent = { kind: "restart_card", card: 2 } as const;
  const cases: Array<[TurnOutcome, string]> = [
    [{ kind: "dispatched", intent, result: fail("no_such_card") }, "kortet-findes-ikke"],
    [{ kind: "dispatched", intent, result: fail("no_target") }, "sig-det-med-et-kortnummer"],
    [{ kind: "dispatched", intent: { kind: "send_prompt", card: null, text: "x" }, result: fail("ambiguous_focus") }, "sig-det-med-et-kortnummer"],
    [{ kind: "dispatched", intent: { kind: "close_cards", cards: [], all: true }, result: fail("no_cards") }, "ingen-kort-aabne"],
    [{ kind: "dispatched", intent, result: fail("browser_card") }, "det-kort-er-en-browser"],
    [{ kind: "dispatched", intent, result: fail("dispatch_exception") }, "noget-gik-galt-se-skaermen"],
    [{ kind: "dispatched", intent, result: fail("helt_ukendt_kode") }, "noget-gik-galt-se-skaermen"],
    [{ kind: "router_reject" }, "det-fangede-jeg-ikke"],
    [{ kind: "stt_empty" }, "ingen-lyd-fanget-proev-igen"],
    [{ kind: "stt_error", message: "WS døde" }, "noget-gik-galt-se-skaermen"],
  ];
  for (const [outcome, expectedKey] of cases) {
    it(`→ ${expectedKey}`, () => {
      expect(templateReplySource(outcome).audioKey).toBe(expectedKey);
    });
  }
});

describe("totalitet (v4-spec §10.1 + multi-spec §6): ingen kodevej taler dynamisk tekst", () => {
  const INTENTS: readonly VoiceIntent[] = [
    { kind: "send_prompt", card: 1, text: "x" },
    { kind: "new_card", count: 1 },
    { kind: "close_cards", cards: [1] },
    { kind: "close_cards", cards: [], all: true },
    { kind: "restart_card", card: 1 },
    { kind: "open_browser", url_hint: null },
  ];
  const CODES = [
    "no_such_card", "no_target", "ambiguous_focus", "no_cards",
    "browser_card", "invalid_count", "router_reject",
    "dispatch_exception", "dry_run_exception", "whatever",
  ];
  it("alle dispatched-svar har audioKey i CLIP_TEXTS", () => {
    for (const intent of INTENTS) {
      for (const result of [ok(intent.kind), ...CODES.map(fail)]) {
        const reply = templateReplySource({ kind: "dispatched", intent, result });
        expect(reply.audioKey, JSON.stringify({ intent, result })).not.toBeNull();
        expect(Object.keys(CLIP_TEXTS)).toContain(reply.audioKey);
      }
    }
  });
  it("alle kæde-kombinationer har audioKey i CLIP_TEXTS", () => {
    for (const first of [ok("new_card"), fail("no_cards"), fail("whatever")]) {
      for (const second of [ok("open_browser"), fail("browser_card")]) {
        const reply = templateReplySource({
          kind: "dispatched_chain",
          intents: [INTENTS[1], INTENTS[5]] as never[],
          results: [first, second] as never[],
        });
        expect(reply.audioKey).not.toBeNull();
        expect(Object.keys(CLIP_TEXTS)).toContain(reply.audioKey);
      }
    }
  });
  it("dry_run taler ikke (audioKey null, HUD-tekst bevaret)", () => {
    const reply = templateReplySource({
      kind: "dry_run",
      intent: { kind: "restart_card", card: 2 },
      result: { ok: true, kind: "restart_card", message: "Dry-run: restart_card resolveret; ingen handling udført", dry_run: true, action_count: 0 },
    });
    expect(reply).toEqual({
      text: "Dry-run: restart_card resolveret; ingen handling udført",
      audioKey: null,
    });
  });
  it("manifestet spejler CLIP_TEXTS 1:1 (fil ⇄ kode kan ikke drifte)", () => {
    const manifestPath = fileURLToPath(
      new URL("../../voice-eval/reply-clips-manifest.txt", import.meta.url),
    );
    const rows = readFileSync(manifestPath, "utf8")
      .split(/\r?\n/u)
      .map((line) => line.trim())
      .filter((line) => line && !line.startsWith("#"))
      .map((line) => line.split("\t"));
    const fromManifest = Object.fromEntries(
      rows.map(([file, text]) => [file.replace(/\.wav$/u, ""), text]),
    );
    expect(fromManifest).toEqual(CLIP_TEXTS);
  });
  it("klip-nøglerne følger slug-konventionen af deres tekst", () => {
    const slug = (text: string) =>
      text
        .toLocaleLowerCase("da-DK")
        .replaceAll("æ", "ae").replaceAll("ø", "oe").replaceAll("å", "aa")
        .normalize("NFKD").replace(/[̀-ͯ]/gu, "")
        .replace(/[^a-z0-9]+/gu, "-").replace(/^-|-$/gu, "");
    for (const [key, text] of Object.entries(CLIP_TEXTS)) {
      expect(slug(text)).toBe(key);
    }
  });
});
