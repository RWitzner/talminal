# Talminal — arkitektur

Ét kort over kodebasen, skrevet til dig der lige har klonet og står i godt 100 Rust-filer.
Hver påstand her er læst i koden, ikke hentet fra en spec. Er noget uenigt med koden,
er koden rigtig — og så er det en fejl i denne fil.

Dokumentet er på dansk, ligesom doc-kommentarerne i kildekoden. Se
[CONTRIBUTING.md](../.github/CONTRIBUTING.md) for hvorfor, og for at engelske issues og PR'er er
velkomne.

## Hvad produktet er

En Tauri 2-app til Windows: et canvas af **kort**, hvor hvert kort er en agent-terminal
(Claude Code eller Codex CLI), et browservindue eller en agent-til-agent-chat. Du styrer
det med stemmen på dansk, eller med musen og tastaturet.

Backend er Rust (`src-tauri/src/`), frontend er React + TypeScript (`src/`). Grænsen er
Tauri-kommandoer og -events; **al autoritativ tilstand bor i Rust**, og frontenden er en
projektion der genhentes ved event.

## Kortet over kortet

```
                      ┌──────────────────────────────────────────┐
   Stemme ──────────► │  src/voice/         pipeline · ptt · stt │
   (PTT-hotkey)       │                     router · dispatch    │
                      └───────────────┬──────────────────────────┘
                                      │ Tauri-kommandoer
   ┌──────────────────────────────────▼──────────────────────────┐
   │  src/App.tsx · CanvasSurface.tsx      React-fladen          │
   │      Card.tsx · BrowserCard.tsx · ChatCard.tsx              │
   └──────────────────────────────────┬──────────────────────────┘
                                      │  invoke / listen
   ┌──────────────────────────────────▼──────────────────────────┐
   │  src-tauri/src/main.rs            kommandoer + wiring       │
   ├─────────────────────────────────────────────────────────────┤
   │  registry.rs   sandheden om kort (nummer, navn, backend)    │
   │  pty.rs        én ConPTY pr. terminal-kort                  │
   │  submit.rs     tekst og Enter som to writes                 │
   │  browser_host.rs  WebView2-børn + CDP                       │
   │  mcp.rs        JSON-RPC-flade agenterne kalder tilbage på   │
   │  workspaces/   ét synligt vindue ad gangen, på tværs af pid │
   │  threads/      agent-til-agent-beskeder, append-only JSONL  │
   │  profiles.rs   HVAD EN AGENT ER  ◄── start her              │
   └─────────────────────────────────────────────────────────────┘
```

## Kort og canvas

Et **kort** er en post i et proces-globalt registry (`registry.rs:256`) med et nummer, et
navn `card-N` og én af tre backends: Terminal, Browser eller Chat (`registry.rs:176`).

- **Numre genbruges.** Allokatoren tager det laveste ledige nummer, ikke det næste i rækken
  (`registry.rs:289`). Luk kort 3–6, opret to nye, og du får 3 og 4 igen. Hæng derfor
  aldrig politik eller tokens på nummeret alene.
- **Wire-navnet er `card-N`.** Det danske visningsnavn "Kort N" laves ét sted
  (`src/cardLabel.ts:13`) og kun til visning. Omdøber du identifieren, brækker MCP-fladen,
  `spawn_card`/`close_card` og React-nøglerne på én gang.
- **Layout persisteres ikke.** `WorkspaceCard.x/y/w/h` findes på wiren, men fladen kalder
  aldrig `update_card_geometry` — layoutet er et afledt CSS-grid
  (`src/canvas/responsiveLayout.ts:132`). Rediger ikke geometrien i den tro at kortene
  flytter sig.
- **Canvas starter altid tomt.** `startup_load()` sætter tilstanden til `empty_file()`
  (`workspace.rs:500`). Kort fra sidste session genopstår ikke. Projektfiler og agenternes
  egne tråde overlever; korttopologien gør ikke.
- **Der er ingen polling af kortlisten.** Enhver backend-ændring der skal ses, *skal*
  emitte et event som lytter-effekten i `src/App.tsx` abonnerer på (`cards-changed`,
  `browser-card-updated`, `workspace-revealed`, `browser-card-dead`) — ellers ligger
  kortet usynligt.

## PTY — den ene ejer af terminalen

