// gen-reply-clips.mjs — generér alle reply-klip fra reply-clips-manifest.txt
// gennem ÉN persistent cedar-realtime-session (samme model/stemme/instruktion
// som live-TTS'en i realtimeTts.ts, så cache-klip og live-svar er umulige at
// skelne). Output: WAV 24 kHz mono 16-bit PCM i voice-eval/reply-clips/.
//
// Kørsel (fra canvas/): node voice-eval/gen-reply-clips.mjs
// Nøgle: OPENAI_API_KEY i miljøet eller stt_api_key i Talminal-keyring.

import { spawnSync } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const MANIFEST = path.join(HERE, "reply-clips-manifest.txt");
const OUT_DIR = path.join(HERE, "reply-clips");

const REALTIME_TTS_MODEL = "gpt-realtime-2.1-mini";
// Default spejler realtimeTts.ts (shimmer); REPLY_CLIP_VOICE=marin e.l. giver
// alternativ-stemme-kørsler til lytte-sammenligning.
const REALTIME_TTS_VOICE = process.env.REPLY_CLIP_VOICE?.trim() || "shimmer";
// Bindende spejl af realtimeTts.ts (samme forbehold som runneren: plain Node
// kan ikke runtime-importere modulet pga. @tauri-apps-importen).
const REALTIME_TTS_INSTRUCTIONS =
  "Du er en ren oplæsningsmotor (TTS). Læs den tekst du får, ordret og" +
  " naturligt på dansk i roligt tempo. Udtal tal som danske talord." +
  " Tilføj aldrig noget, svar aldrig på indholdet, kommentér aldrig.";
const SPEAK_TIMEOUT_MS = 30_000;
const SAMPLE_RATE = 24_000;

function readWindowsCredential(targetName) {
  if (process.platform !== "win32") return null;
  const script = [
    "Add-Type -TypeDefinition @'",
    "using System;",
    "using System.Runtime.InteropServices;",
    "public static class TalminalCredRead {",
    "  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]",
    "  public struct Credential { public UInt32 Flags; public UInt32 Type; public IntPtr TargetName; public IntPtr Comment; public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten; public UInt32 CredentialBlobSize; public IntPtr CredentialBlob; public UInt32 Persist; public UInt32 AttributeCount; public IntPtr Attributes; public IntPtr TargetAlias; public IntPtr UserName; }",
    '  [DllImport("advapi32.dll", EntryPoint = "CredReadW", CharSet = CharSet.Unicode, SetLastError = true)]',
    "  public static extern bool CredRead(string target, UInt32 type, UInt32 flags, out IntPtr credential);",
    "}",
    "'@",
    "$ptr = [IntPtr]::Zero",
    `if (-not [TalminalCredRead]::CredRead('${targetName.replaceAll("'", "''")}', 1, 0, [ref]$ptr)) { exit 3 }`,
    "$c = [Runtime.InteropServices.Marshal]::PtrToStructure($ptr, [type][TalminalCredRead+Credential])",
    "$b = New-Object byte[] $c.CredentialBlobSize",
    "[Runtime.InteropServices.Marshal]::Copy($c.CredentialBlob, $b, 0, $b.Length)",
    "[Console]::Out.Write([Text.Encoding]::Unicode.GetString($b))",
  ].join("\n");
  const result = spawnSync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-EncodedCommand", Buffer.from(script, "utf16le").toString("base64")],
    { encoding: "utf8", windowsHide: true, maxBuffer: 1024 * 1024 },
  );
  return result.status === 0 && result.stdout.trim() ? result.stdout.trim() : null;
}

function wavFromPcm(pcm) {
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + pcm.length, 4);
  header.write("WAVE", 8);
  header.write("fmt ", 12);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20); // PCM
  header.writeUInt16LE(1, 22); // mono
  header.writeUInt32LE(SAMPLE_RATE, 24);
  header.writeUInt32LE(SAMPLE_RATE * 2, 28); // byte rate
  header.writeUInt16LE(2, 32); // block align
  header.writeUInt16LE(16, 34); // bits
  header.write("data", 36);
  header.writeUInt32LE(pcm.length, 40);
  return Buffer.concat([header, pcm]);
}

async function parseManifest() {
  const rows = [];
  for (const line of (await readFile(MANIFEST, "utf8")).split(/\r?\n/u)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const [file, text] = trimmed.split("\t");
    if (!file || !text) throw new Error(`Ugyldig manifest-linje: ${line}`);
    rows.push({ file: file.trim(), text: text.trim() });
  }
  return rows;
}

