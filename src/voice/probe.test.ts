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
      model: "gpt-4o-transcribe",
      supports_partials: true,
      supports_domain_prompt: true,
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
      model: "gpt-4o-transcribe",
      endpoint: "wss://api.openai.com/v1/realtime?intent=transcription",
      prompt: "ORDLISTE",
    });
  });

  it("udelader domaene-prompten naar ruten ikke understoetter den", async () => {
    await probeSttRoute(routes({ supports_domain_prompt: false }));
    expect(sttMocks.createOpenAiSttClient).toHaveBeenCalledWith(
      expect.objectContaining({ prompt: null }),
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
