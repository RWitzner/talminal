<div align="center"><a name="readme-top"></a>

# Talminal

**Tal med dine terminaler.**<br/>
Et canvas af agent-terminaler — Claude Code og Codex CLI — som du styrer med stemmen.<br/>
Sig *"opret to kort"*, *"send: kør testene"*, *"luk kort tre"*, og se dem arbejde<br/>
ved siden af hinanden. Til Windows 11.

[![][license-shield]][license-link]
![][windows-shield]
[![][tauri-shield]][tauri-link]
[![][rust-shield]][rust-link]
![][voice-shield]
[![][pr-welcome-shield]][pr-welcome-link]<br/>
[![][ci-shield]][ci-link]
[![][github-stars-shield]][github-stars-link]
[![][github-forks-shield]][github-forks-link]
[![][github-contributors-shield]][github-contributors-link]
[![][github-issues-shield]][github-issues-link]
[![][github-last-commit-shield]][github-last-commit-link]

</div>

<!--
DEMO-HERO — pladsholder.
GIF'en optages i et saniteret miljø (ingen rigtige stier, nøgler eller projektnavne)
inden v0.1.0. Indtil da er der ingen, frem for et billede der lyver om produktet.
-->

> **English notice:** the voice pipeline currently understands **Danish only** — English
> support is planned. Everything else works in English: the app is usable with mouse and
> keyboard without any voice keys, and English issues and pull requests are very welcome.
> The docs and the code comments are in Danish; see
> [CONTRIBUTING.md](CONTRIBUTING.md).

---

## Installation

```powershell
# Prøv den — kør fra den projektmappe du vil arbejde i
npx talminal

# Behold den
npm i -g talminal
```

Kør kommandoen **fra en projektmappe**. Talminal finder projektroden (nærmeste `.git`) og
åbner et canvas for netop det repo. Kører du den et andet sted, får du et canvas for det
sted.

> **Endnu ikke udgivet.** npm-pakken publiceres først ved v0.1.0. Navnet er reserveret,
> men kommandoerne ovenfor virker ikke endnu. Indtil da: byg fra kilde.

### Byg fra kilde

```powershell
git clone https://github.com/RWitzner/talminal.git
cd talminal
npm ci
npm run tauri dev
```

Første byg tager nogle minutter — `rust-toolchain.toml` får `rustup` til at hente den
pinnede toolchain, og Rust-siden skal kompileres helt.

## Forudsætninger

| Krav | Noter |
|---|---|
| **Windows 11 x64** | Kun Windows. ConPTY, Credential Manager og WebView2 er alle i den kritiske sti. |
| **WebView2** | Følger med Windows 11. Testet mod 151.0.4129.59. |
| **Node + npm** | Versionen står i `.nvmrc` (testet: Node 24.15.0, npm 11.5.2). **Node er også et runtime-krav**, ikke kun et byggeværktøj: browser-kort starter `npx @playwright/mcp` mens appen kører. |
| **Rust** | Versionen er pinnet i `rust-toolchain.toml`. Kun nødvendig hvis du bygger fra kilde. |
| **C++ Build Tools** (MSVC) | Til at linke Rust-siden. Kun ved byg fra kilde. |
| **Claude Code CLI** | Testet mod **2.1.220**. |
| **Codex CLI** | **Valgfri.** Testet mod **0.145.0**. Bruger du kun Claude-kort, behøver du den ikke — og så sendes der intet til OpenAI ad den vej. |
| **Mikrofon-tilladelse** | Kun til stemmestyring. Windows' privatlivsindstilling skal tillade mikrofonadgang; ingen app kan omgå den. |

Appen starter fint **uden nogen nøgler**. Kort, terminaler og browser-kort virker; det er
kun stemmevejen der beder om noget.

## Nøgler (BYOK)

Talminal har ingen konto og ingen server. Du indtaster dine egne nøgler under
**Indstillinger → Nøgler**, og de gemmes i Windows Credential Manager — ikke i en fil.
Selve rute-valget står under **Model & routing**.

