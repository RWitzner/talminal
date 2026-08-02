/**
 * @vitest-environment happy-dom
 */
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
import { createPtt, type AudioCapture, type PttState } from "./ptt";
import {
  createOpenAiSttClient,
  STT_DOMAIN_PROMPT,
} from "./stt";

function createTestStt(
  options: Partial<Parameters<typeof createOpenAiSttClient>[0]> = {},
) {
  return createOpenAiSttClient({
    model: "gpt-4o-transcribe",
    endpoint: "wss://api.openai.com/v1/realtime?intent=transcription",
    ...options,
  });
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

async function flush() {
  await new Promise<void>((resolve) => setTimeout(resolve, 0));
}

function makeCapture(): AudioCapture & { stop: ReturnType<typeof vi.fn> } {
  return {
    stop: vi.fn(async () => undefined),
  };
}

class FakeWebSocket {
  static readonly OPEN = 1;
  static readonly CLOSED = 3;
  static instances: FakeWebSocket[] = [];
  readonly sent: string[] = [];
  readyState = FakeWebSocket.OPEN;
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  closed = false;

  constructor(
    readonly url: string,
    readonly protocols?: string | string[],
  ) {
    FakeWebSocket.instances.push(this);
  }

  send(data: string) {
    this.sent.push(data);
  }

  close() {
    this.closed = true;
    this.readyState = FakeWebSocket.CLOSED;
  }

  open() {
    this.onopen?.(new Event("open"));
  }

  message(data: unknown) {
    this.onmessage?.({ data: JSON.stringify(data) } as MessageEvent);
  }

  disconnect() {
    this.readyState = FakeWebSocket.CLOSED;
    this.onclose?.(new Event("close") as CloseEvent);
  }
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  FakeWebSocket.instances = [];
});

