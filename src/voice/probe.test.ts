import { beforeEach, describe, expect, it, vi } from "vitest";
import { probeSttRoute } from "./probe";

// Testen maalte foer batch-grenen i probeSttRoute. Den gren blev slettet
// sammen med OpenRouter-STT (2026-07-29), og tilbage er den egenskab der
// faktisk skal holde: proben bygger klienten ud fra RUTEN — model, endpoint og
// domaene-prompt — og ikke ud fra konstanter. Ellers ville en ny STT-udbyder
// blive testet mod den gamle udbyders endpoint, og "Test forbindelsen" ville
// svare grønt paa noget andet end det brugeren koerer paa.
const sttMocks = vi.hoisted(() => ({ createOpenAiSttClient: vi.fn() }));

vi.mock("./stt", () => ({
  createOpenAiSttClient: sttMocks.createOpenAiSttClient,
  STT_DOMAIN_PROMPT: "ORDLISTE",
}));
vi.mock("../assets/stt-probe-da.pcm?url", () => ({ default: "blob:probe" }));

const routes = (over: Record<string, unknown> = {}) =>
  ({
    stt: {
      slug: "openai",
      label: "OpenAI",
      endpoint: "wss://api.openai.com/v1/realtime?intent=transcription",
      model: "gpt-transcribe",
      supports_partials: true,
      supports_domain_prompt: true,
      language_field: "plural",
      supports_keywords: true,
      ...over,
    },
    routing: {
      slug: "vercel",
      label: "Vercel",
      model: "google/gemini-3.1-flash-lite",
    },
  }) as unknown as Parameters<typeof probeSttRoute>[0];

describe("probeSttRoute", () => {
  beforeEach(() => {
    sttMocks.createOpenAiSttClient.mockReset().mockReturnValue({
      start: vi.fn(async () => {}),
      pushAudio: vi.fn(),
      onPartial: vi.fn(),
      stop: vi.fn(async () => "Luk kort to og tre."),
      abort: vi.fn(),
    });
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => ({
        arrayBuffer: async () => new Uint8Array([1, 2]).buffer,
      })),
    );
  });

  it("bygger klienten ud fra rutens egne felter", async () => {
    await expect(probeSttRoute(routes())).resolves.toBe("Luk kort to og tre.");
    expect(sttMocks.createOpenAiSttClient).toHaveBeenCalledWith({
      model: "gpt-transcribe",
      endpoint: "wss://api.openai.com/v1/realtime?intent=transcription",
      // Dialekten skal med HERFRA og ikke fra en default i stt.ts: proben er
      // det brugeren trykker paa for at faa vished, saa den skal sende
      // praecis det ruten foreskriver.
      languageField: "plural",
      // Proben kaldes uden keywords her, saa feltet skal vaere tomt — ikke
      // fravaerende. Testen nedenfor maaler at brugerens liste NAAR frem.
      keywords: [],
      prompt: "ORDLISTE",
    });
  });

  it("udelader domaene-prompten naar ruten ikke understoetter den", async () => {
    await probeSttRoute(routes({ supports_domain_prompt: false }));
    expect(sttMocks.createOpenAiSttClient).toHaveBeenCalledWith(
      expect.objectContaining({ prompt: null }),
    );
  });

  // Uden denne test kunne kaldestedet i Settings.tsx droppe keyword-argumentet
  // og ALT ville staa groent — mens "Test forbindelsen" svarede groent paa en
  // konfiguration brugeren aldrig taler paa.
  it("sender brugerens keywords videre naar ruten kan tage dem", async () => {
    await probeSttRoute(routes(), ["TalminalMCP", "Codex"]);
    expect(sttMocks.createOpenAiSttClient).toHaveBeenCalledWith(
      expect.objectContaining({ keywords: ["TalminalMCP", "Codex"] }),
    );
  });

  it("udelader keywords naar ruten ikke kan tage dem", async () => {
    await probeSttRoute(routes({ supports_keywords: false }), ["TalminalMCP"]);
    expect(sttMocks.createOpenAiSttClient).toHaveBeenCalledWith(
      expect.objectContaining({ keywords: [] }),
    );
  });

  it("afbryder klienten hvis transskriptionen fejler", async () => {
    const abort = vi.fn();
    sttMocks.createOpenAiSttClient.mockReturnValue({
      start: vi.fn(async () => {}),
      pushAudio: vi.fn(),
      onPartial: vi.fn(),
      stop: vi.fn(async () => {
        throw new Error("ingen lyd");
      }),
      abort,
    });
    await expect(probeSttRoute(routes())).rejects.toThrow("ingen lyd");
    expect(abort).toHaveBeenCalledTimes(1);
  });
});
