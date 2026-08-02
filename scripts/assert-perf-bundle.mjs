import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";

const expected = process.argv[2];
if (expected !== "on" && expected !== "off") {
  throw new Error("usage: node scripts/assert-perf-bundle.mjs <on|off>");
}

const assets = join(process.cwd(), "dist", "assets");
const javascript = (await readdir(assets))
  .filter((name) => name.endsWith(".js"))
  .sort();
const bundle = (
  await Promise.all(javascript.map((name) => readFile(join(assets, name), "utf8")))
).join("\n");
const sentinels = [
  "__TALMINAL_PERF__",
  "perf_trace_frontend",
  "perfTrace",
  "perfStartedMs",
  "create.frontend.invoke.begin",
  "close.frontend.invoke.begin",
  "voice.router.frontend.begin",
];
const present = sentinels.filter((sentinel) => bundle.includes(sentinel));
const forbiddenWhenOff = [
  "frontend.start",
  "frontend.refresh.",
  "create.frontend.",
  "create.frame_",
  "create.term_open",
  "create.first_pty_chunk",
  "create.pty_listener_registered",
  "create.transport_interactive",
  "close.frontend.",
  "close.removed.",
  "voice.clip.",
  "voice.clip_assets.",
  "voice.dispatch.invoke.",
  "voice.dispatch.command.",
  "voice.reply.selected",
  "voice.release.residual_setup.",
  "voice.release.pipeline",
  "voice.capture.stop.",
  "voice.stt.stop.",
  "voice.stt.final",
  "voice.router.frontend.",
  "voice.turn.cancelled",
  "voice.output_detected",
];
const allForbidden = [...new Set([...sentinels, ...forbiddenWhenOff])];
const leaked = allForbidden.filter((fragment) => bundle.includes(fragment));

if (expected === "off" && leaked.length > 0) {
  throw new Error(
    `normal bundle retained perf strings: ${leaked.join(", ")}`,
  );
}
if (expected === "on" && present.length !== sentinels.length) {
  const missing = sentinels.filter((sentinel) => !present.includes(sentinel));
  throw new Error(`perf bundle lost perf sentinels: ${missing.join(", ")}`);
}

console.log(
  expected === "off"
    ? `perf bundle audit (off): ${javascript.length} JS asset(s), 0/${allForbidden.length} forbidden strings`
    : `perf bundle audit (on): ${javascript.length} JS asset(s), ${present.length}/${sentinels.length} required sentinels`,
);
