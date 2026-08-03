/**
 * Base64 mellem wiren og bytes — ét sted.
 *
 * Fire moduler havde hver sin kopi (`Card.tsx`, `voice/stt.ts`, `voice/tts.ts`,
 * `voice/realtime.ts`), og kopierne var allerede drevet fra hinanden: STT'ens
 * encoder chunker, realtime'ens gjorde ikke. Forskellen er ikke kosmetisk —
 * `String.fromCharCode(...bytes)` har en argument-graense, saa den uchunkede
 * form kaster `RangeError` paa store buffere. Her staar den chunkede.
 */

/** Argument-graensen for `String.fromCharCode(...spread)` ligger et godt
 *  stykke over dette, men 32 KiB er den vaerdi mikrofonvejen har koert paa. */
const STRIDE = 0x8000;

export function bytesToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += STRIDE) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + STRIDE));
  }
  return btoa(binary);
}

export function base64ToBytes(value: string): Uint8Array<ArrayBuffer> {
  const binary = atob(value);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

export function base64ToBuffer(value: string): ArrayBuffer {
  return base64ToBytes(value).buffer;
}
