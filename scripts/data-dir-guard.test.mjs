// Klassifikations-testen for `data-dir-guard.mjs`: hvad er HAARD fejl (exit 1)
// og hvad er blot en note (exit 0)?
//
// Vagtens vaerdi ligger i den skelnen, og skelnen er praecis den slags regel
// der roeres uden at nogen maerker det: gjorde man `hud/`-undtagelsen bredere
// igen, ville alle andre tests i huset stadig vaere groenne. Maalt 2026-08-02
// (T3) var undtagelsen en MAPPE-regel, og en glemsom test der skrev
// `hud\data-dir-guard-negative-probe.json` fik svaret "uroert af testsuiten —
// OK" med exit 0. Testene herunder er hukommelsen om det.
//
// Alt koerer mod EGNE rødder via TALMINAL_HOME/TALMINAL_GLOBAL_HOME og med
// cwd i en temp-mappe, saa manifestet lander i `<tmp>/src-tauri/target/`.
// Vagtens FULDE bevis mod den levende installation ligger et andet sted — i
// `src-tauri/tests/data_dir_guard_negative.rs`, som med vilje forurener den og
// derfor er `#[ignore]`d. Denne fil maa aldrig roere ejerens datamappe.

import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

const GUARD = fileURLToPath(new URL("./data-dir-guard.mjs", import.meta.url));

let work;
let dataDir;

/** Skriver en fil under datamappen; `rel` er en `/`-separeret relativ sti. */
function put(rel, content) {
  const path = join(dataDir, ...rel.split("/"));
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content);
  return path;
}

/** Koerer vagten med begge rødder peget paa sandkassen. cwd er `work`, saa
 *  manifestet aldrig kan lande i repoets rigtige `src-tauri/target/`. */
function guard(mode, env = {}) {
  const result = spawnSync(process.execPath, [GUARD, mode], {
    cwd: work,
    encoding: "utf8",
    env: {
      ...process.env,
      TALMINAL_HOME: dataDir,
      TALMINAL_GLOBAL_HOME: dataDir,
      ...env,
    },
  });
  return { code: result.status, out: `${result.stdout}${result.stderr}` };
}

beforeEach(() => {
  work = mkdtempSync(join(tmpdir(), "data-dir-guard-test-"));
  dataDir = join(work, "Talminal");
  mkdirSync(dataDir, { recursive: true });
});

afterEach(() => {
  rmSync(work, { recursive: true, force: true });
});

describe("haard fejl", () => {
  it("faelder en ny fil uden for mapperne med andre skribenter", () => {
    put("settings.json", "{}");
    expect(guard("snapshot").code).toBe(0);
    put("threads/data-dir-guard-negative-probe.jsonl", "forurening\n");

    const { code, out } = guard("verify");
    expect(code).toBe(1);
    expect(out).toContain("skrev i den RIGTIGE datamappe");
    expect(out).toContain("threads/data-dir-guard-negative-probe.jsonl");
  });

  it("faelder et ukendt navn i hud/ — mappen er ikke fribillet (T3-hullet)", () => {
    put("hud/usage.json", '{"version":1}');
    expect(guard("snapshot").code).toBe(0);
    put("hud/data-dir-guard-negative-probe.json", "fixtur fra en glemsom test\n");

    const { code, out } = guard("verify");
    expect(code).toBe(1);
    expect(out).toContain("hud/data-dir-guard-negative-probe.json");
    // Linjen skal kunne laeses uden at gaette hvorfor hud/ pludselig er roed.
    expect(out).toContain("daekker kun de NAVNE");
  });

  it("faelder et ukendt navn i presence/ — heller ikke DEN mappe er fribillet", () => {
    put("presence/controller.json", "{}");
    expect(guard("snapshot").code).toBe(0);
    put("presence/data-dir-guard-negative-probe.json", "fixtur fra en glemsom test\n");

    const { code, out } = guard("verify");
    expect(code).toBe(1);
    expect(out).toContain("presence/data-dir-guard-negative-probe.json");
  });

  it("faelder en aendring med SAMME laengde men andet indhold", () => {
    put("settings.json", "AAAA");
    expect(guard("snapshot").code).toBe(0);
    put("settings.json", "BBBB");

    const { code, out } = guard("verify");
    expect(code).toBe(1);
    expect(out).toContain("indholdet er aendret");
  });

  it("faelder en sletning uden for mapperne med andre skribenter", () => {
    put("cards.toml", "[[card]]\n");
    expect(guard("snapshot").code).toBe(0);
    rmSync(join(dataDir, "cards.toml"));

    const { code, out } = guard("verify");
    expect(code).toBe(1);
    expect(out).toContain("cards.toml (slettet)");
  });
});