describe("createOpenAiSttClient", () => {
  it("afviser samtidige start-kald mens secret stadig indlæses", async () => {
    const secret = deferred<unknown>();
    vi.mocked(invoke).mockReturnValue(secret.promise);
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = createTestStt();

    const firstStart = client.start();
    await expect(client.start()).rejects.toThrow("already started");

    secret.resolve({ value: "sk-test-key", expires_at: 123 });
    await flush();
    expect(FakeWebSocket.instances).toHaveLength(1);
    FakeWebSocket.instances[0].open();
    await expect(firstStart).resolves.toBeUndefined();
  });

  it("åbner GA transcription-session med browser-kompatibel subprotocol-auth og dansk hint", async () => {
    vi.mocked(invoke).mockResolvedValue({ value: "sk-test-key", expires_at: 123 });
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = createTestStt();

    const started = client.start();
    await flush();
    const ws = FakeWebSocket.instances[0];
    expect(invoke).toHaveBeenCalledWith("mint_transcription_secret");
    expect(ws.url).toBe("wss://api.openai.com/v1/realtime?intent=transcription");
    expect(ws.protocols).toEqual(["realtime", "openai-insecure-api-key.sk-test-key"]);

    ws.open();
    await started;
    expect(JSON.parse(ws.sent[0])).toEqual({
      type: "session.update",
      session: {
        type: "transcription",
        audio: {
          input: {
            format: { type: "audio/pcm", rate: 24000 },
            transcription: {
              model: "gpt-4o-transcribe",
              language: "da",
              prompt: STT_DOMAIN_PROMPT,
            },
            turn_detection: null,
          },
        },
      },
    });
  });

  it("stop_commits_and_returns_final_transcript", async () => {
    vi.mocked(invoke).mockResolvedValue({ value: "sk-test-key", expires_at: 123 });
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const partials: string[] = [];
    const client = createTestStt();
    client.onPartial((text) => partials.push(text));

    const started = client.start();
    await flush();
    const ws = FakeWebSocket.instances[0];
    ws.open();
    await started;

    client.pushAudio(new Uint8Array([0, 1, 2, 255]).buffer);
    expect(JSON.parse(ws.sent[1])).toEqual({
      type: "input_audio_buffer.append",
      audio: "AAEC/w==",
    });

    ws.message({
      type: "conversation.item.input_audio_transcription.delta",
      delta: "luk kort",
    });
    expect(partials).toEqual(["luk kort"]);

    const stopped = client.stop();
    expect(JSON.parse(ws.sent[2])).toEqual({ type: "input_audio_buffer.commit" });
    ws.message({
      type: "conversation.item.input_audio_transcription.completed",
      transcript: "luk kort tre",
    });

    await expect(stopped).resolves.toBe("luk kort tre");
    expect(ws.closed).toBe(true);
  });

  it("afviser og rydder session efter 10 sekunders final-timeout", async () => {
    vi.useFakeTimers();
    vi.mocked(invoke).mockResolvedValue({ value: "sk-test-key", expires_at: 123 });
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = createTestStt();

    const started = client.start();
    await vi.advanceTimersByTimeAsync(0);
    const ws = FakeWebSocket.instances[0];
    ws.open();
    await started;

    const stopped = client.stop();
    const rejection = expect(stopped).rejects.toThrow(
      "Timed out waiting for final transcript after 10000 ms",
    );
    await vi.advanceTimersByTimeAsync(10_000);

    await rejection;
    expect(ws.closed).toBe(true);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("PTT går finalizing → idle uden final ved STT final-timeout", async () => {
    vi.useFakeTimers();
    vi.mocked(invoke).mockResolvedValue({ value: "sk-test-key", expires_at: 123 });
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const states: PttState[] = [];
    const finals: string[] = [];
    const errors: Error[] = [];
    const ptt = createPtt({
      stt: createTestStt(),
      onState: (state) => states.push(state),
      onFinal: (text) => finals.push(text),
      onError: (error) => errors.push(error),
      startCapture: vi.fn(async () => makeCapture()),
    });

    ptt.press();
    await vi.advanceTimersByTimeAsync(0);
    FakeWebSocket.instances[0].open();
    await vi.advanceTimersByTimeAsync(0);
    ptt.release();
    await vi.advanceTimersByTimeAsync(10_000);

    expect(states).toEqual(["listening", "finalizing", "idle"]);
    expect(finals).toEqual([]);
    expect(errors.map((error) => error.message)).toEqual([
      "Timed out waiting for final transcript after 10000 ms",
    ]);
  });

  it("afviser stop på OpenAI error-event og lukker socket", async () => {
    vi.mocked(invoke).mockResolvedValue({ value: "sk-test-key", expires_at: 123 });
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = createTestStt();

    const started = client.start();
    await flush();
    const ws = FakeWebSocket.instances[0];
    ws.open();
    await started;

    const stopped = client.stop();
    ws.message({ type: "error", error: { message: "bad audio" } });

    await expect(stopped).rejects.toThrow("bad audio");
    expect(ws.closed).toBe(true);
  });

  it("afviser start hvis socket lukker før open", async () => {
    vi.mocked(invoke).mockResolvedValue({ value: "sk-test-key", expires_at: 123 });
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = createTestStt();

    const started = client.start();
    await flush();
    FakeWebSocket.instances[0].disconnect();

    await expect(started).rejects.toThrow("closed before opening");
  });

  it("kan starte en ny session efter uventet disconnect", async () => {
    vi.mocked(invoke).mockResolvedValue({ value: "sk-test-key", expires_at: 123 });
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = createTestStt();

    const firstStart = client.start();
    await flush();
    FakeWebSocket.instances[0].open();
    await firstStart;
    FakeWebSocket.instances[0].disconnect();

    const secondStart = client.start();
    await flush();
    expect(FakeWebSocket.instances).toHaveLength(2);
    FakeWebSocket.instances[1].open();
    await expect(secondStart).resolves.toBeUndefined();
  });

  it("ignorerer forsinket close-event fra en afsluttet tidligere session", async () => {
    vi.mocked(invoke).mockResolvedValue({ value: "sk-test-key", expires_at: 123 });
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = createTestStt();

    const firstStart = client.start();
    await flush();
    const first = FakeWebSocket.instances[0];
    first.open();
    await firstStart;
    const firstStop = client.stop();
    first.message({
      type: "conversation.item.input_audio_transcription.completed",
      transcript: "første",
    });
    await firstStop;

    const secondStart = client.start();
    await flush();
    const second = FakeWebSocket.instances[1];
    second.open();
    await secondStart;

    first.onclose?.(new Event("close") as CloseEvent);
    expect(() => client.pushAudio(new Uint8Array([1, 2]).buffer)).not.toThrow();
    expect(JSON.parse(second.sent.at(-1)!)).toEqual({
      type: "input_audio_buffer.append",
      audio: "AQI=",
    });
  });

  it("ignorerer forsinkede message/error-events fra en tidligere session", async () => {
    vi.mocked(invoke).mockResolvedValue({ value: "sk-test-key", expires_at: 123 });
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const partials: string[] = [];
    const client = createTestStt();
    client.onPartial((text) => partials.push(text));

    const firstStart = client.start();
    await flush();
    const first = FakeWebSocket.instances[0];
    first.open();
    await firstStart;
    const firstStop = client.stop();
    first.message({
      type: "conversation.item.input_audio_transcription.completed",
      transcript: "første",
    });
    await firstStop;

    const secondStart = client.start();
    await flush();
    const second = FakeWebSocket.instances[1];
    second.open();
    await secondStart;

    first.message({
      type: "conversation.item.input_audio_transcription.delta",
      delta: "gammel",
    });
    first.message({ type: "error", error: { message: "gammel fejl" } });
    expect(partials).toEqual([]);
    expect(second.closed).toBe(false);

    const secondStop = client.stop();
    second.message({
      type: "conversation.item.input_audio_transcription.completed",
      transcript: "anden",
    });
    await expect(secondStop).resolves.toBe("anden");
  });

  it("stale close/error kan ikke påvirke ny session mens den venter på final", async () => {
    vi.mocked(invoke).mockResolvedValue({ value: "sk-test-key", expires_at: 123 });
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = createTestStt();

    const firstStart = client.start();
    await flush();
    const first = FakeWebSocket.instances[0];
    first.open();
    await firstStart;
    const firstStop = client.stop();
    first.message({
      type: "conversation.item.input_audio_transcription.completed",
      transcript: "første",
    });
    await firstStop;

    const secondStart = client.start();
    await flush();
    const second = FakeWebSocket.instances[1];
    second.open();
    await secondStart;
    const secondStop = client.stop();

    first.onclose?.(new Event("close") as CloseEvent);
    first.message({ type: "error", error: { message: "stale failure" } });
    expect(second.closed).toBe(false);

    second.message({
      type: "conversation.item.input_audio_transcription.completed",
      transcript: "anden",
    });
    await expect(secondStop).resolves.toBe("anden");
  });

  it("kan genstarte straks efter error uden at vente på close-event", async () => {
    vi.mocked(invoke).mockResolvedValue({ value: "sk-test-key", expires_at: 123 });
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = createTestStt();

    const firstStart = client.start();
    await flush();
    const first = FakeWebSocket.instances[0];
    first.open();
    await firstStart;
    const firstStop = client.stop();
    first.message({ type: "error", error: { message: "transient" } });
    await expect(firstStop).rejects.toThrow("transient");

    const secondStart = client.start();
    await flush();
    expect(FakeWebSocket.instances).toHaveLength(2);
    FakeWebSocket.instances[1].open();
    await expect(secondStart).resolves.toBeUndefined();
  });

  it("kan genstarte når error ankommer før stop kaldes", async () => {
    vi.mocked(invoke).mockResolvedValue({ value: "sk-test-key", expires_at: 123 });
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = createTestStt();

    const firstStart = client.start();
    await flush();
    const first = FakeWebSocket.instances[0];
    first.open();
    await firstStart;
    first.message({ type: "error", error: { message: "transient" } });
    await expect(client.stop()).rejects.toThrow();

    const secondStart = client.start();
    await flush();
    expect(FakeWebSocket.instances).toHaveLength(2);
    FakeWebSocket.instances[1].open();
    await expect(secondStart).resolves.toBeUndefined();
  });
});

describe("STT configuration", () => {
  it("start_mints_transcription_secret_not_load_secret", async () => {
    const socket = new FakeWebSocket("", []);
    FakeWebSocket.instances = [];
    const mintSecret = vi.fn(async () => ({
      value: "ephemeral",
      expires_at: 123,
    }));
    const createSocket = vi.fn(
      () => socket as unknown as WebSocket,
    );
    const client = createTestStt({ mintSecret, createSocket });
    const started = client.start();
    await flush();
    expect(mintSecret).toHaveBeenCalledTimes(1);
    expect(invoke).not.toHaveBeenCalled();
    expect(createSocket).toHaveBeenCalledWith(
      "wss://api.openai.com/v1/realtime?intent=transcription",
      ["realtime", "openai-insecure-api-key.ephemeral"],
    );
    socket.open();
    await expect(started).resolves.toBeUndefined();
  });

  it("model_option_overrides_default", async () => {
    vi.mocked(invoke).mockResolvedValue({ value: "sk-test-key", expires_at: 123 });
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = createTestStt({
      model: "gpt-4o-transcribe",
      prompt: "Eget dom?ne",
    });

    const started = client.start();
    await flush();
    const ws = FakeWebSocket.instances[0];
    ws.open();
    await started;

    expect(JSON.parse(ws.sent[0]).session.audio.input.transcription).toEqual({
      model: "gpt-4o-transcribe",
      language: "da",
      prompt: "Eget dom?ne",
    });
  });

  it("mint_failure_rejects_start", async () => {
    const createSocket = vi.fn();
    const client = createTestStt({
      mintSecret: vi.fn(async () => {
        throw new Error("mint fejlede");
      }),
      createSocket,
    });

    await expect(client.start()).rejects.toThrow("mint fejlede");
    expect(createSocket).not.toHaveBeenCalled();
  });

  it("udelader prompt når ruten ikke understøtter den", async () => {
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = createTestStt({
      model: "openai/gpt-4o-transcribe",
      endpoint: "wss://example.test/v1/realtime",
      prompt: null,
      mintSecret: async () => ({ value: "ek-test", expires_at: 0 }),
    });
    const started = client.start();
    await flush();
    const ws = FakeWebSocket.instances.at(-1)!;
    ws.open();
    await started;

    const transcription = JSON.parse(ws.sent[0]).session.audio.input
      .transcription as Record<string, unknown>;
    expect(transcription).not.toHaveProperty("prompt");
  });

  it("bruger rutens endpoint og model", async () => {
    vi.stubGlobal("WebSocket", FakeWebSocket);
    const client = createTestStt({
      model: "openai/gpt-4o-transcribe",
      endpoint: "wss://example.test/v1/realtime",
      mintSecret: async () => ({ value: "ek-test", expires_at: 0 }),
    });
    const started = client.start();
    await flush();
    const ws = FakeWebSocket.instances.at(-1)!;
    ws.open();
    await started;

    expect(ws.url).toBe("wss://example.test/v1/realtime");
    expect(JSON.parse(ws.sent[0]).session.audio.input.transcription.model).toBe(
      "openai/gpt-4o-transcribe",
    );
  });
});
