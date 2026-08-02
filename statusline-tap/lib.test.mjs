import { describe, expect, it } from "vitest";
import {
  buildDebugRecord,
  clampPercent,
  contextFileKey,
  debugSessionKey,
  extractContext,
  extractUsage,
  toIsoReset,
} from "./lib.mjs";

describe("extractUsage", () => {
  const NOW = "2026-07-21T19:00:00.000Z";

  it("uddrager 5h+uge fra en fuld payload med epoch-sekunder", () => {
    const payload = JSON.stringify({
      session_id: "abc-123",
      rate_limits: {
        five_hour: { used_percentage: 23.5, resets_at: 1784672400 },
        seven_day: { used_percentage: 41.2, resets_at: 1785222000 },
      },
    });
    expect(extractUsage(payload, NOW)).toEqual({
      version: 1,
      writtenAt: NOW,
      fiveHourPercent: 23.5,
      fiveHourResetsAt: new Date(1784672400 * 1000).toISOString(),
      weeklyPercent: 41.2,
      weeklyResetsAt: new Date(1785222000 * 1000).toISOString(),
      sessionId: "abc-123",
    });
  });

  it("returnerer null uden rate_limits (API-nøgle-bruger / frisk session)", () => {
    expect(extractUsage(JSON.stringify({ session_id: "x" }), NOW)).toBeNull();
  });

  it("returnerer null ved ugyldig JSON", () => {
    expect(extractUsage("ikke json", NOW)).toBeNull();
  });

  it("tåler manglende seven_day og klamper >100", () => {
    const payload = JSON.stringify({
      rate_limits: { five_hour: { used_percentage: 130 } },
    });
    expect(extractUsage(payload, NOW)).toEqual({
      version: 1,
      writtenAt: NOW,
      fiveHourPercent: 100,
      fiveHourResetsAt: null,
      weeklyPercent: null,
      weeklyResetsAt: null,
      sessionId: null,
    });
  });
});

describe("extractContext", () => {
  const NOW = "2026-07-22T10:00:00.000Z";
  const CARD_ENV = { TALMINAL_SESSION_ID: "kort-3", TALMINAL_RUN_ID: "run-uuid-1" };
  const FULL_PAYLOAD = JSON.stringify({
    session_id: "cc-session-1",
    cwd: "C:\\projekter\\demo",
    model: { id: "claude-fable-5", display_name: "Fable 5" },
    context_window: { used_percentage: 18, context_window_size: 1000000 },
  });

  it("uddrager context v1 fra fuld payload med kort-env", () => {
    expect(extractContext(FULL_PAYLOAD, CARD_ENV, NOW)).toEqual({
      version: 1,
      writtenAt: NOW,
      cardName: "kort-3",
      runId: "run-uuid-1",
      sessionId: "cc-session-1",
      cwd: "C:\\projekter\\demo",
      usedPercent: 18,
      windowSize: 1000000,
      modelDisplayName: "Fable 5",
    });
  });

  it("returnerer null uden TALMINAL_SESSION_ID (ekstern terminal)", () => {
    expect(extractContext(FULL_PAYLOAD, {}, NOW)).toBeNull();
    expect(extractContext(FULL_PAYLOAD, { TALMINAL_SESSION_ID: "" }, NOW)).toBeNull();
  });

  it("returnerer null ved ugyldig JSON, manglende context_window eller manglende cwd", () => {
    expect(extractContext("ikke json", CARD_ENV, NOW)).toBeNull();
    expect(extractContext(JSON.stringify({ cwd: "C:\\x" }), CARD_ENV, NOW)).toBeNull();
    expect(
      extractContext(
        JSON.stringify({ context_window: { used_percentage: 5 } }),
        CARD_ENV,
        NOW,
      ),
    ).toBeNull();
  });

  it("klamper usedPercent og tåler manglende valgfrie felter", () => {
    const payload = JSON.stringify({
      cwd: "C:\\x",
      context_window: { used_percentage: 130 },
    });
    expect(extractContext(payload, { TALMINAL_SESSION_ID: "kort-1" }, NOW)).toEqual({
      version: 1,
      writtenAt: NOW,
      cardName: "kort-1",
      runId: null,
      sessionId: null,
      cwd: "C:\\x",
      usedPercent: 100,
      windowSize: null,
      modelDisplayName: null,
    });
  });
});