`pty.rs` ejer præcis én ConPTY pr. terminal-kort. Tre empiriske krav fra Windows styrer
designet, og de er skrevet ned i filens egen hoved-kommentar (`pty.rs:4-16`):

1. Den første `ESC[6n`-cursorforespørgsel skal besvares af værten selv, ellers blokerer alt.
2. Reader-tråden skal **altid** dræne — flow control er blokerende backpressure.
3. Der kommer ingen reader-EOF ved child-exit, kun efter at masteren droppes.

`submit.rs` skriver prompt-tekst og det afsluttende `\r` som **to separate writes** med en
verificeret TUI-redraw imellem (`submit.rs:134`, `:195`). Uden det lander Enter før
agentens tekstfelt har modtaget teksten. Gabet mellem de to writes kommer fra profilens
`submit_gap_ms`.

**Env-politikken ved spawn** (`pty.rs:215-229`): en ubetinget nested-scrub
(`CLAUDECODE`, `CLAUDE_CODE_*`, `CODEX_SANDBOX*` — `profiles.rs:82`) plus profilens
to-lags deny-liste. Kontrakten er ærlig og står i [SECURITY.md](../.github/SECURITY.md): **et kort
giver agenten præcis samme adgang som en terminal åbnet i det workspace** — hverken mere
eller mindre. Deny-listen er oprydning, ikke en sandkasse.

## MCP og browser-kort

En agent i et kort får ved spawn injiceret to MCP-servere: Talminals egen, og en
Playwright-MCP der peger på kortets eget CDP-endpoint.

- `mcp.rs` er en håndrullet JSON-RPC 2.0-flade over `tiny_http` på 127.0.0.1
  (`mcp.rs:251`) med fire `browser_card_*`-tools og tre tråd-tools (`mcp.rs:613`).
  Identiteten er et Bearer-token pr. kort.
- `worker_mcp.rs` skriver selve injektionen, og **den er forskellig pr. profil**:
  Claude får en config-fil, codex får `-c`-overrides. Hvilken vej der bruges, står som
  `McpInjection` i profilen (`profiles.rs:60`).
- `browser_host.rs` oversætter tool-kald til rigtige WebView2-børn, grupperet i **scopes**
  — én browserproces pr. agent, holdt i live af en usynlig **keeper**-webview. En poller
  afstemmer registry-kort mod CDP's target-liste hvert 2. sekund.

## Voice-pipelinen

Én tur: tastetryk → mikrofon → STT → router-model → dispatch → lydklip.

| Trin | Hvor |
|---|---|
| PTT-hotkey (niveau-sampling hvert 5 ms) | `src-tauri/src/wake_hotkey.rs` |
| Arbitration mellem DOM-events og poller-kanter | `src/voice/hotkeyBridge.ts` |
| Optagelse + resampling til PCM16 24 kHz | `src/voice/ptt.ts` |
| Streamende dansk STT over WebSocket | `src/voice/stt.ts` |
| Klassifikation til 1–10 intents (tvunget function-tool) | `src/voice/router.ts` |
| Deterministisk dispatch til Tauri-kommandoer | `src/voice/dispatch.ts` |
| Svar som ét af 14 forudindspillede klip | `src/voice/replies.ts` |

**Dikteringen er den samme tur med de tre sidste led skåret af.** `src/voice/dictation.ts`
genbruger `createPtt` — samme mikrofon, samme STT — men afleverer transskriptet råt i stedet
for at route og dispatche det. Hvor det lander, afgøres af `voice/dictationTarget.ts`, og
selve indsættelsen sker gennem ét window-event (`src/dictationInsert.ts`) med to modtagere:
`Card.tsx` kalder `term.paste()`, `ChatCard.tsx` lægger til sin kladde. Begge composere er
React-ejede, og det er derfor der er et event og ikke en ref.

**Polleren har to slots**, ikke én: `HotkeySlot::{Ptt, Dictation}`, hver med sin
kant-detektor, vk-cache og versions-tæller (`wake_hotkey.rs`). Tællerne er bevidst adskilte
— en fælles ville lade en ændring af den ene genvej sluge release-kanten på et igangværende
hold i den anden. `SUSPENDED` er derimod fælles, fordi `HotkeyRecorder` skal kunne gøre
begge tavse mens den optager en ny binding. At to genveje ikke må dele trigger-tast
håndhæves af `wake_hotkey::collides` og er **ikke** en lighedstest: combo-matchet er
subset, så ét `Ctrl+Shift+Space`-tryk fyrer også en genvej bundet til `Ctrl+Space`.

