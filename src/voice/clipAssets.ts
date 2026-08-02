// Klip-loader (v4-spec §6.6): henter reply-klippene fra public/reply-clips/
// (WAV 24 kHz mono 16-bit) og afleverer RÅ PCM-buffers — WAV-headerens
// 44 bytes strippes her, ellers afspiller PcmPlayer headeren som klik.
import { CLIP_TEXTS, type ClipKey } from "./replies";

export const WAV_HEADER_BYTES = 44;

export async function loadReplyClips(
  fetcher: typeof fetch = fetch,
): Promise<Map<string, ArrayBuffer>> {
  const keys = Object.keys(CLIP_TEXTS) as ClipKey[];
  const loaded = await Promise.all(
    keys.map(async (key) => {
      try {
        const response = await fetcher(`/reply-clips/${key}.wav`);
        if (!response.ok) return null;
        const wav = await response.arrayBuffer();
        if (wav.byteLength <= WAV_HEADER_BYTES) return null;
        return [key, wav.slice(WAV_HEADER_BYTES)] as const;
      } catch {
        return null;
      }
    }),
  );
  const assets = new Map<string, ArrayBuffer>();
  for (const entry of loaded) {
    if (entry) assets.set(entry[0], entry[1]);
  }
  const missing = keys.filter((key) => !assets.has(key));
  if (missing.length > 0) console.warn("voice.clips.missing", missing);
  return assets;
}