### Den korte opskrift

Stemmevejen har to trin, og de kan bruge **den samme nøgle**:

1. **Tale-til-tekst** kræver altid en **OpenAI-nøgle**.
2. **Router-modellen**, der forstår hvad du sagde, kan bruge fire ruter. Vælg én:

| Rute | Du skal bruge | Målt p50 |
|---|---|---|
| `vercel` — **standard** | OpenAI-nøgle **+** Vercel AI Gateway-nøgle | 774 ms |
| `openrouter` | OpenAI-nøgle **+** OpenRouter-nøgle | — |
| `google` | OpenAI-nøgle **+** Google AI-nøgle | 680 ms |
| `openai` | **Kun** OpenAI-nøglen | 1322 ms |

**Vil du nøjes med én nøgle, så vælg `openai`-ruten.** Tale-til-tekst og routeren deler
nøgle-slot, så én OpenAI-nøgle dækker hele stemmevejen. Afvejningen er ærlig: den er
omkring dobbelt så langsom som standardruten — 1,3 s mod 0,77 s i median. Begge scorede
41/41 i eval-suiten, så det er hastighed, ikke præcision, der adskiller dem.

**Bemærk på standardruten:** `vercel` affyrer et andet skud efter 1200 ms, hvis det første
ikke er svaret endnu. Det giver lejlighedsvis to fakturerbare requests pr. ytring. De
øvrige ruter gør det ikke. Se [PRIVACY.md](PRIVACY.md).

## Sådan bruger du den

- **Opret et kort:** dobbeltklik på tom canvasflade — eller sig det.
- **Tal til den:** hold **`Ctrl+Shift+Space`** nede, sig din kommando, slip. Genvejen kan
  ændres under Indstillinger → Stemme.
- **Kortnumre er dit ordforråd.** *"Luk kort to og tre."* *"Send til kort et: kør
  testene."* *"Genstart codex-kortet."*
- **Browser-kort** åbnes af agenten selv via MCP, når den har brug for at se en side.

Canvas **starter altid tomt**. Dine projektfiler og agenternes egne tråde overlever, men
korttopologien gendannes ikke — det er en udtalt kontrakt, ikke en fejl.

## Statusline-tap (valgfri)

Vil du se Claude Codes forbrugsprocenter i canvas'ens HUD, kan du installere en tap:

```powershell
node statusline-tap/install.mjs
```

**Læs hvad den gør, før du kører den.** Den **skriver i din globale
`~/.claude/settings.json`** og peger `statusLine` på sig selv. Din hidtidige statusline
forsvinder ikke — den gemmes som delegat og kaldes uændret ved hvert tick. Der tages
backup af filen første gang, og `node statusline-tap/install.mjs --uninstall` ruller
ændringen tilbage.

Kører du aldrig kommandoen, rører intet i dette repo din `settings.json`.

## Hvad du skal vide, før du kører den

- **Et kort er ikke en sandkasse.** Agenten får præcis samme adgang som en terminal åbnet
  i det workspace. Talminal sender heller ingen tilladelses-flag til agenterne, så de
  kører under **din egen** konfiguration — også hvis du har sat dem permissivt op.
  [SECURITY.md](SECURITY.md) skriver det ud.
- **Hvad der sendes hvorhen** står i [PRIVACY.md](PRIVACY.md). Kort: din stemme til
  OpenAI, transskriptet til den rute du valgte, dine prompts til agentens egen udbyder.
  Ingen telemetri.
- **Usigneret binær.** Kommer du til at hente en ZIP i stedet for at bruge npm, kan
  SmartScreen advare. npm-udpakkede filer bærer ikke Mark-of-the-Web, så den vej rammer
  det ikke.

## Dokumentation

