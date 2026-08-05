import { spawnSync } from "node:child_process";
import { readFile, readdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { hedgedCall } from "./hedge.mjs";
import { resolveTarget, resolveTargets } from "../src/voice/intents.ts";
import { routeVoiceTranscript } from "../src/voice/router.ts";

const ROUTES = {
  vercel: {
    base: "https://ai-gateway.vercel.sh/v1",
    model: "google/gemini-3.1-flash-lite",
    decoration: { providerOptions: { gateway: { sort: "ttft" } } },
    hedge: true,
    keySlot: "provider_key_vercel",
    envVar: "AI_GATEWAY_API_KEY",
  },
  google: {
    base: "https://generativelanguage.googleapis.com/v1beta/openai",
    model: "gemini-3.1-flash-lite",
    decoration: {},
    hedge: false,
    keySlot: "provider_key_google",
    envVar: "GOOGLE_API_KEY",
  },
  openrouter: {
    base: "https://openrouter.ai/api/v1",
    model: "google/gemini-3.1-flash-lite",
    decoration: {},
    hedge: false,
    keySlot: "provider_key_openrouter",
    envVar: "OPENROUTER_API_KEY",
  },
  // Eval-rute (ikke en produktionsrute i providers.rs): OpenAI direkte, saa
  // routerens praecision kan maales for en bruger der KUN har en OpenAI-noegle
  // og ingen Google-/Vercel-/OpenRouter-konto. Modellen kan overstyres med
  // --model, saa katalogets oevrige ID'er kan proeves uden en kodeaendring.
  openai: {
    base: "https://api.openai.com/v1",
    model: "gpt-5.6-luna",
    // reasoning_effort: "none" er IKKE en tuning-knap her, men et krav: gpt-5.6
    // afviser function tools paa /v1/chat/completions med enhver anden vaerdi
    // ("use /v1/responses or set reasoning_effort to 'none'"). Routeren er et
    // tvunget tool-kald, saa uden den staar ruten helt af. Maalt 2026-08-02.
    decoration: { reasoning_effort: "none" },
    hedge: false,
    keySlot: "provider_key_openai",
    envVar: "OPENAI_API_KEY",
  },
};

const HERE = path.dirname(fileURLToPath(import.meta.url));
const AUDIO_MODE = process.argv.includes("--audio");
const PARSE_ONLY = process.argv.includes("--parse-only");
// Sender domaene-ordlisten med til STT'en, som appen goer. Se
// `STT_DOMAIN_PROMPT` nedenfor for hvorfor det ikke er default.
const STT_PROMPT_MODE = process.argv.includes("--stt-prompt");
const CARDS = [1, 2, 3, 5].map((number) => ({ number }));
const FOCUSED = { A: 2, B: null, C: 2 };
// T6 (review fix): Gate 1's procentvise tærskler dækker KUN de oprindelige 41
// rækker (utterances.md's egen dokumentation) — u42-u48's tærskel er en
// TVÆRS-AF-KØRSLER vurdering ("≥7/7 i mindst én kørsel, ≥6/7 i begge") som
// ingen enkelt kørsel kan gate alene, så de holdes ude af Gate 1's tællere og
// rapporteres separat.
const T6_NEW_IDS = new Set(["u42", "u43", "u44", "u45", "u46", "u47", "u48"]);
const VALID_EXPECTATIONS = {
  u01: {
    kind: "send_prompt",
    card: null,
    textPattern: /test.*(?:fix|fejl|ret)|(?:fix|ret).*test/iu,
  },
  u02: { kind: "send_prompt", card: 3, textPattern: /readme/iu },
  u03: {
    kind: "send_prompt",
    card: 1,
    textPattern: /npm\s+run\s+build|build/iu,
  },
  u04: {
    kind: "send_prompt",
    card: 5,
    textPattern: /commit|pty\s+resize/iu,
  },
  u05: {
    kind: "send_prompt",
    card: null,
    textPattern: /vercel|deploy/iu,
  },
  u06: {
    kind: "send_prompt",
    card: 2,
    textPattern: /dark\s*mode/iu,
  },
  u10: { kind: "close_cards", cards: [3] },
  u11: { kind: "close_cards", cards: [5] },
  u12: { kind: "restart_card", card: 1 },
  u13: { kind: "restart_card", card: 3 },
  u22: { kind: "new_card", count: 4 },
  u23: { kind: "new_card", count: 1 },
  u24: { kind: "new_card", count: 3 },
  u25: { kind: "close_cards", cards: [2, 3] },
  u31: { kind: "open_browser", url_hint: null },
  u32: { kind: "open_browser", url_hint: "github" },
  u34: { kind: "send_prompt", card: 3, textPattern: /langt/iu },
  u42: { kind: "new_card", count: 1, agent: "codex" },
  u45: { kind: "new_card", count: 1 },
  u47: { kind: "new_card", count: 3 },
};
const CHAIN_EXPECTATIONS = {
  u35: [
    { kind: "new_card", count: 1 },
    { kind: "open_browser", url_hint: null },
  ],
  u36: [
    { kind: "close_cards", cards: [2] },
    { kind: "restart_card", card: 3 },
  ],
  u37: [
    { kind: "new_card", count: 3 },
    { kind: "open_browser", url_hint: "github" },
    { kind: "send_prompt", card: 1, textPattern: /status/iu },
  ],
  u39: [
    { kind: "close_cards", cards: [2] },
    { kind: "send_prompt", card: 3, textPattern: /langt/iu },
  ],
  u41: [
    { kind: "new_card", count: 3 },
    { kind: "open_browser", url_hint: "github" },
    { kind: "send_prompt", card: 1, textPattern: /status/iu },
    { kind: "send_prompt", card: 2, textPattern: /task\s*11/iu },
    { kind: "send_prompt", card: 3, textPattern: /task\s*12/iu },
  ],
  u43: [
    { kind: "new_card", count: 2, agent: "claude" },
    { kind: "new_card", count: 2, agent: "codex" },
  ],
  u44: [
    { kind: "new_card", count: 2, agent: "codex" },
    { kind: "send_prompt", card: 1, textPattern: /status/iu },
  ],
};
// Target-check pr. kort-bærende kæde-element (multi-spec §7): create-/browser-
// elementer har intet kort-mål og dækkes alene af intent-matchet.
const CARD_BEARING_KINDS = new Set([
  "send_prompt",
  "close_cards",
  "restart_card",
]);
const AMBIGUOUS_EXPECTATIONS = {
  u07: "router-reject",
  u08: "router-reject",
  u09: "router-reject",
  u14: "router-reject",
  u15: "router-reject",
  u16: "router-reject",
  u17: "router-reject",
  u18: "router-reject",
  u19: "router-reject",
  u20: "router-reject",
  u21: "router-reject",
  u26: "no_target",
  u27: "no_such_card",
  u28: "ambiguous_focus",
  u29: "router-reject",
  u30: "router-reject",
  u33: "router-reject",
  u38: "router-reject",
  u40: "router-reject",
  u46: "no_target",
  u48: "no_target",
};

function readWindowsCredential(targetName) {
  if (process.platform !== "win32") return null;

  const script = String.raw`
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class TalminalCredRead {
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
  public struct Credential {
    public UInt32 Flags;
    public UInt32 Type;
    public IntPtr TargetName;
    public IntPtr Comment;
    public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
    public UInt32 CredentialBlobSize;
    public IntPtr CredentialBlob;
    public UInt32 Persist;
    public UInt32 AttributeCount;
    public IntPtr Attributes;
    public IntPtr TargetAlias;
    public IntPtr UserName;
  }
  [DllImport("advapi32.dll", EntryPoint = "CredReadW", CharSet = CharSet.Unicode, SetLastError = true)]
  public static extern bool CredRead(string target, UInt32 type, UInt32 flags, out IntPtr credential);
  [DllImport("advapi32.dll", SetLastError = true)]
  public static extern void CredFree(IntPtr credential);
}
'@
$ptr = [IntPtr]::Zero
if (-not [TalminalCredRead]::CredRead('${targetName.replaceAll("'", "''")}', 1, 0, [ref]$ptr)) { exit 3 }
try {
  $credential = [Runtime.InteropServices.Marshal]::PtrToStructure($ptr, [type][TalminalCredRead+Credential])
  $bytes = New-Object byte[] $credential.CredentialBlobSize
  [Runtime.InteropServices.Marshal]::Copy($credential.CredentialBlob, $bytes, 0, $bytes.Length)
  [Console]::Out.Write([Text.Encoding]::Unicode.GetString($bytes))
} finally {
  [TalminalCredRead]::CredFree($ptr)
}
`;
  const encoded = Buffer.from(script, "utf16le").toString("base64");
  const result = spawnSync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-EncodedCommand", encoded],
    { encoding: "utf8", windowsHide: true, maxBuffer: 1024 * 1024 },
  );
  return result.status === 0 && result.stdout.trim()
    ? result.stdout.trim()
    : null;
}

