// Vagt mod at testsuiten skriver i den RIGTIGE Talminal-datamappe.
//
// Sandkassen i src-tauri/tests/common/mod.rs daekker alt der tager
// `common::serial()`. Dette script daekker resten: en ny testfil der aldrig
// naaede at tage serial(), en ny undermappe ingen har taenkt paa, en fremtidig
// tredje kilde til `talminal_base()`. Det er forskellen paa en regel man skal
// kende og et tjek der bliver roedt — og det er derfor det her hoerer i CI naar
// repoet aabnes: en bidragyder skal ikke behoeve at have laest en runbook for
// ikke at oedelaegge sin egen installation ved sin foerste `cargo test`.
//
// Brug (fra canvas/):
//   node scripts/data-dir-guard.mjs snapshot     # FOER cargo test
//   cargo test  (og cargo test --features supervision)
//   node scripts/data-dir-guard.mjs verify       # exit 1 hvis noget aendrede sig
//
// Manifestet ligger under src-tauri/target/, saa `cargo clean` rydder det og
// git ignorerer det.

import { createHash } from "node:crypto";
import { mkdir, readFile, readdir, stat, writeFile } from "node:fs/promises";
import { dirname, join, relative, sep } from "node:path";

/** Manifestets skema-version. Boer op naar `scan()`s format aendrer sig, saa et
 *  gammelt manifest giver et frisk snapshot i stedet for 289 falske roede. */
const MANIFEST_VERSION = 2;

const mode = process.argv[2];
if (mode !== "snapshot" && mode !== "verify") {
  console.error("usage: node scripts/data-dir-guard.mjs <snapshot|verify>");
  process.exit(2);
}

/** Begge levende rødder: TALMINAL_HOME (projekt-state, samme opløsning som
 *  cards::talminal_base()) og TALMINAL_GLOBAL_HOME (settings.json,
 *  last_project, active_workspace.json — samme opløsning som
 *  project::global_base()). Har udvikleren ikke sat overrides, falder begge
 *  tilbage på %LOCALAPPDATA%\Talminal og er dermed samme mappe — så scannes
 *  den kun én gang. Sætter `serial()` (tests/common/mod.rs) derimod
 *  TALMINAL_GLOBAL_HOME til en anden sandkasse end TALMINAL_HOME, er det
 *  to rødder, og begge skal være urørte. */
function realDataDirs() {
  const localAppData = process.env.LOCALAPPDATA;
  const home = process.env.TALMINAL_HOME || (localAppData && join(localAppData, "Talminal"));
  const global = process.env.TALMINAL_GLOBAL_HOME || (localAppData && join(localAppData, "Talminal"));
  if (!home || !global) {
    console.error("data-dir-guard: hverken TALMINAL_HOME/TALMINAL_GLOBAL_HOME eller LOCALAPPDATA er sat");
    process.exit(2);
  }
  return home === global ? [home] : [home, global];
}

const dataDirs = realDataDirs();
const manifestPath = join("src-tauri", "target", "data-dir-guard.json");

/** Undermapper med ANDRE skribenter end appen og testene. De skal ikke kunne
 *  give en falsk roed: statusline-tap'en skriver `hud\usage.json` mens Claude
 *  Code koerer (usage_hud.rs dokumenterer den kontrakt), og Python-controlleren
 *  ejer `ingress\` og `presence\`. Koerer ejeren ritualet med tap'en aktiv,
 *  aendrer de sig midt i maalingen uden at en test har roert dem.
 *
 *  De ignoreres IKKE — de rapporteres som en note og faelder ikke koerslen.
 *  Alt andet (threads\, projects\, signals\, workspace.json, settings.json,
 *  cards.toml) er app- og test-flade og er haard fejl. */
const EXTERNAL_WRITERS = ["hud/", "ingress/", "presence/"];

const isExternal = (path) => EXTERNAL_WRITERS.some((prefix) => path.startsWith(prefix));

/** Relativ sti -> `{ size, hash }`, for hver fil rekursivt.
 *
 *  **Indholdet er signalet, ikke stoerrelsen.** Vagten hed "uroert af
 *  testsuiten", men sammenlignede indtil 2026-07-27 kun filstoerrelser — og en
 *  overskrivning der tilfaeldigvis rammer samme laengde er praecis den slags en
 *  test laver: `last_project` med et andet slug af samme laengde, en
 *  `settings.json` hvor ét felt er vendt, et `status.json` med en anden pid.
 *  Reviewet falsificerede kontrakten ved at aendre AAAA til BBBB i en fil under
 *  roden; `verify` svarede exit 0 og "uroert". Nu er svaret roedt.
 *
 *  mtime er stadig ikke med: den larmer over filer appen selv roerer mellem to
 *  koersler uden at aendre en byte. */
