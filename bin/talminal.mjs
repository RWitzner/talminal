#!/usr/bin/env node
// Shebang'en er IKKE kosmetik. Uden den genererer npm en cmd-shim der ikke
// kalder node:
//
//     "%dp0%\node_modules\talminal\bin\talminal.mjs"   %*
//
// Da `.mjs` hverken har filassociation eller staar i PATHEXT, goer cmd
// INTET og returnerer 0. `talminal` ville altsaa se ud til at lykkes uden
// nogensinde at starte. Maalt med npm 11.5.2 / Node 24.15.0.
//
// Node har ingen `exec` paa Windows, saa vi kan ikke erstatte processen.
// Vi spawner og proxy'er argv OG exit-koden. Taber man exit-koden, bliver
// `npx talminal` ubrugelig i scripts — og en test der kun tjekker
// "ingen fejl" ville bestaa paa den oedelagte form.

import { spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const exe = path.join(here, '..', 'vendor', 'talminal.exe')

if (process.platform !== 'win32' || process.arch !== 'x64') {
  // `os`/`cpu` i package.json giver normalt en ren EBADPLATFORM ved install.
  // Naar man alligevel naar hertil (fx `npm i --force`), skal beskeden sige
  // hvorfor frem for at lade en exe-fejl forvirre.
  console.error(
    `talminal: kun Windows x64 er understoettet (fandt ${process.platform}/${process.arch}).\n` +
      'ConPTY, Credential Manager og WebView2 er alle i den kritiske sti.',
  )
  process.exit(1)
}

if (!existsSync(exe)) {
  console.error(
    `talminal: binaeren mangler (${exe}).\n` +
      'Pakken skal indeholde vendor/. Er den installeret med --ignore-scripts,\n' +
      'er DET ikke aarsagen — der er ingen postinstall-download; binaererne\n' +
      'ligger i selve tarballen. Prøv en ren geninstallation.',
  )
  process.exit(1)
}

// `stdio: 'inherit'` giver ogsaa Ctrl+C-videresendelse gratis: barnet deler
// vores konsol og faar selv konsol-signalet fra Windows.
const child = spawn(exe, process.argv.slice(2), { stdio: 'inherit' })

child.on('error', (err) => {
  console.error(`talminal: kunne ikke starte ${exe}: ${err.message}`)
  process.exit(1)
})

child.on('exit', (code, signal) => {
  // Et signal-draebt barn har ingen exit-kode. 1 er den aerlige oversaettelse:
  // "det gik ikke godt", frem for at lade `null` blive til 0.
  process.exit(signal ? 1 : (code ?? 0))
})
