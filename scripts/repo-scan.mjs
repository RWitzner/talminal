// Secret-/PII-/brand-scan af det sporede traeloader — CI's gate foer et
// offentligt repo, og en kommando en bidragyder selv kan koere.
//
// Brug:
//   node scripts/repo-scan.mjs            # exit 1 hvis der er uklassificerede hits
//   node scripts/repo-scan.mjs --verbose  # vis ogsaa de klassificerede
//
// HVORFOR JAVASCRIPT OG IKKE `git grep` I ET CI-STEP: fase 1's T9 maalte at
// den slags regex er SHELL-AFHAENGIG. Den samme kommando skrevet til
// PowerShells anfoerselstegn-semantik sender kun to backslashes videre i Git
// Bash og rammer et ANDET hit-saet — PowerShell-formen fangede tre steder som
// Bash-formen ikke saa. En gate der ser groennere ud end traeet er, er vaerre
// end ingen gate. Her gaar moenstrene aldrig gennem en shell, saa problemet
// findes ikke: der er én form, og den er den samme overalt.
//
// GATE-SEMANTIK: maalet er nul UKLASSIFICEREDE hits. Et hit der efter manuel
// inspektion er legitimt, foeres som en eksplicit undtagelse HER med sin
// begrundelse — det forsvinder ikke ved at blive ignoreret.

import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";

const verbose = process.argv.includes("--verbose");

/** Sporede filer, NUL-separeret saa navne med mellemrum ikke braekker.
 *  `git ls-files` og ikke en mappe-walk: kun det der faktisk bliver
 *  offentligt, tael med — node_modules og target/ er ikke i repoet. */
function trackedFiles() {
  const r = spawnSync("git", ["ls-files", "-z"], { encoding: "buffer" });
  if (r.status !== 0) {
    console.error("repo-scan: `git ls-files` fejlede — koeres den i et git-repo?");
    process.exit(2);
  }
  return r.stdout.toString("utf8").split("\0").filter(Boolean);
}

/** Binaere filer springes over. En PNG kan tilfaeldigvis indeholde en byte-
 *  sekvens der ligner et token, og en falsk positiv i en gate koster mere
 *  tillid end den fanger. NUL-byte er den samme heuristik git selv bruger. */
function readText(path) {
  let buf;
  try {
    buf = readFileSync(path);
  } catch {
    return null; // slettet mellem ls-files og laesning
  }
  if (buf.includes(0)) return null;
  return buf.toString("utf8");
}

/** Scriptets egen sti, som `git ls-files` skriver den. */
const SELV = "scripts/repo-scan.mjs";

/**
 * En scanner der ligger i det traeer den scanner, matcher SIG SELV. Denne fil
 * SKAL indeholde `personaos` og `C:\\Users` — det er dét moenstrene er.
 *
 * Fanget af CI 2026-08-03, ikke lokalt, og aarsagen er vaerd at kende: scannet
 * bruger `git ls-files`, saa det ser kun SPOREDE filer. Da jeg koerte det
 * lokalt, var scriptet stadig utracked og usynligt for sig selv; foerst efter
 * commit'en blev det en del af sit eget scan-omraade. En lokal koersel foer
 * commit kan altsaa ikke afsloere den her fejlklasse.
 *
 * Undtagelsen er BEVIDST SMAL. Kun de to gates hvis moenstre findes i filen som
 * DEFINITION, er undtaget. Token-, mail-, UUID- og ejernavns-gaterne er stadig
 * aktive paa filen — de moenstre optraeder ikke i kilden som data, saa en
 * indsat noegle her ville stadig blive fanget.
 */
const erScannerensEgenKilde = (fil) => fil === SELV;

/**
 * Hver gate: `pattern` finder kandidater, `allow` klassificerer dem som
 * legitime. En kandidat uden `allow`-traef er en FEJL.
 *
 * `allow` faar (linjen, filstien) — nogle undtagelser er kun legitime ét sted.
 */