function loadByok(envName, keyringUser) {
  return (
    process.env[envName]?.trim() ||
    readWindowsCredential(`${keyringUser}.Talminal`)
  );
}

function createRouterTransport(apiKey, route, options = {}) {
  return async (body) => {
    const requestBody = JSON.stringify({
      ...JSON.parse(body),
      model: route.model,
      ...route.decoration,
    });
    const fire = async () => {
      const started = performance.now();
      const response = await fetch(`${route.base}/chat/completions`, {
        method: "POST",
        headers: {
          authorization: `Bearer ${apiKey}`,
          "content-type": "application/json",
        },
        body: requestBody,
      });
      const responseText = await response.text();
      options.latencies?.push(performance.now() - started);
      // Token-forbruget opsamles ved siden af latensen: modelvalget for en
      // router afgoeres paa pris OG praecision, og uden usage-tal er
      // pris-siden et gaet. Feltet er valgfrit i svaret — mangler det, staar
      // taelleren stille i stedet for at forurene totalen med NaN.
      if (options.usage && response.ok) {
        try {
          const parsed = JSON.parse(responseText)?.usage;
          if (parsed) {
            options.usage.calls += 1;
            options.usage.input += parsed.prompt_tokens ?? 0;
            options.usage.output += parsed.completion_tokens ?? 0;
          }
        } catch {
          // Et uparsbart svar fanges alligevel af router-parsen nedenfor.
        }
      }
      if (!response.ok) {
        let message = `OpenAI-routeren fejlede (${response.status})`;
        try {
          const payload = JSON.parse(responseText);
          if (typeof payload?.error?.message === "string") {
            message = payload.error.message;
          }
        } catch {
          // Bevar statusfejlen, hvis svaret ikke er JSON.
        }
        throw new Error(message);
      }
      return responseText;
    };
    return route.hedge ? hedgedCall(fire) : fire();
  };
}

