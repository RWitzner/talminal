# Talminal statusline-tap

Claude Codes `statusLine`-kommando modtager en JSON-payload på stdin med bl.a.
`rate_limits` (5-timers- og ugevindue, kun for abonnenter, efter sessionens
første API-svar). Tap'en her udnytter det: den skriver udtrækket til den
GLOBALE `%LOCALAPPDATA%\Talminal\hud\usage.json` og delegerer derefter
uændret til den statusline-kommando der var konfigureret i forvejen. Canvas'ens
usage-HUD (`src/UsageHud.tsx` + `read_usage_snapshot`-commanden) læser samme fil.

Der er ÉN fil for alle sessioner: usage.json er konto-niveau-data, og
canvas'en sætter `TALMINAL_HOME` per-projekt ved opstart — derfor ignorerer
tap'en bevidst den variabel (ellers ville kort-sessioner skrive projekt-stien
mens eksterne terminaler skrev den globale). Både tap (skriver) og Rust-læseren
respekterer `TALMINAL_GLOBAL_HOME` (ikke-tom) som override af basen — mest til
test; normalt bruges `%LOCALAPPDATA%\Talminal`.

## Install

    node statusline-tap/install.mjs

- Kopierer `tap.mjs`/`lib.mjs` til `%LOCALAPPDATA%\Talminal\hud\`.
- Gemmer den eksisterende `statusLine.command` som delegat i `tap-config.json`.
- Tager backup af `~/.claude/settings.json` → `settings.json.talminal-tap.bak`
  (kun første gang, og kun hvis filen fandtes i forvejen).
- Peger `statusLine` på tap'en. Gælder NYE Claude Code-sessioner.

Geninstall er idempotent (genopfrisker scripts, bevarer delegaten) — kør den
efter ændringer i tap.mjs/lib.mjs.

### Opt-in: hvad kommandoen rører

Tap'en installeres ALDRIG automatisk. Intet sker før du selv kører kommandoen
ovenfor, og den gør præcis to ting uden for sin egen hud-mappe:

- Den **skriver i din `~/.claude/settings.json`** og sætter `statusLine` til
  tap'en. Det er din globale Claude Code-konfiguration, ikke en projektfil.
- Den tager en backup af den fil til `~/.claude/settings.json.talminal-tap.bak`
  første gang (kun hvis `settings.json` allerede fandtes), så originalen kan
  gendannes i hånden.

Din hidtidige statusline forsvinder ikke: den gemmes som delegat og kaldes
uændret ved hvert tick. `node statusline-tap/install.mjs --uninstall` ruller
ændringen tilbage. Kører du aldrig install-kommandoen, rører intet i dette
repo din `settings.json`.

## Uninstall

    node statusline-tap/install.mjs --uninstall

Gendanner `statusLine` til delegaten (eller fjerner den, hvis der ingen var).
Er `statusLine` ikke tap'ens, er kommandoen en no-op og rører ingenting.
Er tap'en aktiv, mens selve `tap-config.json`-FILEN er væk, AFBRYDER kommandoen
i stedet uden at røre `settings.json` — dér ville den fjerne statuslinen, ikke
gendanne den (se Fejlfinding).

## usage.json-kontrakt (v1 — FROSSEN)

    {
      "version": 1,
      "writtenAt": "2026-07-21T19:05:12.345Z",
      "fiveHourPercent": 5,
      "fiveHourResetsAt": "2026-07-21T22:20:00.000Z",
      "weeklyPercent": 1,
      "weeklyResetsAt": "2026-07-28T07:00:00.000Z",
      "sessionId": "…" 
    }

Procenter 0-100; `weeklyPercent`/`weeklyResetsAt`/`sessionId` kan være null;
uden `rate_limits` i payloaden skrives filen slet ikke (API-nøgle-brugere,
friske sessioner). Filen indeholder ingen hemmeligheder.

## context v1 (per-kort badge)

KUN kort-sessioner (canvas'ens PTY-spawn sætter `TALMINAL_SESSION_ID` =
kortnavnet, som tap'en arver) skriver derudover
`%LOCALAPPDATA%\Talminal\hud\context\<kortnavn>.json` med payloadens
`context_window.used_percentage` — én fil per kort, restart overskriver:

    {
      "version": 1,
      "writtenAt": "2026-07-22T09:12:00.000Z",
      "cardName": "kort-3",
      "runId": "…",
      "sessionId": "…",
      "cwd": "C:\\projekt",
      "usedPercent": 18,
      "windowSize": 1000000,
      "modelDisplayName": "Fable 5"
    }

`cardName`/`cwd`/`usedPercent` er obligatoriske (canvas'ens badge joiner på
navn+cwd); resten kan være null. Eksterne terminaler skriver aldrig
(ingen env-var). Filer ældre end 7 dage prunes opportunistisk ved skrivning.
Læses af `read_context_snapshots` (context_hud.rs) og vises som procent-badge
i kortets header (ContextBadge.tsx).

## Fejlfinding

- HUD viser `—` med kørende kort: den globale
  `%LOCALAPPDATA%\Talminal\hud\usage.json` mangler eller er >60 min gammel.
  Tjek at `statusLine` i `~/.claude/settings.json` peger på tap'en, og at en
  CC-session har produceret mindst ét svar siden install. Alle sessioner
  (kort-CC og eksterne terminaler) skriver samme globale fil.
- Kontoskift (`/login`): tap'en følger automatisk med — payloadens
  `rate_limits` afspejler den nye konto fra sessionens næste API-svar, også i
  allerede kørende sessioner. Vent på næste svar + op til 30 s HUD-poll.
  Opdaterer den slet ikke, er sessionen sandsynligvis startet FØR
  tap-installen: den kører så stadig den gamle statusline-kommando og rører
  aldrig usage.json — genstart kortet/terminalen.
- Rodårsags-værktøj: opret `tap-debug.on` i hud-mappen (eller sæt
  `TALMINAL_TAP_DEBUG=1`) — hvert statusline-tick skriver så
  `tap-debug-<sessionId>.json` ved siden af tap'en med rå payload, env-tilstand
  og skrive-udfald (`written`/`no-snapshot`/`skipped-no-base`/`rename-failed`).
  Slet markørfilen og dumpene når jagten er slut. NB: manuelle tap-tests skal
  pipe BOM-frit — PowerShell-pipes prepender UTF-8-BOM som knækker
  JSON-parsning; brug Git Bash og `printf`.
- Statuslinen i terminalen forsvandt: kør `--uninstall` (eller gendan fra
  `.bak`-filen) og fejlmeld — tap'en skal delegere ubetinget.
- Installeren afbryder med "tap-config.json mangler": afbrydelsen rammer kun
  når selve config-FILEN er væk (en fil med `delegate: null` er legitim og
  geninstallerer fint). `--uninstall` afbryder på samme tilstand og er altså
  ingen udvej — den ville FJERNE `statusLine`, ikke gendanne den. Findes
  `settings.json.talminal-tap.bak`, er den den eneste vej til den oprindelige
  statusline: gendan `statusLine` derfra. Findes den ikke — backuppen tages kun
  ved FØRSTE install og kun hvis `settings.json` fandtes i forvejen — er den
  gamle kommando ikke gemt nogen steder; skriv den ind i
  `~/.claude/settings.json` i hånden, eller læg den i `tap-config.json` som
  `{"version":1,"delegate":"<kommando>"}` og kør `--uninstall` igen.