const GATES = [
  {
    navn: "personlige stier",
    // Begge separatorer, begge kasus. Windows-stier optraeder baade som
    // `C:\Users\...` (Rust raw strings) og `C:/Users/...` (JS/TS-literaler).
    pattern: /[cC]:[\\/]+[uU]sers[\\/]+([^\\/"'\s,)\]]*)/g,
    allow: (linje, fil) =>
      // `x` er den neutrale pladsholder fase 1 (T9) indfoerte overalt hvor en
      // test eller et kommentar-eksempel havde brug for en Windows-sti. Den er
      // ikke et brugernavn, og den maa gerne staa.
      /[cC]:[\\/]+[uU]sers[\\/]+x([\\/]|["'\s,)\]]|$)/.test(linje) ||
      erScannerensEgenKilde(fil),
    hvorfor: "en RIGTIG brugerprofil-sti i et offentligt repo peger paa ejerens maskine",
  },
  {
    navn: "ejernavn i en sti",
    // Bevidst SNAEVER. Vi gater IKKE paa navnet alene: ejeren er
    // ophavsretshaver, og Apache-2.0 KRAEVER tilskrivning — `NOTICE`,
    // `package.json` og `Cargo.toml` skal baere navnet. Det der ikke maa staa,
    // er navnet som et STI-segment, for saa er det en hjemmemappe.
    pattern: /[\\/~]robins?([\\/]|$)/gi,
    allow: () => false,
    hvorfor: "navnet som sti-segment er en hjemmemappe, ikke en tilskrivning",
  },
  {
    navn: "mailadresser",
    // TLD-kravet udelukker ikon-stien `128x128@2x.png`, som en naiv regex
    // ellers rammer (maalt fund fra fase 1).
    pattern: /\b[a-z0-9._%+-]+@[a-z0-9.-]+\.(com|dk|net|org|io|dev)\b/gi,
    allow: (linje) =>
      // noreply-adresser er kanalen, ikke en laekage.
      /noreply@|users\.noreply\.github\.com/i.test(linje),
    hvorfor: "en privat mail i et offentligt repo er en spam-magnet",
  },
  {
    navn: "session-UUIDer",
    pattern: /\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/gi,
    allow: (linje) =>
      // Syntetiske fixture-UUIDer har otte ENS tegn i foerste gruppe. En aegte
      // Claude-/Codex-session-UUID har det ikke. Fase 1 erstattede den ene
      // aegte (`presence.rs`) med `00000000-0000-4000-8000-000000000001`.
      /\b([0-9a-f])\1{7}-/i.test(linje),
    hvorfor: "et aegte session-id knytter repoet til en konkret koersel hos ejeren",
  },
  {
    navn: "token-moenstre",
    pattern: /\b(sk-[A-Za-z0-9]{20,}|xoxb-[A-Za-z0-9-]{10,}|ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})/g,
    allow: () => false,
    hvorfor: "en levende noegle i historikken kan ikke kaldes tilbage",
  },
  {
    navn: "brand-rester",
    // Fokus-specens Delta 3. Det haandskrevne identity-map er en arbejdsliste;
    // DETTE er den mekaniske sandhed. PII-scannet fanger stier, mails og
    // tokens — ikke brand-rester.
    pattern: /personaos|cnvs/gi,
    allow: (_linje, fil) => erScannerensEgenKilde(fil),
    hvorfor: "produktet har aldrig heddet det, og et hit betyder at omdoebningen har en rest",
  },
];

let uklassificerede = 0;
let klassificerede = 0;

for (const gate of GATES) {
  const fejl = [];
  const ok = [];
  for (const fil of trackedFiles()) {
    const tekst = readText(fil);
    if (tekst === null) continue;
    const linjer = tekst.split(/\r?\n/);
    for (let i = 0; i < linjer.length; i++) {
      gate.pattern.lastIndex = 0;
      if (!gate.pattern.test(linjer[i])) continue;
      const post = `${fil}:${i + 1}: ${linjer[i].trim().slice(0, 160)}`;
      if (gate.allow(linjer[i], fil)) ok.push(post);
      else fejl.push(post);
    }
  }
  klassificerede += ok.length;
  uklassificerede += fejl.length;

  if (fejl.length === 0) {
    console.log(`OK   ${gate.navn} — ${ok.length} klassificeret, 0 uklassificeret`);
  } else {
    console.error(`FEJL ${gate.navn} — ${fejl.length} uklassificeret (${gate.hvorfor}):`);
    for (const f of fejl) console.error(`       ${f}`);
  }
  if (verbose && ok.length > 0) {
    console.log(`     klassificerede undtagelser:`);
    for (const o of ok) console.log(`       ${o}`);
  }
}

console.log(
  `repo-scan: ${uklassificerede} uklassificerede hits, ${klassificerede} klassificerede undtagelser`,
);
process.exit(uklassificerede === 0 ? 0 : 1);
