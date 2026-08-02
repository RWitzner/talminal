#!/usr/bin/env node
// Talminal statusline-tap: skriver rate_limits fra Claude Codes statusline-
// stdin til <base>\hud\usage.json og delegerer derefter til den oprindelige
// statusline-kommando (tap-config.json ved siden af det DEPLOYEDE script).
// Tap-fejl må ALDRIG vælte statuslinen — delegationen sker ubetinget til
// sidst. CC dræber in-flight statusline-scripts ved nye opdateringer; tmp +
// rename garanterer at usage.json aldrig ses halvskrevet.
import { spawn } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  buildDebugRecord,
  contextFileKey,
  debugSessionKey,
  extractContext,
  extractUsage,
} from "./lib.mjs";

const scriptDir = dirname(fileURLToPath(import.meta.url));

function readStdin() {
  return new Promise((resolve) => {
    const chunks = [];
    process.stdin.on("data", (chunk) => chunks.push(chunk));
    process.stdin.on("end", () => resolve(Buffer.concat(chunks)));
    process.stdin.on("error", () => resolve(Buffer.concat(chunks)));
  });
}

// usage.json/context-filerne er hud-data — én global base for alle sessioner.
// Canvas'en sætter TALMINAL_HOME per-projekt ved opstart, så den variabel
// må aldrig styre basen her (kort-sessioner arver den og ville skrive
// projekt-stien mens eksterne terminaler skrev den globale). Spejler
// project::global_base(): TALMINAL_GLOBAL_HOME (ikke-tom) vinder, ellers
// %LOCALAPPDATA%\Talminal. Uden brugbar LOCALAPPDATA springes skrivningen
// over (review-fund 8) — aldrig en relativ sti under statuslinens cwd.
function resolveGlobalBase() {
  const globalHome = process.env.TALMINAL_GLOBAL_HOME;
  const localAppData = process.env.LOCALAPPDATA;
  return globalHome && globalHome !== ""
    ? globalHome
    : localAppData && localAppData !== ""
      ? join(localAppData, "Talminal")
      : null;
}

// Fælles tmp+rename-skrivning (CC dræber in-flight statusline-scripts —
// target-filen må aldrig ses halvskrevet). Windows: rename-over kan tabe et
// race mod en samtidig skriver — ét kort genforsøg, derefter opgives stille
// (næste statusline-tick skriver igen); tmp ryddes op så opgivne forsøg ikke
// efterlader forældreløse filer.
function writeAtomic(dir, fileName, value) {
  mkdirSync(dir, { recursive: true });
  const target = join(dir, fileName);
  const tmp = join(dir, `${fileName}.tmp-${process.pid}`);
  writeFileSync(tmp, JSON.stringify(value, null, 2));
  try {
    renameSync(tmp, target);
    return "written";
  } catch {
    try {
      renameSync(tmp, target);
      return "written-after-retry";
    } catch {
      try {
        unlinkSync(tmp);
      } catch {
        /* opgiv stille */
      }
      return "rename-failed";
    }
  }
}

function writeSnapshot(snapshot) {
  const base = resolveGlobalBase();
  if (!base) return "skipped-no-base";
  return writeAtomic(join(base, "hud"), "usage.json", snapshot);
}

// Spec: lukkede korts context-filer er harmløse (badgen kræver et kørende
// kort), men tap'en sletter opportunistisk filer ældre end 7 dage så mappen
// ikke gror. Kun denne skrivnings egen fil fredes; stille ved enhver fejl.
const CONTEXT_PRUNE_MS = 7 * 24 * 60 * 60 * 1000;

function pruneOldContextFiles(contextDir, keepFileName) {
  try {
    const cutoff = Date.now() - CONTEXT_PRUNE_MS;
    for (const name of readdirSync(contextDir)) {
      if (name === keepFileName) continue;
      if (!name.endsWith(".json") && !name.includes(".json.tmp-")) continue;
      const path = join(contextDir, name);
      try {
        if (statSync(path).mtimeMs < cutoff) unlinkSync(path);
      } catch {
        /* stille — næste tick prøver igen */
      }
    }
  } catch {
    /* stille — oprydning er best effort */
  }
}

function writeContextSnapshot(context) {
  const base = resolveGlobalBase();
  if (!base) return "skipped-no-base";
  const contextDir = join(base, "hud", "context");
  const fileName = `${contextFileKey(context.cardName)}.json`;
  const outcome = writeAtomic(contextDir, fileName, context);
  pruneOldContextFiles(contextDir, fileName);
  return outcome;
}

// Debug-dump (rodårsags-jagt: kort-sessioner der aldrig skriver usage.json).
// Armeres med filen tap-debug.on VED SIDEN AF det deployede script eller
// TALMINAL_TAP_DEBUG i env. Dumpet skrives også til scriptDir — bevidst
// uafhængigt af base-opløsningen, så skipped-no-base-tilfældet selv fanges.
function debugDump(payloadText, snapshot, writeResult, contextResult) {
  try {
    const armed = (process.env.TALMINAL_TAP_DEBUG ?? "") !== ""
      || existsSync(join(scriptDir, "tap-debug.on"));
    if (!armed) return;
    const record = buildDebugRecord({
      payloadText,
      snapshot,
      writeResult,
      contextResult,
      env: process.env,
      nowIso: new Date().toISOString(),
      pid: process.pid,
    });
    const dumpPath = join(scriptDir, `tap-debug-${debugSessionKey(payloadText, process.pid)}.json`);
    writeFileSync(dumpPath, JSON.stringify(record, null, 2));
  } catch {
    /* dump-fejl må aldrig vælte tap'en */
  }
}

function delegate(stdinBuffer) {
  let command = null;
  try {
    const config = JSON.parse(readFileSync(join(scriptDir, "tap-config.json"), "utf-8"));
    command = typeof config.delegate === "string" && config.delegate.trim() !== ""
      ? config.delegate
      : null;
  } catch {
    /* ingen config → ingen delegat */
  }
  if (!command) {
    process.exit(0);
  }
  const child = spawn(command, { shell: true, stdio: ["pipe", "inherit", "inherit"] });
  child.on("error", () => process.exit(0));
  child.on("close", (code) => process.exit(code ?? 0));
  // EPIPE hvis delegaten dør uden at dræne stdin — en uhandlet stream-fejl
  // ville vælte tap'en og dermed statuslinen (review-fund 5).
  child.stdin.on("error", () => {});
  child.stdin.end(stdinBuffer);
}

const stdinBuffer = await readStdin();
const payloadText = stdinBuffer.toString("utf-8");
let snapshot = null;
let writeResult = "no-snapshot";
try {
  snapshot = extractUsage(payloadText, new Date().toISOString());
  if (snapshot) writeResult = writeSnapshot(snapshot);
} catch (err) {
  /* tap-fejl må ikke vælte statuslinen */
  writeResult = `error: ${err?.message ?? err}`;
}
// Context v1 (per-kort badge): uafhængig af usage-udfaldet — kun kort-
// sessioner (TALMINAL_SESSION_ID i env) producerer et udtræk.
let contextResult = "no-context";
try {
  const context = extractContext(payloadText, process.env, new Date().toISOString());
  if (context) contextResult = writeContextSnapshot(context);
} catch (err) {
  /* tap-fejl må ikke vælte statuslinen */
  contextResult = `error: ${err?.message ?? err}`;
}
debugDump(payloadText, snapshot, writeResult, contextResult);
delegate(stdinBuffer);
