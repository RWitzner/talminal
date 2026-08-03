#!/usr/bin/env node
// Dode links i dokumentationen — en gate, ikke en oprydning.
//
// Repoets dokumenter krydsrefererer hinanden tungt: SECURITY.md naevnes 12
// steder, CONTRIBUTING 5, ARCHITECTURE 4. Flytter eller omdoeber nogen en fil,
// braekker de links TAVST — der er ingen compiler og ingen test der laeser
// markdown, saa fejlen opdages foerst af en fremmed der klikker.
//
// Den blev skrevet fordi doc-filerne skulle flyttes ud af repo-roden, og en
// flytning uden et vaern er praecis den slags aendring der kan forringe de
// dokumenter vi har brugt tid paa at goere sande. Nu er den en staaende gate:
// den betaler for sig selv naeste gang nogen omdoeber noget.
//
// Kun RELATIVE links tjekkes. Eksterne URL'er ville kraeve netvaerk og goere
// gaten flaky — og en doed ekstern URL er ikke vores fejl paa samme maade.

import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

// `git ls-files` frem for en mappe-scanning: kun SPOREDE filer kan braekke for
// en fremmed, og det holder node_modules og byggeartefakter ude uden en
// ekskluderingsliste der selv kan drifte.
const tracked = execFileSync('git', ['ls-files', '*.md'], {
  cwd: repoRoot,
  encoding: 'utf8',
})
  .split('\n')
  .filter(Boolean)

// [tekst](maal) — men ikke ![billede](...) er ogsaa et link, saa begge tages.
// Reference-stil (`[tekst][id]` + `[id]: maal`) haandteres af den anden regex.
const INLINE = /\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g
const REFERENCE = /^\[[^\]]+\]:\s*(\S+)/gm

const isExternal = (target) =>
  /^(https?:|mailto:|tel:|data:|#)/i.test(target)

const problems = []
let checked = 0

for (const file of tracked) {
  const abs = path.join(repoRoot, file)
  const text = readFileSync(abs, 'utf8')
  const dir = path.dirname(abs)

  for (const regex of [INLINE, REFERENCE]) {
    regex.lastIndex = 0
    let match
    while ((match = regex.exec(text)) !== null) {
      const raw = match[1]
      if (isExternal(raw)) continue

      // `fil.md#afsnit` — vi verificerer filen, ikke ankeret. Ankre er
      // genereret af overskrifter og ville kraeve en markdown-parser for at
      // tjekke praecist; filen er det der braekker ved en flytning.
      const [relative] = raw.split('#')
      if (!relative) continue

      checked += 1
      const resolved = path.resolve(dir, decodeURIComponent(relative))
      if (!existsSync(resolved)) {
        const line = text.slice(0, match.index).split('\n').length
        problems.push({ file, line, target: raw })
      }
    }
  }
}

if (problems.length > 0) {
  console.error(`link-check: ${problems.length} doede link(s)\n`)
  for (const p of problems) {
    console.error(`  ${p.file}:${p.line} -> ${p.target}`)
  }
  console.error(
    '\nEt relativt link peger paa noget der ikke findes. Blev en fil flyttet\n' +
      'eller omdoebt, skal henvisningerne med.',
  )
  process.exit(1)
}

console.log(
  `link-check: ${checked} relative link(s) i ${tracked.length} markdown-filer — alle findes.`,
)
