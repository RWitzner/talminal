#!/usr/bin/env node
// Installerer statusline-tap'en: kopierer tap.mjs/lib.mjs til
// %LOCALAPPDATA%\Talminal\hud\, gemmer den eksisterende statusline-kommando
// som delegat i tap-config.json og peger ~/.claude/settings.json på tap'en.
// Backup af settings.json tages KUN første gang (bevarer originalen).
// `node install.mjs --uninstall` ruller statusLine tilbage til delegaten — og
// afbryder hvis selve tap-config.json er væk, så en FJERNELSE ikke maskeres som
// en gendannelse.
// Gælder nye Claude Code-sessioner; kørende sessioner beholder den gamle
// kommando til de genstartes.
import { copyFileSync, existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { isTapCommand, planInstall, planUninstall } from "./install-lib.mjs";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const settingsPath = join(homedir(), ".claude", "settings.json");
const backupPath = join(homedir(), ".claude", "settings.json.talminal-tap.bak");

// Uden LOCALAPPDATA kan hud-mappen ikke opløses — stop med en læsbar besked
// FØR nogen skrivning (i stedet for en rå TypeError fra join(undefined, ...)).
if (!process.env.LOCALAPPDATA) {
  console.error("LOCALAPPDATA er ikke sat eller tom — kan ikke opløse %LOCALAPPDATA%\\Talminal\\hud. Intet er ændret.");
  process.exit(1);
}
const hudDir = join(process.env.LOCALAPPDATA, "Talminal", "hud");
const tapPath = join(hudDir, "tap.mjs");
const configPath = join(hudDir, "tap-config.json");

function writeJsonAtomic(path, value) {
  const tmp = `${path}.tmp-${process.pid}`;
  writeFileSync(tmp, JSON.stringify(value, null, 2) + "\n");
  renameSync(tmp, path);
}

// Manglende settings.json = tom konfiguration (review-fund 6) — installeren
// skal virke "med enhver/ingen statusline".
const settings = existsSync(settingsPath)
  ? JSON.parse(readFileSync(settingsPath, "utf-8"))
  : {};

if (process.argv.includes("--uninstall")) {
  // Rør ALDRIG en statusLine der ikke er tap'ens — uninstall uden aktiv tap
  // er en no-op (review-fund 2, anden form).
  if (!isTapCommand(settings?.statusLine?.command ?? null)) {
    console.log("Tap'en er ikke aktiv i settings.json — intet at gendanne.");
    process.exit(0);
  }
  // Samme skelnen som install-vejen (se planInstall): en MANGLENDE tap-config-FIL
  // er tab af kæden til den oprindelige statusline — ikke et legitimt "ingen
  // delegat". Behandlede vi den som {}, blev delegaten null, planUninstall slettede
  // statusLine helt, og brugeren fik beskeden "gendannet" om en FJERNELSE. Stop i
  // stedet, uden at røre settings.json, og peg på den eneste vej tilbage.
  if (!existsSync(configPath)) {
    // Backuppen tages KUN ved FØRSTE install og kun hvis settings.json fandtes
    // (se betingelsen længere nede) — på en maskine hvor den aldrig var opfyldt,
    // findes .bak'en ikke. Uden forgreningen sendte beskeden brugeren efter en
    // fil der ikke er der, i præcis det øjeblik hun har mest brug for et ærligt
    // svar.
    const vejTilbage = existsSync(backupPath)
      ? `Den oprindelige kommando står i ${backupPath}; gendan statusLine derfra.`
      : `Der er ingen backup at gendanne fra (${backupPath} findes ikke — den tages kun ved FØRSTE install), så den oprindelige kommando er ikke gemt nogen steder. Kender du den, kan du enten skrive den ind i ${settingsPath} som statusLine.command, eller lægge den i ${configPath} som {"version":1,"delegate":"<kommando>"} og køre --uninstall igen; ellers slet statusLine-blokken for at få Claude Codes standard.`;
    console.error(
      `tap-config.json mangler (selve FILEN er væk) — --uninstall ville FJERNE statusLine helt, ikke gendanne den. ${vejTilbage} Intet er ændret.`,
    );
    process.exit(1);
  }
  const config = JSON.parse(readFileSync(configPath, "utf-8"));
  writeJsonAtomic(settingsPath, planUninstall(settings, config.delegate ?? null));
  console.log(`statusLine gendannet til: ${config.delegate ?? "(fjernet)"}`);
  process.exit(0);
}

mkdirSync(hudDir, { recursive: true });
copyFileSync(join(scriptDir, "tap.mjs"), tapPath);
copyFileSync(join(scriptDir, "lib.mjs"), join(hudDir, "lib.mjs"));

// undefined = config-FILEN mangler (abort-signal til planInstall når tap'en er
// aktiv); findes filen, er delegate ?? null legitimt "ingen delegat".
const existingDelegate = existsSync(configPath)
  ? JSON.parse(readFileSync(configPath, "utf-8")).delegate ?? null
  : undefined;
const plan = planInstall(settings, process.execPath, tapPath, existingDelegate);

if (plan.abort) {
  console.error(plan.abort);
  process.exit(1);
}

if (!plan.alreadyInstalled && existsSync(settingsPath) && !existsSync(backupPath)) {
  copyFileSync(settingsPath, backupPath);
}
writeJsonAtomic(configPath, { version: 1, delegate: plan.delegate });
writeJsonAtomic(settingsPath, plan.settings);

console.log(plan.alreadyInstalled
  ? "Tap allerede installeret — scripts og delegat genopfrisket."
  : `Tap installeret. Delegat: ${plan.delegate ?? "(ingen — statuslinen var tom)"}`);
console.log("Gælder nye Claude Code-sessioner (kørende beholder den gamle kommando til genstart).");
