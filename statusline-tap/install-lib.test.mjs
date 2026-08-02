import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { isTapCommand, planInstall, planUninstall } from "./install-lib.mjs";

// Fixtur: en vilkårlig ANDEN statusline-kommando, dvs. den delegat tap'en skal
// fange og gendanne. Indholdet er opakt — kun identiteten testes.
const OTHER_STATUSLINE =
  '"C:\\Program Files\\nodejs\\node.exe" "C:/Users/x/.claude/hud/other-hud.mjs"';
const NODE = "C:\\Program Files\\nodejs\\node.exe";
const TAP = "C:\\Users\\x\\AppData\\Local\\Talminal\\hud\\tap.mjs";

describe("planInstall", () => {
  it("fanger den eksisterende kommando som delegat", () => {
    const plan = planInstall({ statusLine: { type: "command", command: OTHER_STATUSLINE } }, NODE, TAP, null);
    expect(plan.delegate).toBe(OTHER_STATUSLINE);
    expect(plan.alreadyInstalled).toBe(false);
    expect(plan.settings.statusLine).toEqual({ type: "command", command: `"${NODE}" "${TAP}"` });
  });

  it("er idempotent: geninstall bevarer den gamle delegat", () => {
    const first = planInstall({ statusLine: { type: "command", command: OTHER_STATUSLINE } }, NODE, TAP, null);
    const second = planInstall(first.settings, NODE, TAP, first.delegate);
    expect(second.alreadyInstalled).toBe(true);
    expect(second.delegate).toBe(OTHER_STATUSLINE);
  });

  it("afbryder når tap'en er aktiv men tap-config-FILEN er væk (mistet tap-config)", () => {
    const tapSettings = { statusLine: { type: "command", command: `"${NODE}" "${TAP}"` } };
    const plan = planInstall(tapSettings, NODE, TAP, undefined);
    expect(plan.abort).toContain("tap-config.json mangler");
    expect(plan.settings).toBeUndefined();
  });

  it("lover ikke at --uninstall giver en tom delegat-kæde — M21 afviser den", () => {
    // Teksten læses af operatøren i præcis den tilstand hvor --uninstall nu
    // AFBRYDER; den gamle formulering beskrev en kæde der ikke længere opstår.
    const tapSettings = { statusLine: { type: "command", command: `"${NODE}" "${TAP}"` } };
    const plan = planInstall(tapSettings, NODE, TAP, undefined);
    expect(plan.abort).not.toContain("TOM delegat-kæde");
    expect(plan.abort).toContain("--uninstall er ingen udvej");
  });

  it("geninstall med legitim null-delegat aborterer IKKE (config findes, delegate:null)", () => {
    const tapSettings = { statusLine: { type: "command", command: `"${NODE}" "${TAP}"` } };
    const plan = planInstall(tapSettings, NODE, TAP, null);
    expect(plan.abort).toBeUndefined();
    expect(plan.alreadyInstalled).toBe(true);
    expect(plan.delegate).toBeNull();
  });

  it("håndterer manglende statusLine (delegat = null) og bevarer øvrige settings", () => {
    const plan = planInstall({ model: "x" }, NODE, TAP, null);
    expect(plan.delegate).toBeNull();
    expect(plan.settings.model).toBe("x");
    expect(plan.settings.statusLine.command).toBe(`"${NODE}" "${TAP}"`);
  });
});

describe("planUninstall", () => {
  it("gendanner delegaten", () => {
    const settings = planUninstall({ statusLine: { type: "command", command: "tap" } }, OTHER_STATUSLINE);
    expect(settings.statusLine).toEqual({ type: "command", command: OTHER_STATUSLINE });
  });

  it("fjerner statusLine når delegat er null og bevarer øvrige settings", () => {
    const settings = planUninstall({ statusLine: { type: "command", command: "tap" }, model: "x" }, null);
    expect(settings.statusLine).toBeUndefined();
    expect(settings.model).toBe("x");
  });
});

describe("isTapCommand", () => {
  it("genkender tap-kommandoen og afviser en anden statusline-kommando/null", () => {
    expect(isTapCommand(`"${NODE}" "${TAP}"`)).toBe(true);
    expect(isTapCommand(OTHER_STATUSLINE)).toBe(false);
    expect(isTapCommand(null)).toBe(false);
  });
});

