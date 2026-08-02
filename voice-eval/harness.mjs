// Kæde-ytringer mod single-intent-schemaet er udefinerede (multi-spec §10.2):
// kæde-cases er pipeline-only, så realtime-sporet filtrerer rækkerne fra pr. ID
// (parseren har intet chain-flag — reviewer-K1).
// u42-u48 (T6): agent-valg er pipeline-only i v1 (plan-globalt constraint) —
// realtime-sporets 5-tool-schema har ingen agent-param og kan hverken
// repraesentere new_card's agent-felt eller de nye kaeder (u43/u44), saa hele
// T6-tillaegget skippes ligesom de oprindelige kaede-rækker ovenfor.
export const REALTIME_SKIP_IDS = new Set([
  "u35",
  "u36",
  "u37",
  "u38",
  "u39",
  "u40",
  "u41",
  "u42",
  "u43",
  "u44",
  "u45",
  "u46",
  "u47",
  "u48",
]);

function unquoteMarkdown(value) {
  const trimmed = value.trim();
  return trimmed.startsWith('"') && trimmed.endsWith('"')
    ? trimmed.slice(1, -1)
    : trimmed;
}

export function parseUtterances(markdown) {
  let classification = null;
  const rows = [];
  for (const line of markdown.split(/\r?\n/u)) {
    if (/^###\s+Gyldige kommandoer\b/u.test(line)) {
      classification = "valid";
      continue;
    }
    if (/^###\s+Bevidst tvetydige\b/u.test(line)) {
      classification = "ambiguous";
      continue;
    }
    if (!/^\|\s*u\d{2}\s*\|/u.test(line)) continue;
    if (classification === null) {
      throw new Error("Utterance row is outside a classified section");
    }

    const cells = line
      .split("|")
      .slice(1, -1)
      .map((cell) => cell.trim());
    const [id, context, utterance, , expectedIntent, expectedTarget] = cells;
    const expectedKind = classification === "ambiguous"
      ? null
      : expectedIntent.match(/`([a-z_]+)/u)?.[1] ?? null;
    const targetMatches = [...expectedTarget.matchAll(/\b(\d+)\b/gu)].map(
      (match) => Number(match[1]),
    );
    const expectedReason = classification === "ambiguous"
      ? line.match(/\b(no_target|no_such_card|ambiguous_focus|router-reject)\b/u)?.[1] ??
        null
      : null;
    const expectedCountMatch = expectedIntent.match(/\bcount:\s*(\d+)\b/u);
    const expectedCwdHintMatch = expectedIntent.match(/\bcwd_hint:\s*"([^"]+)"/u);

    rows.push({
      id,
      context,
      classification,
      utterance: unquoteMarkdown(utterance),
      expectedKind,
      expectedTarget:
        expectedKind === "close_cards" || targetMatches.length === 0
          ? null
          : targetMatches[0],
      expectedTargets:
        expectedKind === "close_cards" ? targetMatches : null,
      expectedCount: expectedCountMatch ? Number(expectedCountMatch[1]) : null,
      expectedCwdHint: expectedCwdHintMatch?.[1] ?? null,
      expectsProjectRoot:
        expectedKind === "new_card_in_project" &&
        /\bprojekt-rod\b/u.test(expectedTarget),
      expectedReason,
    });
  }
  return rows;
}
