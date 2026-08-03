// Vaern om npm-pakkens form. De tre ting her er alle blevet maalt paa denne
// maskine (npm 11.5.2, Node 24.15.0) og er alle tavse naar de braekker — det
// er derfor de fortjener en test frem for en kommentar.

import { describe, expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const read = (p) => readFileSync(path.join(repoRoot, p), 'utf8')
const pkg = JSON.parse(read('package.json'))

describe('npm-pakkens form', () => {
  // MAALT: uden shebang genererer npm en cmd-shim der IKKE kalder node
  //     "%dp0%\node_modules\talminal\bin\talminal.mjs"   %*
  // Da .mjs hverken har filassociation eller staar i PATHEXT, goer cmd intet
  // og returnerer 0. Probe-maaling: uden shebang -> tom output, exit 0;
  // med shebang -> output, exit 7. Kommandoen ville altsaa se ud til at
  // lykkes uden nogensinde at starte.
  it('bin-shim starter med en node-shebang', () => {
    const shim = read('bin/talminal.mjs')
    expect(shim.split(/\r?\n/)[0]).toBe('#!/usr/bin/env node')
  })

  // Node har ingen exec paa Windows, saa shim'en SKAL videregive barnets
  // exit-kode selv. Gaar den tabt, er `npx talminal` ubrugelig i scripts —
  // og en test der kun tjekker "ingen fejl" ville bestaa paa den form.
  it('bin-shim videregiver exit-koden', () => {
    const shim = read('bin/talminal.mjs')
    expect(shim).toMatch(/process\.exit\(signal \? 1 : \(code \?\? 0\)\)/)
  })

  it('pakken kan udgives og peger paa shim\'en', () => {
    expect(pkg.private).toBeUndefined()
    expect(pkg.bin).toEqual({ talminal: 'bin/talminal.mjs' })
  })

  // `files` er en ALLOWLIST. Uden den ville en publish tage hele
  // arbejdstraeet med (maalt: 361 kildefiler og nul binaerer).
  it('files er en allowlist der baerer binaererne og deres rettighedsgrundlag', () => {
    expect(pkg.files).toContain('vendor/')
    expect(pkg.files).toContain('bin/')
    // NOTICE og ASSETS.md skal med: tarballen er sin egen
    // distributionskanal, og ASSETS.md er lydklippenes eneste grant.
    for (const f of ['README.md', 'LICENSE', 'NOTICE', 'ASSETS.md']) {
      expect(pkg.files).toContain(f)
    }
  })

  // `vendor/` maa ALDRIG spores: 19 MB binaerer i en offentlig historik kan
  // ikke fjernes igen. npm tager dem med alligevel — `files` slaar
  // .gitignore i en tarball, og det er praecis dér npm-semantikken er
  // kontraintuitiv.
  it('vendor er gitignoreret trods at den er med i tarballen', () => {
    expect(read('.gitignore')).toMatch(/^vendor\/$/m)
  })

  it('os og cpu giver en ren EBADPLATFORM', () => {
    expect(pkg.os).toEqual(['win32'])
    expect(pkg.cpu).toEqual(['x64'])
  })
})
