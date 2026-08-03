#!/usr/bin/env node
// Release-byg med sti-neutralisering + den gate der beviser den (§2b.5, gate 5).
//
// HVORFOR ET SCRIPT OG IKKE EN CONFIG-FIL
//
// Release-binaererne baerer byggemaskinens stier. Maalt foer dette:
// 930 forekomster af byggerens hjemmemappe i talminal-canvas.exe, 76 i
// talminal.exe, 94 i mcp_probe.exe. Seks af dem er fulde stier ind i
// Talminals egen kilde, herunder `secrets.rs` og `pty.rs`.
//
// `strip` loeser det IKKE. Stierne ligger ikke i debug-metadata, men i
// rustc-emitterede panic-`Location`-strenge (`file!()`) — almindelige
// statiske strenge i .rdata, som overlever enhver strip.
//
// `[profile.release] trim-paths` loeser det heller ikke: den er MAALT
// uverificerbar paa denne toolchain — cargo 1.89.0 svarer
// "requires the Cargo feature called `trim-paths`, but that feature is not
// stabilized in this version of Cargo ... may require the nightly release".
// Toolchainen er pinnet til stable 1.89.0, saa den vej er lukket.
//
// `.cargo/config.toml` med `rustflags` ville virke, men rammer CI-cachen:
// noeglen i ci.yml er `Cargo.lock` + toolchain, IKKE config.toml, og
// actions/cache gemmer ikke ved exact-hit. Hver eneste koersel ville
// genbygge hele dependency-traeet uden nogensinde at persistere — et
// direkte slag mod bidragyder-barren.
//
// Tilbage staar `--remap-path-prefix` sat pr. release-byg. Flaget er stabilt,
// og stierne udledes HER paa kaldstidspunktet, saa der ikke ligger en
// maskinafhaengig sti i en fil andre bygger med.
//
// HVAD DEN IKKE KAN — MAALT, IKKE FORMODET
//
// Remappet fjerner ~94 %: talminal-canvas.exe gik fra 930 til 84 forekomster.
// Resten har to kilder som rustc-flaget PR. KONSTRUKTION ikke kan naa:
//
//   1. `env!`-udvidelser og build-script-output (~51 i canvas.exe).
//      `--remap-path-prefix` omskriver kildestier i debug-info og i
//      `file!()`/panic-`Location`. Den roerer IKKE strenge en crate selv har
//      bygget af `env!("CARGO_MANIFEST_DIR")` eller skrevet ud fra et
//      build-script — for rustc er de almindelige data.
//   2. aws-lc-sys' C-oversaettelsesenheder (~32), som baerer deres egne
//      `__FILE__` i DOS-8.3-form (`CARGO~1\...\AWS-LC~1.0\...\bio.c`).
//      De kommer fra cl.exe. rustc ser dem aldrig.
//
// DEN FULDE LOESNING er derfor ikke et flag, men en byggesti uden brugernavn:
// klon til fx `C:\src\talminal` og saet `CARGO_HOME=C:\cargo`. Saa indeholder
// stierne ikke navnet, uanset hvilken mekanisme der indlejrede dem. Prisen er
// en frisk registry-hentning. Scriptet virker uaendret dér — og gaten bliver
// nul af sig selv, fordi der ikke er noget brugernavn at finde.
//
// Indtil da: hvad der laekker er ejerens Windows-brugernavn og et
// mappe-layout. Det staar i forvejen i `authors`, i `repository`-URL'en og paa
// npm. Den marginale afsloering er derfor lille — men den er ikke nul, og
// SECURITY.md siger det ligeud i stedet for at lade remappet se ud som en
// lukning.
//
// GATEN ER DERFOR EN REGRESSIONS-GATE, ikke en nul-gate: den faelder hvis
// tallet VOKSER over det maalte udgangspunkt. En nul-gate ville vaere
// uopnaaelig by construction og dermed en gate ingen kan passere.