| | |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Ét kort over kodebasen — start her |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Ritualet, sprogreglen, PR-forventninger |
| [SECURITY.md](SECURITY.md) | Trusselsmodellen og hvordan du rapporterer |
| [PRIVACY.md](PRIVACY.md) | Dataflow, retention, hvad der bliver liggende |
| [SUPPORT.md](SUPPORT.md) | Spørgsmål og de hyppigste årsager |
| [ASSETS.md](ASSETS.md) | Medieaktiver og deres rettighedsgrundlag |

## Licens

[Apache License 2.0](LICENSE) — se også [NOTICE](NOTICE).

**Lydklippene er undtaget.** De 15 ElevenLabs-genererede klip i `public/reply-clips/` og
`src/assets/` er købt med kommerciel brugsret og **sublicenseres ikke** under Apache-2.0.
Du kan regenerere dem under din egen konto med det medfølgende script. Se
[ASSETS.md](ASSETS.md).

---

<sub>Talminal — *tal* (imperativ af "at tale") + *terminal*. Talk to your terminals.</sub>

<!--
Reference-link-definitioner.

Farverne er appens egne — taget fra `src/`, ikke valgt frit: 0a1220 canvas-mørke,
7ab6e8 blå, 4dd6b7 teal, e8b046 rav, e06058 rød, 9eafc1 stål, 8fa7e0 periwinkle.

RAEKKE 1 er statisk og render altid. RAEKKE 2 spoerger GitHubs API og render
foerst naar repoet er offentligt — indtil da viser de "repo not found".
-->

[license-link]: https://github.com/RWitzner/talminal/blob/main/LICENSE
[license-shield]: https://img.shields.io/badge/license-Apache--2.0-9eafc1?style=flat-square&labelColor=0a1220

[windows-shield]: https://img.shields.io/badge/Windows%2011-x64-7ab6e8?style=flat-square&labelColor=0a1220

[tauri-link]: https://tauri.app
[tauri-shield]: https://img.shields.io/badge/Tauri-2-4dd6b7?style=flat-square&labelColor=0a1220&logo=tauri&logoColor=edf5fc

[rust-link]: https://www.rust-lang.org
[rust-shield]: https://img.shields.io/badge/rust-1.89-e8b046?style=flat-square&labelColor=0a1220&logo=rust&logoColor=edf5fc

[voice-shield]: https://img.shields.io/badge/stemme-dansk-8fa7e0?style=flat-square&labelColor=0a1220

[pr-welcome-link]: https://github.com/RWitzner/talminal/pulls
[pr-welcome-shield]: https://img.shields.io/badge/PRs-welcome-4dd6b7?style=flat-square&labelColor=0a1220

[ci-link]: https://github.com/RWitzner/talminal/actions/workflows/ci.yml
[ci-shield]: https://img.shields.io/github/actions/workflow/status/RWitzner/talminal/ci.yml?branch=main&style=flat-square&label=CI&labelColor=0a1220

[github-stars-link]: https://github.com/RWitzner/talminal/stargazers
[github-stars-shield]: https://img.shields.io/github/stars/RWitzner/talminal?style=flat-square&labelColor=0a1220&color=e8b046&logo=github&logoColor=edf5fc

[github-forks-link]: https://github.com/RWitzner/talminal/network/members
[github-forks-shield]: https://img.shields.io/github/forks/RWitzner/talminal?style=flat-square&labelColor=0a1220&color=7ab6e8&logo=github&logoColor=edf5fc

[github-contributors-link]: https://github.com/RWitzner/talminal/graphs/contributors
[github-contributors-shield]: https://img.shields.io/github/contributors/RWitzner/talminal?style=flat-square&labelColor=0a1220&color=4dd6b7

[github-issues-link]: https://github.com/RWitzner/talminal/issues
[github-issues-shield]: https://img.shields.io/github/issues/RWitzner/talminal?style=flat-square&labelColor=0a1220&color=e06058

[github-last-commit-link]: https://github.com/RWitzner/talminal/commits/main
[github-last-commit-shield]: https://img.shields.io/github/last-commit/RWitzner/talminal?style=flat-square&labelColor=0a1220&color=9eafc1
