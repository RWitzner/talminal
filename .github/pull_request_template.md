<!--
Tak fordi du bidrager. Dansk og engelsk er lige velkomne.
Er ændringen større end en fejlrettelse, så åbn gerne et issue først.
-->

## Hvad og hvorfor

<!--
Diffen viser HVAD. Skriv HVORFOR: hvilken afvejning traf du, og hvad overvejede du
i stedet? Kodebasen er skrevet sådan — hold stilen.
-->

## Hvordan er det verificeret

<!--
Ritualet står i CONTRIBUTING.md. Indsæt de tal du målte, ikke "alt grønt":
-->

- [ ] `cargo test --locked` — passed: 
- [ ] `cargo test --locked --features supervision` — passed: 
- [ ] `npx vitest run` — suiter/tests: 
- [ ] `npm run build` — forbidden strings: 
- [ ] `cargo fmt --check` + alle tre clippy — nul output
- [ ] `node scripts/repo-scan.mjs` — uklassificerede hits: 
- [ ] `node scripts/data-dir-guard.mjs verify` — uberørt

Appen var lukket under kørslen: <!-- ja / nej -->

## Tjekliste

- [ ] Nye tests der rører proces-global tilstand (datamappen, env, registryet) tager
      `common::serial()` som første linje
- [ ] Ingen ændring i `version`-felterne (`package.json`, `Cargo.toml`, `tauri.conf.json`)
- [ ] Ingen personlige stier, mails eller nøgler — `repo-scan.mjs` er grøn
- [ ] Rører ændringen dataflowet, er [PRIVACY.md](../blob/main/PRIVACY.md) opdateret i
      samme PR
- [ ] Tilføjer den et medieaktiv, står proveniensen i
      [ASSETS.md](../blob/main/ASSETS.md)

## Licens

- [ ] Jeg leverer mit bidrag under [Apache-2.0](../blob/main/LICENSE), samme licens som
      projektet, og jeg har ret til at bidrage med det