import { execFileSync } from 'node:child_process'
import { readFileSync, readdirSync } from 'node:fs'
import { homedir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const tauriDir = path.join(repoRoot, 'src-tauri')
const cargoHome = process.env.CARGO_HOME || path.join(homedir(), '.cargo')

const check = process.argv.includes('--check-only')

if (!check) {
  const flags = [
    `--remap-path-prefix=${path.join(cargoHome, 'registry')}=/cargo-registry`,
    `--remap-path-prefix=${repoRoot}=/talminal`,
  ].join(' ')
  console.log(`release-build: RUSTFLAGS=${flags}`)
  execFileSync('cargo', ['build', '--release', '--locked'], {
    cwd: tauriDir,
    stdio: 'inherit',
    env: { ...process.env, RUSTFLAGS: `${process.env.RUSTFLAGS ?? ''} ${flags}`.trim() },
  })
}

// ---------------------------------------------------------------------------
// Gaten. Maaler de tre klasser HVER FOR SIG — et samlet tal ville skjule at
// den ene kilde er lukket og den anden ikke er.
// ---------------------------------------------------------------------------

const releaseDir = path.join(tauriDir, 'target', 'release')
const exes = readdirSync(releaseDir).filter((f) => f.endsWith('.exe'))
if (exes.length === 0) {
  console.error('release-build: ingen .exe i target/release — byg foerst')
  process.exit(1)
}

// Byggerens hjemmemappe, som den ville staa i en binaer. Vi leder efter selve
// brugernavnet, ikke en fast sti — det er dét der ikke maa udleveres.
const home = homedir()
const userName = path.basename(home)
const needle = Buffer.from(`${path.dirname(home)}${path.sep}${userName}`, 'latin1')

const countIn = (buf, sub) => {
  let n = 0
  let i = buf.indexOf(sub)
  while (i !== -1) {
    n++
    i = buf.indexOf(sub, i + 1)
  }
  return n
}

// Maalt udgangspunkt EFTER remappet (2026-08-03). Gaten faelder hvis et tal
// VOKSER — det ville betyde at en ny afhaengighed indlejrer flere stier, og
// det skal opdages her og ikke af en bruger med `strings`.
//
// Bygges der fra en sti uden brugernavn (se hoved-kommentaren), bliver alle
// tallene nul, og gaten er trivielt groen.
const BASELINE = {
  'talminal-canvas.exe': 84,
  'talminal.exe': 63,
  'mcp_probe.exe': 63,
}

let regressed = false
console.log('')
for (const exe of exes) {
  const buf = readFileSync(path.join(releaseDir, exe))
  const total = countIn(buf, needle)
  // aws-lc' C-stier bruger 8.3-kortnavnet for `.cargo`.
  const c = countIn(buf, Buffer.from(`${home}${path.sep}CARGO~1`, 'latin1'))
  const rust = total - c
  const max = BASELINE[exe]
  const over = max !== undefined && total > max
  if (over) regressed = true
  const verdict = total === 0 ? 'REN ' : over ? 'FEJL' : 'OK  '
  console.log(
    `${verdict} ${exe.padEnd(24)} i alt=${String(total).padEnd(4)} ` +
      `(env!/build-script=${String(rust).padEnd(4)} aws-lc/C=${String(c).padEnd(3)})` +
      (max === undefined ? '  [ingen baseline]' : `  baseline=${max}`),
  )
}

console.log('')
if (regressed) {
  console.error(
    `release-build: antallet af stier med brugernavnet "${userName}" er VOKSET over\n` +
      'baseline. En ny afhaengighed indlejrer formentlig sin manifest-sti. Undersoeg\n' +
      'foer release — eller byg fra en sti uden brugernavn, saa tallet bliver nul.',
  )
  process.exit(1)
}
console.log(
  'release-build: gate 5 OK — ingen regression.\n' +
    'Bemaerk: resten er env!/build-script-strenge og aws-lc\' C-stier, som\n' +
    'rustc-flaget ikke kan naa. Se SECURITY.md og hoved-kommentaren i dette script.',
)
