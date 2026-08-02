// Ren logik for statusline-tap'en — testbar uden fs/proces-sideeffekter.
// Kontrakt v1 (FROSSEN, se planens Global Constraints): camelCase, procenter
// 0-100, resets som ISO-8601 eller null. resets_at fra Claude Code kan være
// epoch-sekunder, epoch-ms eller ISO-streng — heuristikken < 1e12 = sekunder.
// session_id stammer fra CC's statusline-docs (IKKE verificeret mod en anden
// statusline-implementerings StatuslineStdin-type — review-fund 7); worst case
// er feltet altid null.

const EPOCH_MS_THRESHOLD = 1e12;

export function clampPercent(value) {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return Math.max(0, Math.min(100, value));
}

export function toIsoReset(value) {
  if (value == null) return null;
  const numeric = typeof value === "number"
    ? value
    : typeof value === "string" && value.trim() !== "" ? Number(value) : Number.NaN;
  if (Number.isFinite(numeric)) {
    const millis = Math.abs(numeric) < EPOCH_MS_THRESHOLD ? numeric * 1000 : numeric;
    const date = new Date(millis);
    return Number.isNaN(date.getTime()) ? null : date.toISOString();
  }
  if (typeof value === "string") {
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? null : date.toISOString();
  }
  return null;
}

// Context v1 (per-kort context-badge, se docs/superpowers/specs/
// 2026-07-22-card-context-badge-design.md): skrives KUN for kort-sessioner —
// TALMINAL_SESSION_ID (kortnavnet) arves fra canvas'ens PTY-spawn, eksterne
// terminaler har den ikke. cardName+cwd+usedPercent er obligatoriske
// (frontendens join kræver navn+cwd); resten er tolerant-nullable.

export function extractContext(payloadText, env, nowIso) {
  const cardName = env.TALMINAL_SESSION_ID;
  if (typeof cardName !== "string" || cardName === "") return null;
  let payload;
  try {
    payload = JSON.parse(payloadText);
  } catch {
    return null;
  }
  const usedPercent = clampPercent(payload?.context_window?.used_percentage);
  if (usedPercent == null) return null;
  const cwd = payload?.cwd;
  if (typeof cwd !== "string" || cwd === "") return null;
  const windowSize = payload?.context_window?.context_window_size;
  const modelDisplayName = payload?.model?.display_name;
  const runId = env.TALMINAL_RUN_ID;
  return {
    version: 1,
    writtenAt: nowIso,
    cardName,
    runId: typeof runId === "string" && runId !== "" ? runId : null,
    sessionId: typeof payload?.session_id === "string" ? payload.session_id : null,
    cwd,
    usedPercent,
    windowSize: typeof windowSize === "number" && Number.isFinite(windowSize) ? windowSize : null,
    modelDisplayName: typeof modelDisplayName === "string" ? modelDisplayName : null,
  };
}

// Filnøgle for context-filen: kortnavnet gjort filnavn-sikkert (spec:
// alt uden for [A-Za-z0-9._-] erstattes med _).
export function contextFileKey(cardName) {
  return cardName.replace(/[^A-Za-z0-9._-]/g, "_");
}

// Debug-instrumentering (rodårsags-jagt: kort-sessioner der aldrig skriver
// usage.json). buildDebugRecord/debugSessionKey er rene; tap.mjs skriver
// recorden ved siden af det deployede script når tap-debug.on findes dér
// (eller TALMINAL_TAP_DEBUG er sat). rawStdin er hele statusline-payloaden —
// lokal diagnostik, ingen hemmeligheder ud over hvad CC selv sender.

const SAFE_SESSION_KEY = /^[A-Za-z0-9-]+$/;

export function debugSessionKey(payloadText, pid) {
  try {
    const sessionId = JSON.parse(payloadText)?.session_id;
    if (typeof sessionId === "string" && SAFE_SESSION_KEY.test(sessionId)) return sessionId;
  } catch {
    /* falder igennem til pid */
  }
  return `pid-${pid}`;
}

export function buildDebugRecord({ payloadText, snapshot, writeResult, contextResult, env, nowIso, pid }) {
  return {
    version: 1,
    writtenAt: nowIso,
    pid,
    env: {
      TALMINAL_GLOBAL_HOME: env.TALMINAL_GLOBAL_HOME ?? null,
      LOCALAPPDATA: env.LOCALAPPDATA ?? null,
      TALMINAL_HOME: env.TALMINAL_HOME ?? null,
      TALMINAL_SESSION_ID: env.TALMINAL_SESSION_ID ?? null,
    },
    extractOk: snapshot != null,
    writeResult,
    contextResult: contextResult ?? null,
    rawStdin: payloadText,
  };
}

export function extractUsage(payloadText, nowIso) {
  let payload;
  try {
    payload = JSON.parse(payloadText);
  } catch {
    return null;
  }
  const limits = payload?.rate_limits;
  const fiveHourPercent = clampPercent(limits?.five_hour?.used_percentage);
  if (fiveHourPercent == null) return null;
  return {
    version: 1,
    writtenAt: nowIso,
    fiveHourPercent,
    fiveHourResetsAt: toIsoReset(limits?.five_hour?.resets_at),
    weeklyPercent: clampPercent(limits?.seven_day?.used_percentage),
    weeklyResetsAt: toIsoReset(limits?.seven_day?.resets_at),
    sessionId: typeof payload?.session_id === "string" ? payload.session_id : null,
  };
}