function argValue(flag) {
  const match = process.argv.find((arg) => arg.startsWith(`${flag}=`));
  if (match) return match.slice(flag.length + 1);
  const index = process.argv.indexOf(flag);
  return index >= 0 ? process.argv[index + 1] ?? null : null;
}

function percentile(sorted, fraction) {
  if (sorted.length === 0) return null;
  const index = Math.min(
    sorted.length - 1,
    Math.ceil(fraction * sorted.length) - 1,
  );
  return sorted[Math.max(0, index)];
}

function unquoteMarkdown(value) {
  const trimmed = value.trim();
  return trimmed.startsWith('"') && trimmed.endsWith('"')
    ? trimmed.slice(1, -1)
    : trimmed;
}

function parseUtterances(markdown) {
  // Rækker klassificeres efter SEKTION, ikke efter første inline-kodetoken:
  // u28/u29's forklaringstekst nævner `send_prompt` og fejlklassificeredes ellers
  // som gyldige rækker (26/25-aggregatet). Kæde-sektionen er en gyldig-sektion
  // med chain-flag (multi-spec §7).
  let inAmbiguousSection = false;
  let inChainSection = false;
  const rows = [];
  for (const line of markdown.split(/\r?\n/u)) {
    if (/^###\s/u.test(line)) {
      inAmbiguousSection = /tvetydige/iu.test(line);
      inChainSection = /kæde-cases/iu.test(line);
    }
    if (!/^\|\s*u\d{2}\s*\|/u.test(line)) continue;
    const cells = line
      .split("|")
      .slice(1, -1)
      .map((cell) => cell.trim());
    const [id, context, utterance] = cells;
    rows.push({
      id,
      context,
      utterance: unquoteMarkdown(utterance),
      ambiguous: inAmbiguousSection,
      chain: inChainSection,
    });
  }
  return rows;
}

