# Sådan bidrager du til Talminal

Tak fordi du kigger. Det her er et lille projekt med én vedligeholder, så det vigtigste
du kan gøre, er at åbne et issue før du bygger noget stort — så vi ikke opdager for sent
at vi var uenige om retningen.

## Sproget

**Doc-kommentarerne i koden er på dansk.** Det gælder `browser_host.rs`, `mcp.rs`,
`pty.rs` og resten. Det er et bevidst valg, ikke et efterslæb, og det bliver ikke lavet
om: projektet er skrevet af og til danske udviklere, og kommentarerne bærer begrundelser
der er lettere at skrive præcist på modersmålet.

**Issues og pull requests på engelsk er meget velkomne.** Du behøver ikke skrive dansk
for at bidrage. Skriver du ny kode, må dine egne kommentarer gerne være på engelsk — bland
bare ikke sprog inden i én fil.

Du skal vide hvad du går ind til: kan du ikke læse dansk, vil du bruge en oversætter på
kommentarerne. Det er den ærlige pris ved at åbne det her repo som det er, i stedet for
at oversætte godt 100 filer og tabe nuancerne.

## Forudsætninger

- **Windows 11 x64.** Produktet er kun Windows — ConPTY, Credential Manager og WebView2
  er alle i den kritiske sti. `package.json` erklærer `"os": ["win32"]`, så en
  installation på mac eller Linux fejler pænt i stedet for kryptisk.
- **Rust** — versionen er pinnet i `rust-toolchain.toml`. Har du `rustup`, henter den
  selv den rigtige toolchain første gang du kører `cargo`.
- **Node** — versionen står i `.nvmrc`. Node er ikke kun et byggeværktøj: browser-kort
  starter `npx @playwright/mcp` ved kørsel, så det er også et **runtime**-krav.
- **C++ Build Tools** (MSVC) til at linke Rust-siden.
- **WebView2** — følger med Windows 11.

Du behøver **ikke** Claude Code eller Codex CLI installeret for at køre testsuiten. Det
gjorde du indtil august 2026, uden at nogen havde skrevet det ned; CI fandt det, og det
er rettet. Skal du køre selve appen, skal du naturligvis have mindst én af dem.

## Ritualet

Det her er den ene ting du skal kunne køre. Alt skal være grønt, og **appen skal være
lukket** imens — en kørende Talminal skriver i datamappen, og så bliver vagten til sidst
rød med rette.

```powershell
node scripts/data-dir-guard.mjs snapshot

cd src-tauri
cargo test --locked
cargo test --locked --features supervision
cargo check --locked --features perf-trace
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked -- -D warnings
cargo clippy --locked --all-targets --features keyring-smoke,live-data-probe -- -D warnings
cd ..

npm ci          # kun nødvendig efter en frisk klon eller et skift i package-lock.json
npm run build
npx vitest run
node scripts/repo-scan.mjs
node scripts/data-dir-guard.mjs verify
```

Forventningen, målt 2026-08-03:

| Trin | Forventet |
|---|---|
| `cargo test --locked` | 637 passed, 0 failed, 1 ignored |
| `cargo test --locked --features supervision` | 658 passed, 0 failed, 1 ignored |
| `npx vitest run` | 78 suiter, 823 tests |
| `npm run build` | `0/30 forbidden strings` |
| `cargo fmt --check` og alle tre clippy | **nul output** |
| `node scripts/repo-scan.mjs` | 0 uklassificerede hits |
| `node scripts/data-dir-guard.mjs verify` | `er uroert af testsuiten — OK` |

Tallene **vokser** når du tilføjer tests. Det er tilvækst, ikke regression. Det der ikke
må ske, er at et tal falder, eller at `fmt`/`clippy` begynder at sige noget.

### Hvorfor tre clippy-kørsler