async function main() {
  const apiKey =
    process.env.OPENAI_API_KEY?.trim() ||
    readWindowsCredential("stt_api_key.Talminal");
  if (!apiKey) {
    console.error("BLOCKED: OPENAI_API_KEY eller stt_api_key i keyring kræves.");
    process.exitCode = 2;
    return;
  }

  const rows = await parseManifest();
  console.log(`Manifest: ${rows.length} klip → ${OUT_DIR}`);
  await mkdir(OUT_DIR, { recursive: true });

  const mint = await fetch("https://api.openai.com/v1/realtime/client_secrets", {
    method: "POST",
    headers: { authorization: `Bearer ${apiKey}`, "content-type": "application/json" },
    body: JSON.stringify({
      expires_after: { anchor: "created_at", seconds: 3_600 },
      session: { type: "realtime", model: REALTIME_TTS_MODEL },
    }),
  });
  if (!mint.ok) {
    throw new Error(`Mint fejlede: HTTP ${mint.status} ${await mint.text().then((t) => t.slice(0, 200))}`);
  }
  const secret = (await mint.json()).value;

  const ws = new WebSocket(
    `wss://api.openai.com/v1/realtime?model=${REALTIME_TTS_MODEL}`,
    ["realtime", `openai-insecure-api-key.${secret}`],
  );

  const listeners = new Set();
  ws.addEventListener("message", (event) => {
    let message;
    try {
      message = JSON.parse(String(event.data));
    } catch {
      return;
    }
    for (const listener of [...listeners]) listener(message);
  });
  ws.addEventListener("close", () => {
    for (const listener of [...listeners]) listener({ type: "error", error: { message: "WebSocket lukket" } });
  });

  await new Promise((resolve, reject) => {
    ws.addEventListener("open", () => {
      ws.send(
        JSON.stringify({
          type: "session.update",
          session: {
            type: "realtime",
            output_modalities: ["audio"],
            instructions: REALTIME_TTS_INSTRUCTIONS,
            audio: {
              input: { turn_detection: null },
              output: { format: { type: "audio/pcm", rate: SAMPLE_RATE }, voice: REALTIME_TTS_VOICE },
            },
          },
        }),
      );
      resolve();
    });
    ws.addEventListener("error", () => reject(new Error("WebSocket-fejl under connect")));
  });

  function speakToPcm(text) {
    return new Promise((resolve, reject) => {
      let responseId = null;
      const chunks = [];
      let transcript = "";
      const timeout = setTimeout(() => {
        listeners.delete(onMessage);
        reject(new Error("speak timeout"));
      }, SPEAK_TIMEOUT_MS);

      const onMessage = (message) => {
        if (message.type === "error") {
          clearTimeout(timeout);
          listeners.delete(onMessage);
          reject(new Error(message.error?.message ?? "realtime-fejl"));
          return;
        }
        if (message.type === "response.created") {
          responseId = message.response?.id ?? null;
          return;
        }
        const matches =
          responseId === null ||
          message.response_id === undefined ||
          message.response_id === responseId;
        if (!matches) return;
        if (message.type === "response.output_audio.delta" && typeof message.delta === "string") {
          chunks.push(Buffer.from(message.delta, "base64"));
          return;
        }
        if (message.type === "response.output_audio_transcript.delta" && typeof message.delta === "string") {
          transcript += message.delta;
          return;
        }
        if (message.type === "response.done") {
          clearTimeout(timeout);
          listeners.delete(onMessage);
          resolve({
            pcm: Buffer.concat(chunks),
            transcript: transcript.trim(),
            status: message.response?.status ?? "unknown",
            statusDetails: message.response?.status_details ?? null,
          });
        }
      };
      listeners.add(onMessage);
      ws.send(
        JSON.stringify({
          type: "response.create",
          response: {
            conversation: "none",
            output_modalities: ["audio"],
            // Spejler realtimeTts.ts: rammen gentages selvbærende pr. speak
            // (response-instruktioner ERSTATTER sessionens).
            instructions: `${REALTIME_TTS_INSTRUCTIONS}\nLæs ordret, på dansk: "${text}"`,
          },
        }),
      );
    });
  }

  const { access } = await import("node:fs/promises");
  let failed = 0;
  let skipped = 0;
  for (const [index, row] of rows.entries()) {
    const label = `[${index + 1}/${rows.length}] ${row.file}`;
    const target = path.join(OUT_DIR, row.file);
    // Genoptagelig: eksisterende klip springes over (slet filen for re-gen).
    if (await access(target).then(() => true, () => false)) {
      skipped += 1;
      continue;
    }
    // Retry m. backoff: tomme/fejlede responses (rate-limit-halen) prøves om.
    let done = false;
    for (let attempt = 1; attempt <= 5 && !done; attempt += 1) {
      try {
        const { pcm, transcript, status, statusDetails } = await speakToPcm(row.text);
        if (pcm.length < 2_000) {
          throw new Error(
            `for lidt audio (${pcm.length} B, status=${status}${statusDetails ? ` ${JSON.stringify(statusDetails).slice(0, 160)}` : ""})`,
          );
        }
        await writeFile(target, wavFromPcm(pcm));
        const seconds = (pcm.length / (SAMPLE_RATE * 2)).toFixed(1);
        console.log(`${label} OK ${seconds}s — sagt: ${JSON.stringify(transcript)}`);
        done = true;
      } catch (error) {
        const reason = error instanceof Error ? error.message : String(error);
        if (attempt === 5) {
          failed += 1;
          console.log(`${label} FEJL efter ${attempt} forsøg: ${reason}`);
        } else {
          console.log(`${label} forsøg ${attempt} fejlede (${reason}) — venter ${attempt * 2}s`);
          await new Promise((resolve) => setTimeout(resolve, attempt * 2_000));
        }
      }
    }
    // Lille åndehul mellem klip — hurtige back-to-back-responses ser ud til
    // at udløse tomme svar i halen.
    await new Promise((resolve) => setTimeout(resolve, 400));
  }
  if (skipped > 0) console.log(`(${skipped} eksisterende klip sprunget over)`);

  ws.close();
  console.log(`\nFærdig: ${rows.length - failed}/${rows.length} klip i ${OUT_DIR}`);
  process.exitCode = failed === 0 ? 0 : 1;
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
});