function validateParsedRows(rows) {
  const validRows = rows.filter((row) => !row.ambiguous && !row.chain);
  const chainRows = rows.filter((row) => row.chain);
  const ambiguousRows = rows.filter((row) => row.ambiguous);
  const expectedValidIds = Object.keys(VALID_EXPECTATIONS);
  const expectedChainIds = Object.keys(CHAIN_EXPECTATIONS);
  const expectedAmbiguousIds = Object.keys(AMBIGUOUS_EXPECTATIONS);
  const duplicateIds = rows
    .map((row) => row.id)
    .filter((id, index, ids) => ids.indexOf(id) !== index);

  if (duplicateIds.length > 0) {
    throw new Error(
      `Duplikerede utterance-ID'er: ${[...new Set(duplicateIds)].join(", ")}`,
    );
  }

  const actualValidIds = validRows.map((row) => row.id);
  const actualChainIds = chainRows.map((row) => row.id);
  const actualAmbiguousIds = ambiguousRows.map((row) => row.id);
  if (
    validRows.length !== 20 ||
    chainRows.length !== 7 ||
    ambiguousRows.length !== 21 ||
    expectedValidIds.some((id) => !actualValidIds.includes(id)) ||
    expectedChainIds.some((id) => !actualChainIds.includes(id)) ||
    expectedAmbiguousIds.some((id) => !actualAmbiguousIds.includes(id)) ||
    actualValidIds.some((id) => !(id in VALID_EXPECTATIONS)) ||
    actualChainIds.some((id) => !(id in CHAIN_EXPECTATIONS)) ||
    actualAmbiguousIds.some((id) => !(id in AMBIGUOUS_EXPECTATIONS))
  ) {
    throw new Error(
      `Forventede præcis 20 gyldige + 7 kæder + 21 tvetydige rækker; fandt ${validRows.length} + ${chainRows.length} + ${ambiguousRows.length}`,
    );
  }

  for (const row of rows) {
    if (!(row.context in FOCUSED) || !row.utterance) {
      throw new Error(`Ugyldig kontekst eller tom utterance i ${row.id}`);
    }
  }

  return { validRows, chainRows, ambiguousRows };
}

function normalizedIntent(intent) {
  if (!intent) return null;
  switch (intent.kind) {
    case "send_prompt":
      return {
        kind: intent.kind,
        card: intent.card,
        textPattern: intent.text,
      };
    case "new_card":
      return {
        kind: intent.kind,
        count: intent.count,
        ...(intent.agent === undefined ? {} : { agent: intent.agent }),
      };
    case "close_cards":
      return {
        kind: intent.kind,
        cards: intent.cards,
        ...(intent.all === undefined ? {} : { all: intent.all }),
      };
    case "restart_card":
      return { kind: intent.kind, card: intent.card };
    case "open_browser":
      return { kind: intent.kind, url_hint: intent.url_hint };
    default:
      return { kind: intent.kind };
  }
}

function arraysEqual(left, right) {
  return (
    Array.isArray(left) &&
    Array.isArray(right) &&
    left.length === right.length &&
    left.every((value, index) => value === right[index])
  );
}

function intentMatchesExpectation(intent, expectation) {
  const actual = normalizedIntent(intent);
  if (!actual) return false;

  const actualKeys = Object.keys(actual).sort();
  const expectedKeys = Object.keys(expectation).sort();
  if (!arraysEqual(actualKeys, expectedKeys)) return false;

  return expectedKeys.every((key) => {
    const expectedValue = expectation[key];
    const actualValue = actual[key];
    if (key === "textPattern") {
      return (
        expectedValue instanceof RegExp &&
        typeof actualValue === "string" &&
        expectedValue.test(actualValue)
      );
    }
    if (Array.isArray(expectedValue)) {
      return arraysEqual(actualValue, expectedValue);
    }
    return actualValue === expectedValue;
  });
}

function actualTarget(intent, context) {
  if (!intent) return { reason: "router-reject" };
  if (intent.kind === "close_cards") {
    const result = resolveTargets(intent, CARDS);
    return result.ok ? { cards: result.cards } : { reason: result.reason };
  }
  if (intent.kind === "new_card") {
    // v4-new_card bærer intet cwd_hint — målet er altid projekt-roden.
    return { cwd_hint: null };
  }
  if (intent.kind === "open_browser") {
    return { url_hint: intent.url_hint ?? null };
  }
  const result = resolveTarget(intent, FOCUSED[context], CARDS);
  return result.ok ? { card: result.card } : { reason: result.reason };
}