Det ser overdrevet ud, og det er det ikke. `--all-targets` trækker dev-dependencies ind,
og self-dev-dependency'en i `Cargo.toml` tænder dermed featuren `test-seams` for hele
pakken — så produktionens keyring-vej i `secrets.rs` er cfg-strippet væk og bliver
**aldrig** lintet af den kørsel. Kørslen uden `--all-targets` ser præcis den gren. Den
tredje bygger de to opt-in-gatede testfiler, som ellers ikke kompileres af noget i CI, så
en syntaksfejl dér ville overleve et fuldt grønt ritual.

### `cargo test` må ikke koste dig noget

Barren er skarp: **klon, `cargo test`, og intet uden for repoet er ændret.** To ting
håndhæver det, og begge findes fordi de blev overtrådt før.

**Keyring.** Testsuiten skrev tidligere rigtige credentials i din Windows Credential
Manager. Én kørsel efterlod fem poster; 311 havde hobet sig op. Nu går normale tests til
en in-memory store (`src-tauri/src/secrets/store.rs`), og der findes ikke én kompileret
linje i en test der kan nå credential-storen uden featuren `keyring-smoke`. Vil du køre
den ægte smoke:

```powershell
cd src-tauri
cargo test --features keyring-smoke --test keyring_smoke
```

Den bruger service-navnet `Talminal-smoke`, aldrig `Talminal`, så den kan hverken læse
eller overskrive dine rigtige nøgler. **Efter en smoke-kørsel er `cmdkey /list` det eneste
sande facit** — en sletning i OS-storen kan ikke verificeres gennem keyring-API'et. Vi har
målt en kørsel der efterlod to poster som `cmdkey /list:<target>` viste intakte, mens
keyrings egen `get_password` meldte `NoEntry` for præcis dem.

**Datamappen.** En test der glemmer `common::serial()` opløser sin datasti til din
levende installation og skriver der — tavst. `common::serial()` sandkasser begge rødder,
og `scripts/data-dir-guard.mjs` fanger dem der glemmer det.

At vagten faktisk fælder noget, er **bevist og ikke påstået**:
`src-tauri/tests/data_dir_guard_negative.rs` indeholder tre bevidst glemsomme sonder, og
CI kører dem ved hver eneste kørsel og kræver at hver giver præcis sit udfald. De er
gatet bag `--features live-data-probe` **og** `#[ignore]`, fordi de skriver i den levende
datamappe — kør dem ikke uden at læse filens hoved-kommentar først.

## Pull requests

- **Åbn et issue først** hvis ændringen er større end en fejlrettelse.
- **Én ting ad gangen.** En PR der både retter en bug og omdøber tre moduler, kan ikke
  reviewes.
- **Ritualet skal være grønt** før du beder om review. CI kører det samme, så du sparer
  dig selv en runde.
- **Skriv hvorfor, ikke hvad.** Diffen viser hvad. Commit-beskeden og kommentarerne skal
  forklare hvilken afvejning du traf, og hvad du overvejede i stedet. Kodebasen er skrevet
  sådan; hold stilen.
- **Rør ikke `version`-felterne** i `package.json`, `Cargo.toml` og `tauri.conf.json` —
  de flyttes samlet ved release.
- **Nye tests der rører proces-global tilstand** (datamappen, env-variabler, registryet)
  skal tage `common::serial()` som første linje.

## Licens på det du bidrager med

Projektet er under [Apache License 2.0](LICENSE). **Ved at åbne en pull request erklærer
du, at dit bidrag leveres under samme licens** (inbound = outbound), og at du har ret til
at bidrage med det. Der er ingen CLA at underskrive.

## Sikkerhed

Fandt du et sikkerhedsproblem, så åbn **ikke** et almindeligt issue. Se
[SECURITY.md](SECURITY.md) — kanalen er GitHubs private sårbarhedsrapportering.

## Hvor du finder rundt

[ARCHITECTURE.md](ARCHITECTURE.md) er ét kort over kodebasen: hvad kortene, PTY'en, MCP'en,
browser-værten og voice-pipelinen er, og hvor de bor. Vil du tilføje en agent, står
mekanismen der — det er `src-tauri/src/profiles.rs` du skal kigge på.

Se også [SUPPORT.md](SUPPORT.md) hvis du bare har et spørgsmål, og
[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
