import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PcmPlayer } from "./audioPlayer";
import { deferred } from "../testHelpers";
import {
  TTS_INSTRUCTIONS,
  TTS_MODEL,
  TTS_VOICE,
  createOpenAiTts,
} from "./tts";

const { invokeMock, FakeChannel } = vi.hoisted(() => {
  class FakeChannel<T> {
    onmessage: (message: T) => void = () => {};
  }
  return { invokeMock: vi.fn(), FakeChannel };
});

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
  Channel: FakeChannel,
}));


function pcmBase64(bytes = [0, 0]): string {
  return btoa(String.fromCharCode(...bytes));
}

function chunkTransport(chunks: string[]) {
  return vi.fn(
    async (_body: string, onChunk: (base64Chunk: string) => void) => {
      for (const chunk of chunks) onChunk(chunk);
    },
  );
}

function fakePlayer(overrides: Partial<PcmPlayer> = {}): PcmPlayer {
  return {
    enqueue: vi.fn(async () => undefined),
    drain: vi.fn(async () => undefined),
    close: vi.fn(async () => undefined),
    getOutputLevel: vi.fn(() => 0),
    ...overrides,
  };
}

describe("createOpenAiTts", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        throw new Error("TTS tests must not call the network");
      }),
    );
  });

  it("cache_hit_skips_network", async () => {
    const clip = new Uint8Array([1, 0, 2, 0]).buffer;
    const transport = chunkTransport([pcmBase64()]);
    const player = fakePlayer();
    const tts = createOpenAiTts({
      transport,
      player,
      assets: new Map([["reply-ready", clip]]),
    });

    const playback = tts.speak({ text: "Klar.", audioKey: "reply-ready" });
    await playback.firstAudio;
    await playback.done;

    expect(transport).not.toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalled();
    expect(player.enqueue).toHaveBeenCalledOnce();
    expect(player.enqueue).toHaveBeenCalledWith(clip);
  });

  it("posts_binding_tts_body_via_streaming_channel", async () => {
    invokeMock.mockImplementation(
      async (_command: string, args: { onChunk: InstanceType<typeof FakeChannel<string>> }) => {
        args.onChunk.onmessage(pcmBase64([1, 0, 254, 255]));
      },
    );
    const player = fakePlayer();
    const tts = createOpenAiTts({ player });

    await tts.speak({ text: "Kort tre er klar.", audioKey: null }).done;

    expect(invokeMock).toHaveBeenCalledOnce();
    expect(invokeMock).toHaveBeenCalledWith("tts_speech_stream", {
      body: expect.any(String),
      onChunk: expect.any(FakeChannel),
    });
    const { body } = invokeMock.mock.calls[0][1] as { body: string };
    expect(JSON.parse(body)).toEqual({
      model: TTS_MODEL,
      voice: TTS_VOICE,
      input: "Kort tre er klar.",
      response_format: "pcm",
      instructions: TTS_INSTRUCTIONS,
    });
    const chunk = vi.mocked(player.enqueue).mock.calls[0][0];
    expect([...new Uint8Array(chunk)]).toEqual([1, 0, 254, 255]);
  });

  it("decodes_chunks_and_enqueues_in_arrival_order", async () => {
    const player = fakePlayer();
    const tts = createOpenAiTts({
      transport: chunkTransport([
        pcmBase64([1, 0]),
        pcmBase64([254, 255]),
      ]),
      player,
    });

    await tts.speak({ text: "Klar.", audioKey: null }).done;

    const calls = vi.mocked(player.enqueue).mock.calls;
    expect(calls).toHaveLength(2);
    expect([...new Uint8Array(calls[0][0])]).toEqual([1, 0]);
    expect([...new Uint8Array(calls[1][0])]).toEqual([254, 255]);
    expect(player.drain).toHaveBeenCalledOnce();
  });

  it("first_audio_resolves_on_first_chunk_while_stream_is_still_open", async () => {
    const streamEnd = deferred<void>();
    let emitChunk!: (base64Chunk: string) => void;
    const transport = vi.fn(
      (_body: string, onChunk: (base64Chunk: string) => void) => {
        emitChunk = onChunk;
        return streamEnd.promise;
      },
    );
    const player = fakePlayer();
    const tts = createOpenAiTts({ transport, player });

    const playback = tts.speak({ text: "Klar.", audioKey: null });
    let firstAudioResolved = false;
    void playback.firstAudio.then(() => {
      firstAudioResolved = true;
    });
    await Promise.resolve();
    expect(firstAudioResolved).toBe(false);

    emitChunk(pcmBase64());
    await playback.firstAudio;
    expect(firstAudioResolved).toBe(true);
    expect(player.enqueue).toHaveBeenCalledOnce();
    expect(player.drain).not.toHaveBeenCalled();

    streamEnd.resolve();
    await playback.done;
    expect(player.drain).toHaveBeenCalledOnce();
  });

  it("stop_mid_stream_ignores_later_chunks", async () => {
    const streamEnd = deferred<void>();
    let emitChunk!: (base64Chunk: string) => void;
    const transport = vi.fn(
      (_body: string, onChunk: (base64Chunk: string) => void) => {
        emitChunk = onChunk;
        return streamEnd.promise;
      },
    );
    const player = fakePlayer();
    const tts = createOpenAiTts({ transport, player });

    const playback = tts.speak({ text: "Klar.", audioKey: null });
    await Promise.resolve();
    emitChunk(pcmBase64());
    await playback.firstAudio;

    playback.stop();
    emitChunk(pcmBase64([9, 9]));
    streamEnd.resolve();

    await expect(playback.done).resolves.toBeUndefined();
    expect(player.enqueue).toHaveBeenCalledOnce();
    expect(player.close).toHaveBeenCalledOnce();
  });

  it("stop_halts_playback_and_resolves_done", async () => {
    const drain = deferred<void>();
    const player = fakePlayer({
      drain: vi.fn(() => drain.promise),
    });
    const tts = createOpenAiTts({
      transport: chunkTransport([pcmBase64()]),
      player,
    });

    const playback = tts.speak({ text: "Klar.", audioKey: null });
    await playback.firstAudio;
    playback.stop();

    await expect(playback.done).resolves.toBeUndefined();
    expect(player.close).toHaveBeenCalledOnce();
  });

  it("stop_before_first_chunk_rejects_first_audio", async () => {
    const streamEnd = deferred<void>();
    let emitChunk!: (base64Chunk: string) => void;
    const transport = vi.fn(
      (_body: string, onChunk: (base64Chunk: string) => void) => {
        emitChunk = onChunk;
        return streamEnd.promise;
      },
    );
    const player = fakePlayer();
    const tts = createOpenAiTts({ transport, player });

    const playback = tts.speak({ text: "Klar.", audioKey: null });
    await Promise.resolve();
    const firstAudio = expect(playback.firstAudio).rejects.toThrow(
      "TTS-afspilning blev stoppet",
    );
    playback.stop();

    await firstAudio;
    await expect(playback.done).resolves.toBeUndefined();
    expect(player.close).toHaveBeenCalledOnce();
    emitChunk(pcmBase64());
    streamEnd.resolve();
    await Promise.resolve();
    expect(player.enqueue).not.toHaveBeenCalled();
  });

  it("transport_error_rejects_first_audio_and_done", async () => {
    const error = new Error("proxy unavailable");
    const tts = createOpenAiTts({
      transport: vi.fn(async () => {
        throw error;
      }),
      player: fakePlayer(),
    });

    const playback = tts.speak({ text: "Klar.", audioKey: null });

    await Promise.all([
      expect(playback.firstAudio).rejects.toBe(error),
      expect(playback.done).rejects.toBe(error),
    ]);
  });

  it("empty_text_is_noop", async () => {
    const transport = chunkTransport([pcmBase64()]);
    const player = fakePlayer();
    const tts = createOpenAiTts({ transport, player });

    const playback = tts.speak({ text: "   ", audioKey: "missing" });

    await expect(playback.firstAudio).resolves.toBeUndefined();
    await expect(playback.done).resolves.toBeUndefined();
    expect(transport).not.toHaveBeenCalled();
    expect(player.enqueue).not.toHaveBeenCalled();
    expect(player.drain).not.toHaveBeenCalled();
    expect(player.close).not.toHaveBeenCalled();
  });
});