function targetMatchesExpectation(target, expectation, context) {
  const expectedTarget = actualTarget(expectation, context);
  if ("reason" in expectedTarget || "reason" in target) {
    return (
      "reason" in expectedTarget &&
      "reason" in target &&
      target.reason === expectedTarget.reason
    );
  }
  if ("cards" in expectedTarget || "cards" in target) {
    return (
      "cards" in expectedTarget &&
      "cards" in target &&
      arraysEqual(target.cards, expectedTarget.cards)
    );
  }
  if ("cwd_hint" in expectedTarget || "cwd_hint" in target) {
    return (
      "cwd_hint" in expectedTarget &&
      "cwd_hint" in target &&
      target.cwd_hint === expectedTarget.cwd_hint
    );
  }
  if ("url_hint" in expectedTarget || "url_hint" in target) {
    return (
      "url_hint" in expectedTarget &&
      "url_hint" in target &&
      target.url_hint === expectedTarget.url_hint
    );
  }
  return (
    "card" in expectedTarget &&
    "card" in target &&
    target.card === expectedTarget.card
  );
}

function audioMime(extension) {
  return {
    ".flac": "audio/flac",
    ".m4a": "audio/mp4",
    ".mp3": "audio/mpeg",
    ".ogg": "audio/ogg",
    ".wav": "audio/wav",
    ".webm": "audio/webm",
  }[extension.toLowerCase()] ?? "application/octet-stream";
}

async function audioFilesById() {
  const audioDir = path.join(HERE, "audio");
  let names;
  try {
    names = await readdir(audioDir);
  } catch {
    return new Map();
  }
  return new Map(
    names
      .filter((name) => /^u\d{2}\./iu.test(name))
      .map((name) => [name.slice(0, 3).toLowerCase(), path.join(audioDir, name)]),
  );
}

/**
 * STT-modellen eval'et transskriberer med.
 *
 * Skal foelge `providers.rs`' STT_ROUTES. Den stod hardkodet paa
 * `gpt-4o-transcribe` og blev ikke opdaget da appen skiftede model — saa
 * eval'et ville have maalt en model appen ikke bruger, og Gate 1's tal ville
 * vaere afkoblet fra produktet uden at noget blev roedt.
 *
 * Overstyres med `--stt-model <navn>`, saa to STT-modeller kan sammenlignes
 * over praecis samme lyd og samme scoringskode.
 */
const DEFAULT_STT_MODEL = "gpt-transcribe";

/**
 * Domaene-prompten sendes KUN med `--stt-prompt`.
 *
 * Det er med vilje ikke default: alle historiske tal (Gate 1 41/41, T6 7/7) er
 * maalt UDEN prompt, og at taende den som standard ville flytte baseline uden
 * at nogen bad om det. Men appen SENDER den, saa uden flaget maaler eval'et
 * ikke den vej brugeren koerer paa — foerst med flaget er de to ens.
 *
 * Bemaerk at fil-endpointet uanset hvad er en anden transport end appens
 * realtime-session. Flaget lukker model- og prompt-hullet, ikke transport-
 * hullet.
 */
const STT_DOMAIN_PROMPT =
  "Dansk kommando til en canvas af nummererede kort: kort et, to, tre, fire, fem, seks, syv, otte, ni, ti. Typiske verber: luk, genstart, åbn, opret, send, sig til, bed. Ord som terminal, browser, projekt, canvas. Flere kommandoer kan kædes med og. Blandet dansk og engelsk kan forekomme.";

async function transcribeAudio(file, apiKey, options = {}) {
  const model = options.model ?? DEFAULT_STT_MODEL;
  const form = new FormData();
  form.append("model", model);
  if (options.prompt) form.append("prompt", STT_DOMAIN_PROMPT);
  form.append(
    "file",
    new Blob([await readFile(file)], { type: audioMime(path.extname(file)) }),
    path.basename(file),
  );
  const response = await fetch("https://api.openai.com/v1/audio/transcriptions", {
    method: "POST",
    headers: { authorization: `Bearer ${apiKey}` },
    body: form,
  });
  const payload = await response.json();
  if (!response.ok) {
    throw new Error(
      payload?.error?.message ?? `OpenAI STT fejlede (${response.status})`,
    );
  }
  if (typeof payload.text !== "string" || !payload.text.trim()) {
    throw new Error(`Tom STT-transskription for ${path.basename(file)}`);
  }
  return payload.text.trim();
}