describe("note — skribenter der ikke er testene", () => {
  /** Hver post: [beskrivelse, forberedelse, aendring]. Navnene er dem
   *  `statusline-tap/tap.mjs` og Python-controlleren faktisk producerer. */
  const cases = [
    [
      "tap'en skriver usage.json om (samme laengde, andet indhold)",
      () => put("hud/usage.json", '{"fiveHourPercent":11}'),
      () => put("hud/usage.json", '{"fiveHourPercent":22}'),
    ],
    [
      "tap'ens tmp+rename-sibling fanges midt i en scanning",
      () => put("hud/usage.json", "{}"),
      () => put("hud/usage.json.tmp-4242", "{}"),
    ],
    [
      "et nyt kort giver en ny context-snapshot-fil",
      () => put("hud/usage.json", "{}"),
      () => put("hud/context/kort-a.json", "{}"),
    ],
    [
      "tap'en sletter en context-fil aeldre end 7 dage",
      () => put("hud/context/gammel.json", "{}"),
      () => rmSync(join(dataDir, "hud", "context", "gammel.json")),
    ],
    [
      "tap-debug-dumpet naar tap'en er armeret",
      () => put("hud/usage.json", "{}"),
      () => put("hud/tap-debug-abc123.json", "{}"),
    ],
    [
      "controllerens presence-heartbeat for en ny session",
      () => put("presence/controller.json", "{}"),
      () => put("presence/session-7.json", "{}"),
    ],
    [
      "controllerens ingress-koe drives tom",
      () => put("ingress/opgave-1.json", "{}"),
      () => rmSync(join(dataDir, "ingress", "opgave-1.json")),
    ],
    // Tap-vaerktoejets ANDEN skribent: `statusline-tap/install.mjs` deployer
    // selve scripts'ene til samme mappe. En geninstallation midt i et ritual
    // maa ikke give en haard roed der anklager testsuiten for et menneskes
    // bevidste handling.
    [
      "installeren opdaterer tap.mjs ved en geninstallation",
      () => put("hud/tap.mjs", "// v1\n"),
      () => put("hud/tap.mjs", "// v2 med en rettelse\n"),
    ],
    [
      "installeren lander lib.mjs og tap-config.json foerste gang",
      () => put("hud/usage.json", "{}"),
      () => {
        put("hud/lib.mjs", "export const x = 1;\n");
        put("hud/tap-config.json", '{"version":1,"delegate":null}');
      },
    ],
  ];

  it.each(cases)("%s giver note og exit 0", (_navn, foer, efter) => {
    foer();
    expect(guard("snapshot").code).toBe(0);
    efter();

    const { code, out } = guard("verify");
    expect(code).toBe(0);
    expect(out).toContain("note —");
    expect(out).toContain("uroert af testsuiten");
  });
});

describe("rødder", () => {
  it("scanner BEGGE rødder naar TALMINAL_GLOBAL_HOME peger et andet sted", () => {
    const global = join(work, "Talminal-global");
    mkdirSync(global, { recursive: true });
    writeFileSync(join(global, "settings.json"), "{}");
    expect(guard("snapshot", { TALMINAL_GLOBAL_HOME: global }).code).toBe(0);
    writeFileSync(join(global, "last_project"), "et-andet-projekt");

    const { code, out } = guard("verify", { TALMINAL_GLOBAL_HOME: global });
    expect(code).toBe(1);
    expect(out).toContain("last_project");
  });

  it("bestaar med et frisk snapshot naar manifestet er fra andre rødder", () => {
    put("settings.json", "{}");
    expect(guard("snapshot").code).toBe(0);

    // Samme manifest, ny global-rod: skemaet kan ikke sammenlignes, og det er
    // et FOERSTE run — ikke en fejl.
    const global = join(work, "Talminal-global");
    mkdirSync(global, { recursive: true });
    const { code, out } = guard("verify", { TALMINAL_GLOBAL_HOME: global });
    expect(code).toBe(0);
    expect(out).toContain("forældet snapshot-skema");
  });
});