**Hver slot har to bindinger**, en primær og en valgfri alternativ, og begge er aktive
samtidig. Det findes fordi den samme funktion bruges to fysiske steder — en musetast ved
skrivebordet, en controller-trigger i headsettet — og alternativet ville være at skifte
binding i indstillingerne hver gang. Der er derfor op til fire genveje og **seks** par at
kollisionstjekke: to inden for hver slot og fire på tværs.

Ingen detektion af hvilket sted brugeren er: en gamepad-binding kan ikke fyre når der ingen
pad er (`PadReader` returnerer `PadState::default()`), og fokus-gaten sikrer at en anden
apps controller-tryk aldrig når Talminal. Bindingen deaktiverer altså sig selv.

`SlotArbiter` voldgifter mellem en slots to bindinger: den der startede holdet ejer det til
den slippes, og kun ejeren kan lukke det. Uden den lås ville to samtidige tryk give to
`press` for én mikrofon. Typen ligger bevidst i den rene del af `wake_hotkey.rs` og ikke i
poller-tråden — ellers kunne den vigtigste logik i modulet ikke testes. Mister den
ejerskabet (bindings-skift eller suspension midt i et hold), **skal** den udsende et
syntetisk `release`: uden det står mikrofonen åben og slotten er permanent låst, fordi
`reset()` sætter `suppress_until_keyup` og ejerens detektor derfor aldrig selv når at melde
slip. Den samme ejerskabsregel findes i `hotkeyBridge.ts` for DOM-vejen.

Kun én af de to *slots* må holde mikrofonen ad gangen. Ejerskabet følger holdet — ikke
pipelinens tilstand, som er `speaking` længe efter tasten er sluppet — og gaten sidder i
`App.tsx`.

**Ruterne bor ét sted — men navnene gør ikke.** `providers.rs` er den eneste kilde til
endpoints og modelnavne: `STT_ROUTES` (2 ruter — samme udbyder og samme realtime-session,
kun modellen skifter) og `ROUTER_ROUTES` (4 ruter). De når fladen over wiren
(`resolve_voice_routes`, `workspace.rs:65` → `voice_routes`, `types.ts:118`), så dér er der
ingen kopi at holde synkron. Undtagelsen er **navnene**: de fire `KEY_SLOT_*`
(`providers.rs:76-79`) står ordret igen i `src/Settings.tsx:40-43`, og slugs står igen i
`STT_CHOICES` og `ROUTING_CHOICES` (`Settings.tsx:610` og `:620`). Tilføjer du en udbyder
eller en model, skal begge sider røres.

Drift ER sket her før: `types.ts:113` manglede `"reasoning_off"` i `decoration`-unionen fra
GPT-5.6-ruten blev skrevet ind, til det blev fanget 2026-08-04. Ingen test vogter
spejlingen — kun læsning.

Nøglerne selv ligger i Windows Credential Manager under service `"Talminal"`
(`secrets.rs:702`) og krydser **aldrig** WebView-grænsen — `load_secret`-kommandoen
svarer `true`/`null`, ikke værdien.

## Tilstand på disken

To rødder, og forskellen betyder noget:

| Funktion | Hvad den opløser | Hvad der bor der |
|---|---|---|
| `project::global_base()` | `%LOCALAPPDATA%\Talminal` | `settings.json`, `last_project`, `active_workspace.json`, `hud\` |
| `cards::talminal_base()` | `%LOCALAPPDATA%\Talminal\projects\<slug>` | `workspace.json`, `threads\`, kortenes state |

Et **projekt** er en mappe: roden findes ved at gå op til nærmeste `.git`
(`project.rs:28-42`), og identiteten er `<basename>-<hex8 af SHA-256 over stien>`
(`project.rs:50-64`). Et **workspace** er én kørende app-proces bundet til ét projekt.
Præcis én proces pr. slug håndhæves af en navngiven mutex plus en `.lock`-fil
(`instance.rs:86-138`), og præcis ét *synligt* vindue ad gangen af en filbaseret
request/ack-protokol, hvis rene tilstandsmaskine ligger uden IO i
`workspaces/protocol.rs`.

Begge env-variabler (`TALMINAL_HOME`, `TALMINAL_GLOBAL_HOME`) kan overrides — det er
netop det testsandkassen bruger. Se `src-tauri/tests/common/mod.rs` og
[CONTRIBUTING.md](../.github/CONTRIBUTING.md).

## Agent-profil-seamet — start her, hvis du vil tilføje en agent

`profiles.rs` er den ene tabel der beskriver *hvad en agent er*. Den er rene `static`-data
plus fire opslagsfunktioner og uden låse — derfor er den næsten helt unit-testbar. To af
dem rører filsystemet, og det er værd at vide når du skriver en test: `resolve_spawn_program`
søger PATH igennem (`profiles.rs:240`) og codex-fallbacken læser npm-vendor-mappen
(`profiles.rs:170`). Resten er ren opslag.

`AgentProfile` (`profiles.rs:23`), felt for felt, med den der faktisk læser det:

| Felt | Betydning | Læses af |
|---|---|---|
| `spawn_command` / `resume_command` | Hvad der køres | `registry.rs` (kopieres ind i kortets config) |
| `env_deny_prefixes` / `env_deny_exact` | To-lags credential-scrub | `pty.rs:215` |
| `submit_gap_ms` | Gab mellem tekst og `\r` | `submit.rs` |
| `transcript_root` | Hvor agentens transcripts bor | `transcripts.rs` |
| `readiness` | TUI-markører for "klar til input" | `prompt_readiness.rs` |
| `mcp` | `ClaudeFlags` eller `CodexOverrides` | `worker_mcp.rs` |
| `exe_fallback` | Hvis binæren ikke er på PATH | `resolve_spawn_program` (`profiles.rs:237`) |
| `attention_patterns` | Bytes der betyder "agenten venter på dig" | `workspaces/attention.rs` |

De to profiler er forskellige på præcis de steder man skulle tro: Claude kører i alt-screen
med `❯`+NBSP som prompt, codex kører **inline uden alt-screen** med en ESC[1m-sekvens
(`profiles.rs:65-77`, målt i en spike mod codex-cli 0.145.0).

**Vil du tilføje en tredje agent**, er profilen ikke nok. Tre steder mere skal røres, og de
er nemme at overse:

1. `transcripts.rs` router stadig på et hardkodet `claude`/`codex`-match.
2. `workspace.rs` har en uafhængig allowlist for `default_agent` i settings — står din
   profil ikke der, kan den ikke vælges.
3. Nested-scrubben i `pty.rs` er **ubetinget** og læser ikke profilen
   (`profiles.rs:82`) — skal din agent skjules for sig selv, hører mønsteret hjemme dér.

## Hvad der ikke er ledningsforbundet

Ærligt, så du ikke bruger en aften på at lede:

- **`restore.rs`** har ingen produktionskalder. Restore-on-launch er afkoblet
  (`main.rs`-kommentaren siger det selv), og logikken er parkeret, ikke slettet.
- **Supervision** (`epoch.rs`, `signals.rs`, `presence.rs`) kompileres kun med
  `--features supervision` og er en parkeret ejer-beslutning. Default-tilstanden stubber
  den frosne IPC-kontrakt (`control.rs`).
- **`transcripts.rs`' codex-vej** er strukturelt død: stien peger på et layout codex ikke
  bruger, og fejlen er tavs. Kendt, og på listen.

## Test-arkitekturen, kort

To ting adskiller denne kodebase fra det du måske forventer, og begge findes fordi de
blev overtrådt før:

- **Datamappen er proces-global tilstand.** Tests der rører den, tager `common::serial()`
  som første linje; den sandkasser begge rødder. `scripts/data-dir-guard.mjs` fanger dem
  der glemmer det — og at vagten faktisk fælder noget, er bevist af tre bevidst glemsomme
  sonder i `tests/data_dir_guard_negative.rs`, som CI kører ved hver kørsel.
- **Secret-storen er bag et seam.** Normale tests rører aldrig Windows Credential Manager;
  de kører mod en in-memory store (`src/secrets/store.rs`). Den ægte keyring nås kun af
  `tests/keyring_smoke.rs` bag `--features keyring-smoke` og under et *andet* service-navn.

Begge er beskrevet i [CONTRIBUTING.md](../.github/CONTRIBUTING.md) sammen med hele verifikationsritualet.