function reportLine(row, intent, target, checks, transcript) {
  const audioNote = AUDIO_MODE ? ` | STT=${JSON.stringify(transcript)}` : "";
  const checkText = Object.entries(checks)
    .map(([name, passed]) => `${name}:${passed ? "PASS" : "FAIL"}`)
    .join(",");
  const passed = Object.values(checks).every(Boolean);
  console.log(
    `${passed ? "PASS" : "FAIL"} ${row.id} intent=${JSON.stringify(intent)} target=${JSON.stringify(target)} checks=${checkText}${audioNote}`,
  );
}

async function main() {
  const routeName = argValue("--route");
  if (routeName === null || !(routeName in ROUTES)) {
    console.error(
      `BLOCKED: --route mangler eller er ukendt. Vaelg en af: ${Object.keys(ROUTES).join(", ")}`,
    );
    process.exitCode = 1;
    return;
  }
  // --model overstyrer rutens model uden kodeaendring. Ruten bestemmer fortsat
  // endpoint, noegle, dekoration og hedge — kun modelnavnet flyttes, saa to
  // modeller kan sammenlignes over praecis samme transport.
  const modelOverride = argValue("--model");
  const route =
    modelOverride === null
      ? ROUTES[routeName]
      : { ...ROUTES[routeName], model: modelOverride };
  const rows = parseUtterances(
    await readFile(path.join(HERE, "utterances.md"), "utf8"),
  );
  const { validRows, chainRows, ambiguousRows } = validateParsedRows(rows);
  if (PARSE_ONLY) {
    console.log(
      `Parse-only: ${validRows.length} gyldige + ${chainRows.length} kæder + ${ambiguousRows.length} tvetydige rækker.`,
    );
    // Gate 1-scope-bevis (review-fix, uden API-kald): samme T6_NEW_IDS-filter
    // som main()'s tællere bruger, kørt mod de faktisk parsede rækker.
    const originalValid = validRows.filter(
      (row) => !T6_NEW_IDS.has(row.id),
    ).length;
    const originalChain = chainRows.filter(
      (row) => !T6_NEW_IDS.has(row.id),
    ).length;
    const originalAmbiguous = ambiguousRows.filter(
      (row) => !T6_NEW_IDS.has(row.id),
    ).length;
    const newCasesTotal =
      validRows.length +
      chainRows.length +
      ambiguousRows.length -
      originalValid -
      originalChain -
      originalAmbiguous;
    console.log(
      `Gate 1-scope (oprindelige rækker): ${originalValid} gyldige + ${originalChain} kæder + ${originalAmbiguous} tvetydige = ${originalValid + originalChain + originalAmbiguous} rækker (T6 nye cases uden for scope: ${newCasesTotal}).`,
    );
    return;
  }

  const routerApiKey = loadByok(route.envVar, route.keySlot);
  if (!routerApiKey) {
    console.error(
      `BLOCKED: noegle mangler. Saet ${route.envVar} eller gem ${route.keySlot} i Talminal keyring.`,
    );
    process.exitCode = 2;
    return;
  }
  const routerLatencies = [];
  const routerUsage = { calls: 0, input: 0, output: 0 };
  const routerTransport = createRouterTransport(routerApiKey, route, {
    latencies: routerLatencies,
    usage: routerUsage,
  });

  let sttApiKey = null;
  let audioFiles = new Map();
  if (AUDIO_MODE) {
    sttApiKey = loadByok("OPENAI_API_KEY", "provider_key_openai");
    audioFiles = await audioFilesById();
    // Kun u01-u30 har indspilninger; u31-u48 falder tilbage til ground-truth-
    // teksten i loopet (reviewer-B1).
    const requiredRows = rows.filter((row) => Number(row.id.slice(1)) <= 30);
    const missingIds = requiredRows
      .filter((row) => !audioFiles.has(row.id))
      .map((row) => row.id);
    if (!sttApiKey || missingIds.length > 0) {
      console.error(
        `BLOCKED: --audio kræver OPENAI_API_KEY/keyring og audiofiler for u01-u30${missingIds.length > 0 ? ` (mangler ${missingIds.join(", ")})` : ""}.`,
      );
      process.exitCode = 2;
      return;
    }
  }

  let intentCorrect = 0;
  let targetCorrect = 0;
  let validCommandsCorrect = 0;
  let ambiguousRejected = 0;
  let newCaseCorrect = 0;
  let newCaseChecked = 0;
  for (const row of rows) {
    // u31-u48 har ingen indspilninger — ground-truth-teksten er fallback.
    const transcript =
      AUDIO_MODE && audioFiles.has(row.id)
        ? await transcribeAudio(audioFiles.get(row.id), sttApiKey, {
            model: argValue("--stt-model") ?? undefined,
            prompt: STT_PROMPT_MODE,
          })
        : row.utterance;
    const intents = await routeVoiceTranscript(transcript, {
      transport: routerTransport,
    });

    if (row.chain) {
      // Kæde-rækker: længde + HVERT element i rækkefølge; rækken tæller som ÉN
      // række i både intent- og target-metrikken (multi-spec §7).
      const expectations = CHAIN_EXPECTATIONS[row.id];
      const lengthMatches =
        Array.isArray(intents) && intents.length === expectations.length;
      const intentPassed =
        lengthMatches &&
        expectations.every((expectation, index) =>
          intentMatchesExpectation(intents[index], expectation),
        );
      const targetPassed =
        lengthMatches &&
        expectations.every((expectation, index) => {
          if (!CARD_BEARING_KINDS.has(expectation.kind)) return true;
          return targetMatchesExpectation(
            actualTarget(intents[index], row.context),
            expectation,
            row.context,
          );
        });
      const commandPassed = intentPassed && targetPassed;
      if (T6_NEW_IDS.has(row.id)) {
        newCaseChecked += 1;
        newCaseCorrect += Number(commandPassed);
      } else {
        intentCorrect += Number(intentPassed);
        targetCorrect += Number(targetPassed);
        validCommandsCorrect += Number(commandPassed);
      }
      reportLine(
        row,
        intents,
        (Array.isArray(intents) ? intents : []).map((intent) =>
          actualTarget(intent, row.context),
        ),
        { intent: intentPassed, target: targetPassed },
        transcript,
      );
      continue;
    }

    if (!row.ambiguous) {
      // Single-rækker: kræv præcis ét element i arrayet.
      const expectation = VALID_EXPECTATIONS[row.id];
      const single = intents?.length === 1 ? intents[0] : null;
      const target = actualTarget(single, row.context);
      const intentPassed =
        intents?.length === 1 && intentMatchesExpectation(single, expectation);
      const targetPassed =
        intents?.length === 1 &&
        targetMatchesExpectation(target, expectation, row.context);
      const commandPassed = intentPassed && targetPassed;
      if (T6_NEW_IDS.has(row.id)) {
        newCaseChecked += 1;
        newCaseCorrect += Number(commandPassed);
      } else {
        intentCorrect += Number(intentPassed);
        targetCorrect += Number(targetPassed);
        validCommandsCorrect += Number(commandPassed);
      }
      reportLine(
        row,
        intents,
        target,
        { intent: intentPassed, target: targetPassed },
        transcript,
      );
      continue;
    }

    // Tvetydige rækker (reviewer-K2): router-reject-rækkerne er intents===null;
    // u26/u27/u28 forventer et GYLDIGT length-1-intent som RESOLVEREN afviser
    // (routerens egen kontrakt, jf. few-shot "Luk to af dem."). En kæde (>=2)
    // failer ALTID — forventningerne må aldrig "løses" ved at tune routeren til
    // at reject'e "Luk kortet."-klassen.
    const expectedReason = AMBIGUOUS_EXPECTATIONS[row.id];
    const target =
      intents === null
        ? actualTarget(null, row.context)
        : intents.length === 1
          ? actualTarget(intents[0], row.context)
          : { reason: "unexpected_chain" };
    const rejected = "reason" in target && target.reason === expectedReason;
    if (T6_NEW_IDS.has(row.id)) {
      newCaseChecked += 1;
      newCaseCorrect += Number(rejected);
    } else {
      ambiguousRejected += Number(rejected);
    }
    reportLine(row, intents, target, { ambiguous: rejected }, transcript);
  }

  // Gate 1's procentvise tærskler er scopet til de OPRINDELIGE 41 rækker
  // (jf. utterances.md's egen dokumentation af Gate 1) — u42-u48 tælles og
  // printes separat nedenfor, uden for disse totaler.
  const originalValidRows = validRows.filter((row) => !T6_NEW_IDS.has(row.id));
  const originalChainRows = chainRows.filter((row) => !T6_NEW_IDS.has(row.id));
  const originalAmbiguousRows = ambiguousRows.filter(
    (row) => !T6_NEW_IDS.has(row.id),
  );
  const intentTotal = originalValidRows.length + originalChainRows.length;
  const targetTotal = intentTotal;
  const ambiguousTotal = originalAmbiguousRows.length;
  const intentAccuracy = intentCorrect / intentTotal;
  const targetAccuracy = targetCorrect / targetTotal;
  console.log("");
  // Modellen kommer fra den valgte rute, ikke fra en global i router.ts: siden
  // Rust begyndte at injicere model og dekoration, eksporterer TS ingen
  // ROUTER_MODEL længere. Linjen refererede stadig til den — og faldt først
  // HER, i opsummeringen, efter alle 48 kald var betalt og bestået.
  console.log(`Model: ${route.model} (rute: ${routeName})`);
  // STT-modellen SKAL staa i opsummeringen. Den var hardkodet og usynlig, og
  // det er praecis derfor den kunne blive haengende paa en model appen ikke
  // laengere brugte: et tal uden en kvittering paa hvad der blev maalt, laeses
  // som om det gjaldt produktet.
  if (AUDIO_MODE) {
    const sttModel = argValue("--stt-model") ?? DEFAULT_STT_MODEL;
    console.log(
      `STT: ${sttModel} · domaene-prompt ${STT_PROMPT_MODE ? "SENDT" : "ikke sendt"} · transport: fil (appen koerer realtime)`,
    );
  }
  if (routerLatencies.length > 0) {
    const sorted = [...routerLatencies].sort((a, b) => a - b);
    console.log(
      `Router-latens (${sorted.length} kald): p50 ${percentile(sorted, 0.5).toFixed(1)} ms · p95 ${percentile(sorted, 0.95).toFixed(1)} ms · max ${sorted[sorted.length - 1].toFixed(1)} ms`,
    );
  }
  if (routerUsage.calls > 0) {
    // Tokens PR. YTRING er det tal en pris-sammenligning skal hvile paa —
    // totalen afhaenger af hvor mange raekker der blev koert. Prisen selv
    // staar bevidst ikke her: den aendrer sig uden at koden goer, og en
    // hardkodet takst ville ligge og lyve stille.
    console.log(
      `Router-tokens (${routerUsage.calls} kald): ${routerUsage.input} ind + ${routerUsage.output} ud = ${routerUsage.input + routerUsage.output} i alt · pr. ytring ${(routerUsage.input / routerUsage.calls).toFixed(0)} ind / ${(routerUsage.output / routerUsage.calls).toFixed(0)} ud`,
    );
  }
  console.log(
    `Intent: ${intentCorrect}/${intentTotal} (${(intentAccuracy * 100).toFixed(1)}%)`,
  );
  console.log(
    `Target: ${targetCorrect}/${targetTotal} (${(targetAccuracy * 100).toFixed(1)}%)`,
  );
  console.log(`Gyldige intent+mål: ${validCommandsCorrect}/${intentTotal}`);
  console.log(`Tvetydige afvist: ${ambiguousRejected}/${ambiguousTotal}`);
  console.log(
    `Samlet korrekt/afvist: ${validCommandsCorrect + ambiguousRejected}/${intentTotal + ambiguousTotal}`,
  );
  // IKKE-GATENDE: T6's u42-u48-tærskel ("≥7/7 i mindst én kørsel, ≥6/7 i
  // begge") er en vurdering PÅ TVÆRS AF TO KØRSLER — operatøren læser dette
  // tal fra begge kørsler og sammenligner selv; denne linje afgør intet alene
  // og indgår ikke i Gate 1 nedenfor.
  console.log(
    `Nye agent-cases (T6, u42-u48, IKKE-gatende — kræver læsning på tværs af to kørsler): ${newCaseCorrect}/${newCaseChecked}`,
  );

  const gatePassed =
    intentAccuracy < 0.9 ||
    targetAccuracy < 0.95 ||
    ambiguousRejected !== 19 ||
    ambiguousTotal !== 19
      ? false
      : true;
  console.log(`Gate 1: ${gatePassed ? "PASS" : "FAIL"}`);
  if (!gatePassed) {
    process.exitCode = 1;
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.stack : error);
  process.exitCode = 1;
});