async function scan(dir) {
  const files = {};
  async function walk(current) {
    let entries;
    try {
      entries = await readdir(current, { withFileTypes: true });
    } catch (error) {
      if (error.code === "ENOENT") return;
      throw error;
    }
    for (const entry of entries) {
      const full = join(current, entry.name);
      if (entry.isDirectory()) {
        await walk(full);
      } else if (entry.isFile()) {
        const nøgle = relative(dir, full).split(sep).join("/");
        try {
          const bytes = await readFile(full);
          files[nøgle] = {
            size: bytes.length,
            hash: createHash("sha256").update(bytes).digest("hex").slice(0, 16),
          };
        } catch (error) {
          // Filen kan vaere vaek eller laast af en anden skribent mellem
          // readdir og readFile. Stoerrelsen alene er stadig bedre end at
          // udelade posten (som ville se ud som en sletning) — og faldet
          // noteres i posten selv, saa en sammenligning mod en hash-post
          // ikke bliver en tavs "uaendret".
          if (error.code !== "ENOENT" && error.code !== "EBUSY" && error.code !== "EPERM") throw error;
          const size = await stat(full).then((s) => s.size).catch(() => -1);
          files[nøgle] = { size, hash: `ulaeselig:${error.code}` };
        }
      }
    }
  }
  await walk(dir);
  return files;
}

/** Bygger manifestets {roots:[{dir,files}]} for de aktuelle rødder. */
async function snapshotRoots() {
  const roots = [];
  for (const dir of dataDirs) {
    roots.push({ dir, files: await scan(dir) });
  }
  return roots;
}

async function writeSnapshot() {
  const roots = await snapshotRoots();
  await mkdir(dirname(manifestPath), { recursive: true });
  await writeFile(
    manifestPath,
    JSON.stringify({ version: MANIFEST_VERSION, roots }, null, 2) + "\n",
    "utf8",
  );
  return roots;
}

if (mode === "snapshot") {
  const roots = await writeSnapshot();
  const total = roots.reduce((n, r) => n + Object.keys(r.files).length, 0);
  console.log(
    `data-dir-guard: snapshot af ${roots.length} rod/rødder (${roots.map((r) => r.dir).join(", ")}) — ${total} fil(er)`,
  );
  process.exit(0);
}

let manifest;
try {
  manifest = JSON.parse(await readFile(manifestPath, "utf8"));
} catch {
  console.error(
    `data-dir-guard: intet snapshot i ${manifestPath} — koer "snapshot" FOER cargo test`,
  );
  process.exit(2);
}

/** Manifestet har skiftet skema to gange: fra {dataDir,files} til
 *  {roots:[{dir,files}]} (T0), og fra stoerrelser til `{size,hash}` pr. fil
 *  (2026-07-27, `version: 2`). Et snapshot i et gammelt skema — eller et der
 *  ikke daekker de rødder vi nu maaler — kan ikke sammenlignes, men det er
 *  ikke en fejl der skal faelde ritualet: det er bare et FOERSTE run under
 *  det nye skema. Vi tager et frisk snapshot og bestaar i stedet for exit 2,
 *  saa en gammel .json under target/ ikke laaser en bidragyder ude (eller,
 *  vaerre, giver 289 falske "aendret"-linjer fordi tal sammenlignes med
 *  objekter). */
const manifestDirs = Array.isArray(manifest.roots) ? manifest.roots.map((r) => r.dir) : null;
const schemaMatches =
  manifest.version === MANIFEST_VERSION &&
  manifestDirs !== null &&
  manifestDirs.length === dataDirs.length &&
  manifestDirs.every((dir, i) => dir === dataDirs[i]);

if (!schemaMatches) {
  await writeSnapshot();
  console.log(
    `data-dir-guard: ukendt eller forældet snapshot-skema i ${manifestPath} — ` +
      `tog et nyt snapshot af ${dataDirs.join(", ")} og bestod (ingen sammenligning denne gang)`,
  );
  process.exit(0);
}

const changes = [];
const notes = [];
const record = (path, line) => (isExternal(path) ? notes : changes).push(line);

for (const root of manifest.roots) {
  const after = await scan(root.dir);
  for (const [path, nu] of Object.entries(after)) {
    const før = root.files[path];
    if (før === undefined) {
      record(path, `  + [${root.dir}] ${path} (${nu.size} B, ny)`);
    } else if (før.size !== nu.size) {
      record(path, `  ~ [${root.dir}] ${path} (${før.size} -> ${nu.size} B)`);
    } else if (før.hash !== nu.hash) {
      // Samme laengde, andet indhold — den klasse den gamle vagt ikke saa.
      record(path, `  ~ [${root.dir}] ${path} (${nu.size} B, indholdet er aendret)`);
    }
  }
  for (const path of Object.keys(root.files)) {
    if (after[path] === undefined) record(path, `  - [${root.dir}] ${path} (slettet)`);
  }
}

if (notes.length > 0) {
  console.log(
    `data-dir-guard: note — ${notes.length} aendring(er) i mapper med andre ` +
      `skribenter end testene (statusline-tap / controller), ikke en fejl:\n` +
      notes.join("\n"),
  );
}

if (changes.length === 0) {
  console.log(`data-dir-guard: ${dataDirs.join(", ")} er uroert af testsuiten — OK`);
  process.exit(0);
}

console.error(
  `data-dir-guard: testsuiten skrev i den RIGTIGE datamappe:\n` +
    changes.join("\n") +
    "\n\nEn test opløste sin datasti til den levende installation. Tag " +
    "`common::serial()` som foerste linje i testen (den sandkasser " +
    "TALMINAL_HOME og TALMINAL_GLOBAL_HOME), eller peg stien et andet sted " +
    "eksplicit.\n\nHvorfor det er alvorligt: rod-mapperne holder LEVENDE " +
    "global tilstand — settings.json opløses af project::global_base() og " +
    "bor praecis her. At forureningen indtil nu kun har ramt threads\\ er et " +
    "tilfaelde, ikke en beskyttelse.",
);
process.exit(1);