// install.mjs har top-level bivirkninger og kan ikke importeres — den koeres
// som underproces mod en sandkasset USERPROFILE/LOCALAPPDATA, saa hverken den
// rigtige ~/.claude/settings.json eller den rigtige hud-mappe kan rammes.
describe("install.mjs --uninstall (underproces)", () => {
  const INSTALL = fileURLToPath(new URL("./install.mjs", import.meta.url));
  const TAP_COMMAND = `"${NODE}" "${TAP}"`;
  let sandbox;

  const env = () => ({
    ...process.env,
    USERPROFILE: sandbox,
    HOME: sandbox,
    LOCALAPPDATA: join(sandbox, "Local"),
  });

  const settingsPath = () => join(sandbox, ".claude", "settings.json");
  const backupPath = () => join(sandbox, ".claude", "settings.json.talminal-tap.bak");
  const hudDir = () => join(sandbox, "Local", "Talminal", "hud");
  const readSettings = () => JSON.parse(readFileSync(settingsPath(), "utf-8"));

  beforeEach(() => {
    sandbox = mkdtempSync(join(tmpdir(), "talminal-tap-test-"));
    // Sikkerhedsbelte: uninstall-vejen SKRIVER settings.json. Bevis derfor
    // FOERST at os.homedir() faktisk foelger den sandkassede USERPROFILE —
    // ellers ville testen redigere ejerens rigtige ~/.claude/settings.json.
    const probe = spawnSync(process.execPath, ["-p", "require('node:os').homedir()"], {
      encoding: "utf8",
      env: env(),
    });
    expect(probe.stdout.trim()).toBe(sandbox);

    mkdirSync(join(sandbox, ".claude"), { recursive: true });
    writeFileSync(
      settingsPath(),
      JSON.stringify({ statusLine: { type: "command", command: TAP_COMMAND }, model: "x" }, null, 2),
    );
    mkdirSync(hudDir(), { recursive: true });
  });

  afterEach(() => {
    rmSync(sandbox, { recursive: true, force: true });
  });

  const uninstall = () =>
    spawnSync(process.execPath, [INSTALL, "--uninstall"], { encoding: "utf8", env: env() });

  it("afbryder uden at røre settings.json når tap-config-FILEN er væk, og peger på .bak'en når den findes", () => {
    writeFileSync(backupPath(), JSON.stringify({ statusLine: { type: "command", command: OTHER_STATUSLINE } }));

    const result = uninstall();

    expect(result.status).toBe(1);
    expect(result.stderr).toContain("tap-config.json mangler");
    // Beskeden skal sige FJERNE, ikke gendanne — og pege på backuppen.
    expect(result.stderr).toContain("FJERNE statusLine");
    expect(result.stderr).toContain("settings.json.talminal-tap.bak");
    expect(result.stderr).toContain("gendan statusLine derfra");
    expect(readSettings().statusLine).toEqual({ type: "command", command: TAP_COMMAND });
  });

  it("sender ikke brugeren efter en .bak der ikke findes", () => {
    // Backuppen tages kun ved FØRSTE install med en eksisterende settings.json —
    // på en maskine hvor den betingelse aldrig var opfyldt, findes filen ikke.
    // Den ubetingede "gendan derfra" pegede dér på et tomrum.
    const result = uninstall();

    expect(result.status).toBe(1);
    expect(result.stderr).toContain("Der er ingen backup at gendanne fra");
    // Præcis den ubetingede formulering fra runde 1 må ikke stå her.
    expect(result.stderr).not.toContain("gendan derfra");
    // Den ærlige vej ud skal stå der i stedet for henvisningen til .bak'en.
    expect(result.stderr).toContain("statusLine.command");
    expect(readSettings().statusLine).toEqual({ type: "command", command: TAP_COMMAND });
  });

  it("gendanner delegaten når config-filen findes (abort'en rammer kun den manglende FIL)", () => {
    writeFileSync(
      join(hudDir(), "tap-config.json"),
      JSON.stringify({ version: 1, delegate: OTHER_STATUSLINE }),
    );

    const result = uninstall();

    expect(result.status).toBe(0);
    expect(result.stdout).toContain(OTHER_STATUSLINE);
    const settings = readSettings();
    expect(settings.statusLine).toEqual({ type: "command", command: OTHER_STATUSLINE });
    expect(settings.model).toBe("x");
  });

  it("fjerner statusLine når config-filen findes med legitim delegate:null", () => {
    writeFileSync(
      join(hudDir(), "tap-config.json"),
      JSON.stringify({ version: 1, delegate: null }),
    );

    const result = uninstall();

    expect(result.status).toBe(0);
    expect(readSettings().statusLine).toBeUndefined();
  });
});