describe("contextFileKey", () => {
  it("bevarer filnavn-sikre kortnavne og erstatter resten med _", () => {
    expect(contextFileKey("kort-3")).toBe("kort-3");
    expect(contextFileKey("Kort_3.b")).toBe("Kort_3.b");
    expect(contextFileKey("kort 3:æble/..\\x")).toBe("kort_3__ble_.._x");
  });
});

describe("debugSessionKey", () => {
  it("bruger session_id fra payloaden når den er filnavn-sikker", () => {
    expect(debugSessionKey(JSON.stringify({ session_id: "abc-123" }), 42)).toBe("abc-123");
  });
  it("falder tilbage til pid ved ugyldig JSON", () => {
    expect(debugSessionKey("ikke json", 42)).toBe("pid-42");
  });
  it("falder tilbage til pid når session_id mangler eller er usikker", () => {
    expect(debugSessionKey(JSON.stringify({}), 7)).toBe("pid-7");
    expect(debugSessionKey(JSON.stringify({ session_id: "..\\evil" }), 7)).toBe("pid-7");
  });
});

describe("buildDebugRecord", () => {
  const NOW = "2026-07-22T09:00:00.000Z";

  it("registrerer env-tilstand, udtræks-udfald og rå payload", () => {
    const record = buildDebugRecord({
      payloadText: '{"session_id":"s"}',
      snapshot: null,
      writeResult: "no-snapshot",
      contextResult: "no-context",
      env: { LOCALAPPDATA: "C:\\Users\\x\\AppData\\Local", TALMINAL_HOME: "C:\\proj" },
      nowIso: NOW,
      pid: 99,
    });
    expect(record).toEqual({
      version: 1,
      writtenAt: NOW,
      pid: 99,
      env: {
        TALMINAL_GLOBAL_HOME: null,
        LOCALAPPDATA: "C:\\Users\\x\\AppData\\Local",
        TALMINAL_HOME: "C:\\proj",
        TALMINAL_SESSION_ID: null,
      },
      extractOk: false,
      writeResult: "no-snapshot",
      contextResult: "no-context",
      rawStdin: '{"session_id":"s"}',
    });
  });

  it("markerer extractOk når et snapshot fandtes", () => {
    const record = buildDebugRecord({
      payloadText: "{}",
      snapshot: { version: 1 },
      writeResult: "written",
      env: { TALMINAL_SESSION_ID: "kort-2" },
      nowIso: NOW,
      pid: 1,
    });
    expect(record.extractOk).toBe(true);
    expect(record.writeResult).toBe("written");
    expect(record.contextResult).toBeNull();
    expect(record.env.LOCALAPPDATA).toBeNull();
    expect(record.env.TALMINAL_SESSION_ID).toBe("kort-2");
  });
});

describe("toIsoReset", () => {
  it("epoch-sekunder → ISO", () => {
    expect(toIsoReset(1784672400)).toBe(new Date(1784672400000).toISOString());
  });
  it("epoch-millisekunder → ISO", () => {
    expect(toIsoReset(1784672400000)).toBe(new Date(1784672400000).toISOString());
  });
  it("ISO-streng bevares", () => {
    expect(toIsoReset("2026-07-21T22:20:00.000Z")).toBe("2026-07-21T22:20:00.000Z");
  });
  it("null/skrald → null", () => {
    expect(toIsoReset(null)).toBeNull();
    expect(toIsoReset("snart")).toBeNull();
  });
});

describe("clampPercent", () => {
  it("klamper til 0-100 og afviser ikke-tal", () => {
    expect(clampPercent(-5)).toBe(0);
    expect(clampPercent(150)).toBe(100);
    expect(clampPercent(Number.NaN)).toBeNull();
    expect(clampPercent("7")).toBeNull();
  });
});
