// gen-reply-clips-elevenlabs.mjs — generér de 14 reply-klip fra
// reply-clips-manifest.txt via ElevenLabs TTS (kommandosæt v4).
// Output: WAV 24 kHz mono 16-bit PCM i canvas/public/reply-clips/
// (mappen klip-loaderen i src/voice/clipAssets.ts serverer fra).
//
// Kørsel (fra canvas/): node voice-eval/gen-reply-clips-elevenlabs.mjs
// Stemmevalg LÅST ved ejer-audition 2026-07-20: Jane (RILOU7YmBhvwJGDGjNmP)
// på eleven_turbo_v2_5 m. language_code=da — v3 vakler på korte sætninger
// (bygget til 250+ tegn), og uden language_code får korte danske sætninger
// ENGELSK accent (for lidt kontekst til sprog-detektion). language_code er
// derfor OBLIGATORISK og hardcodet; previous_text er unsupported på v3.
// Nøgle: ELEVENLABS_API_KEY i miljøet (sat som user-env-var 2026-07-20)
// eller elevenlabs_api_key i Talminal-keyring.
// NB: output_format=pcm_24000 kræver PCM-adgang — verificeret OK på
// en Creator-tier-konto (verificeret 2026-07-20).

import { spawnSync } from "node:child_process";
import { access, mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const MANIFEST = path.join(HERE, "reply-clips-manifest.txt");
const OUT_DIR = path.join(HERE, "..", "public", "reply-clips");
const SAMPLE_RATE = 24_000;
const MODEL_ID = process.env.ELEVENLABS_MODEL_ID?.trim() || "eleven_turbo_v2_5";
const DEFAULT_VOICE_ID = "RILOU7YmBhvwJGDGjNmP"; // Jane — ejer-audition 2026-07-20
const LANGUAGE_CODE = "da"; // obligatorisk — se header-kommentaren
const STT_PROBE_TEXT = "Luk kort to og tre.";
const STT_PROBE_TARGET = path.join(
  HERE,
  "..",
  "src",
  "assets",
  "stt-probe-da.pcm",
);

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

async function speakToPcm(apiKey, voiceId, text) {
  const response = await fetch(
    `https://api.elevenlabs.io/v1/text-to-speech/${voiceId}?output_format=pcm_24000`,
    {
      method: "POST",
      headers: { "xi-api-key": apiKey, "content-type": "application/json" },
      body: JSON.stringify({ text, model_id: MODEL_ID, language_code: LANGUAGE_CODE }),
    },
  );
  if (!response.ok) {
    throw new Error(
      `ElevenLabs HTTP ${response.status}: ${(await response.text()).slice(0, 200)}`,
    );
  }
  return Buffer.from(await response.arrayBuffer());
}

async function main() {
  const apiKey =
    process.env.ELEVENLABS_API_KEY?.trim() ||
    readWindowsCredential("elevenlabs_api_key.Talminal");
  if (!apiKey) {
    console.error("BLOCKED: ELEVENLABS_API_KEY eller elevenlabs_api_key i keyring kræves.");
    process.exitCode = 2;
    return;
  }
  const voiceId = process.env.ELEVENLABS_VOICE_ID?.trim() || DEFAULT_VOICE_ID;
  if (process.argv.includes("--stt-probe")) {
    const pcm = await speakToPcm(apiKey, voiceId, STT_PROBE_TEXT);
    await mkdir(path.dirname(STT_PROBE_TARGET), { recursive: true });
    await writeFile(STT_PROBE_TARGET, pcm);
    console.log(
      `STT-probe: ${STT_PROBE_TARGET} (${(pcm.length / (SAMPLE_RATE * 2)).toFixed(2)}s, ${pcm.length} B)`,
    );
    return;
  }

  const rows = await parseManifest();
  console.log(`Manifest: ${rows.length} klip → ${OUT_DIR} (model ${MODEL_ID})`);
  await mkdir(OUT_DIR, { recursive: true });

  let failed = 0;
  let skipped = 0;
  for (const [index, row] of rows.entries()) {
    const label = `[${index + 1}/${rows.length}] ${row.file}`;
    const target = path.join(OUT_DIR, row.file);
    // Genoptagelig: eksisterende klip springes over (slet filen for re-gen —
    // også vejen til nye takes, hvis et enkelt klip lyder skævt).
    if (await access(target).then(() => true, () => false)) {
      skipped += 1;
      continue;
    }
    for (let attempt = 1; attempt <= 5; attempt += 1) {
      try {
        const pcm = await speakToPcm(apiKey, voiceId, row.text);
        if (pcm.length < 2_000) throw new Error(`for lidt audio (${pcm.length} B)`);
        await writeFile(target, wavFromPcm(pcm));
        console.log(`${label} OK ${(pcm.length / (SAMPLE_RATE * 2)).toFixed(1)}s`);
        break;
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
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
  if (skipped > 0) console.log(`(${skipped} eksisterende klip sprunget over)`);
  console.log(`\nFærdig: ${rows.length - failed}/${rows.length} klip i ${OUT_DIR}`);
  process.exitCode = failed === 0 ? 0 : 1;
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
});
