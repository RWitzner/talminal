import { describe, expect, it, vi } from "vitest";
import { WAV_HEADER_BYTES, loadReplyClips } from "./clipAssets";
import { createClipTts } from "./clipTts";
import { CLIP_TEXTS } from "./replies";

function fakePlayer() {
  return {
    enqueue: vi.fn(async () => {}),
    drain: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    getOutputLevel: vi.fn(() => 0),
  };
}

describe("loadReplyClips", () => {
  it("henter alle 14 klip og stripper WAV-headeren", async () => {
    const wav = new ArrayBuffer(WAV_HEADER_BYTES + 8);
    const fetcher = vi.fn(async () => ({
      ok: true,
      arrayBuffer: async () => wav,
    })) as unknown as typeof fetch;
    const assets = await loadReplyClips(fetcher);
    expect(assets.size).toBe(Object.keys(CLIP_TEXTS).length);
    expect(fetcher).toHaveBeenCalledWith("/reply-clips/sendt.wav");
    expect(fetcher).toHaveBeenCalledWith("/reply-clips/udfoert.wav");
    expect(assets.get("sendt")?.byteLength).toBe(8); // header strippet
  });
  it("manglende fil giver hul i mappet, ikke exception", async () => {
    const fetcher = vi.fn(async () => ({ ok: false })) as unknown as typeof fetch;
    const assets = await loadReplyClips(fetcher);
    expect(assets.size).toBe(0);
  });
});

describe("createClipTts", () => {
  it("afspiller klippet for audioKey og resolver first/done", async () => {
    const player = fakePlayer();
    const clip = new ArrayBuffer(4);
    const tts = createClipTts({ assets: new Map([["sendt", clip]]), player });
    const playback = tts.speak({ text: "Sendt", audioKey: "sendt" });
    await playback.done;
    expect(player.enqueue).toHaveBeenCalledWith(clip);
    expect(player.drain).toHaveBeenCalled();
  });
  it("audioKey null (dry-run) er stilhed uden player-kald", async () => {
    const player = fakePlayer();
    const tts = createClipTts({ assets: new Map(), player });
    await tts.speak({ text: "Dry-run: …", audioKey: null }).done;
    expect(player.enqueue).not.toHaveBeenCalled();
  });
  it("manglende klip er stilhed (ALDRIG TTS-fallback), done resolver", async () => {
    const player = fakePlayer();
    const tts = createClipTts({ assets: new Map(), player });
    await tts.speak({ text: "Sendt", audioKey: "sendt" }).done;
    expect(player.enqueue).not.toHaveBeenCalled();
  });
});
