# Privatliv og dataflow

Talminal indsamler ingenting. Der er ingen telemetri, ingen analytics og ingen
phone-home — appen taler kun med de tjenester du selv har givet den en nøgle til, og med
de sider du selv åbner i et browser-kort.

Men den *sender* naturligvis noget, for det er hele pointen. Her står præcis hvad, hvorhen
og hvornår.

## Hvad der forlader din maskine

| Hvad | Hvorhen | Hvornår |
|---|---|---|
| **Din stemme** (PCM16, 24 kHz) | `wss://api.openai.com/v1/realtime` — modellen `gpt-4o-transcribe` | Kun mens du holder push-to-talk nede |
| **Transskriptet** af det du sagde | Den router-udbyder du har valgt (se nedenfor) | Efter hver ytring |
| **Dine prompts og det agenten læser af dine projektfiler** | Anthropic, via Claude Code | Når du bruger et Claude-kort |
| **Samme, til OpenAI** | OpenAI, via Codex CLI | Når du bruger et codex-kort |
| **De sider et browser-kort besøger** | De pågældende websteder | Når du eller agenten navigerer |

Talminal er et **BYOK-produkt** (bring your own key). Der er ingen konto, ingen server hos
os, og ingen mellemmand: dine requests går direkte fra din maskine til den udbyder, hvis
nøgle du har indtastet.

## Router-udbyderen er dit valg

Transskriptet skal klassificeres af en sprogmodel for at blive til en kommando. Du vælger
ruten i Indstillinger, og valget bestemmer hvem der ser teksten:

| Rute | Endpoint | Model |
|---|---|---|
| `vercel` (standard) | `ai-gateway.vercel.sh` | `google/gemini-3.1-flash-lite` |
| `google` | `generativelanguage.googleapis.com` | `gemini-3.1-flash-lite` |
| `openrouter` | `openrouter.ai` | `google/gemini-3.1-flash-lite` |
| `openai` | `api.openai.com` | `gpt-5.6-luna` |

Tabellen bor ét sted i koden (`src-tauri/src/providers.rs`) — der er ingen fri base-URL,
så en nøgle kan ikke sendes til en forkert destination ved en tastefejl.

**Hedging: standard-ruten kan koste dig to fakturerbare requests.** På `vercel`-ruten —
og kun dér — affyres et andet skud mod gatewayen efter 1200 ms, hvis det første ikke er
svaret endnu. Det er en afvejning mod haletidsudsving, og det betyder at én ytring
lejlighedsvis faktureres to gange. De øvrige tre ruter hedger ikke.

## Hvad der bliver liggende på din maskine

De vigtigste filer under `%LOCALAPPDATA%\Talminal\` — listen er ikke udtømmende:

| Sti | Indhold |
|---|---|
| `settings.json` | Dine indstillinger. Ingen nøgler. |
| `projects\<slug>\workspace.json` | Kort-layout pr. projekt |
| `projects\<slug>\threads\<id>.jsonl` | **Rå agent-til-agent-beskeder**, append-only |
| `projects\<slug>\browser-profiles\` | **Cookies, localStorage og cache** for hver side et browser-kort har været på. Ryddes næste gang du åbner *det* projekt — ikke når du lukker appen |
| `projects\<slug>\worker-mcp\<kort>.json` | MCP-config pr. kort. Bærer et **loopback-sessions-token**, se noten nedenfor |
| `hud\usage.json` | Claude Codes forbrugsprocenter, hvis du har slået statusline-tap'en til |
| `hud\context\<kort>.json` | Kontekstvindue-procent pr. kort, samme betingelse |
| `hud\tap.mjs`, `hud\lib.mjs`, `hud\tap-config.json` | Statusline-tap'ens egne filer, hvis du har installeret den |
| Diverse små tilstandsfiler | `last_project`, `active_workspace.json`, `window_geometry.json`, `project.json`, `instance.json`, `status.json`, låsefiler. Vindues- og projekt-tilstand, **inkl. absolutte stier til dine projekter**. Ingen hemmeligheder |

Bygger du med `--features supervision`, kommer `signals\` og `presence\` oveni.

Dine **API-nøgler** ligger ikke her, men i Windows Credential Manager under service
`Talminal`. Se [SECURITY.md](../.github/SECURITY.md) for hvad det beskytter mod.

**Én undtagelse, så den sætning ikke misforstås:** `worker-mcp\<kort>.json` ovenfor
bærer et `Authorization: Bearer …`-token. Det er *ikke* en API-nøgle til en udbyder — det
er kortets identitet over for Talminals egen loopback-MCP-server, og det er ugyldigt i det
øjeblik appen lukker. Filen slettes når kortet lukkes, og hele mappen ryddes ved næste
opstart.

### Tråd-retention

`threads\<id>.jsonl` er **append-only og roterer ikke**. Beskeder mellem agenter bliver
liggende indtil du selv sletter filen. Der er i dag **ingen sletteknap i UI'et** — det er
en kendt mangel, og den er på listen. Vil du rydde op nu, kan du slette filerne i mappen
manuelt, mens appen er lukket.

### Fejlfindings-dumps

Slår du statusline-tap'ens debug til (`TALMINAL_TAP_DEBUG=1` eller en markørfil
`tap-debug.on`), skrives der ved hvert statusline-tick en
`tap-debug-<sessionId>.json` med den **rå payload** fra Claude Code, miljøtilstand og
skrive-udfald. Det er nyttigt til fejljagt og bør slås fra igen bagefter — slet
markørfilen og dumpene når du er færdig.

## Codex-kort arver din egen konfiguration

Talminal sender **ingen tilladelses-flag** til hverken Claude Code eller Codex CLI —
`spawn_command` er `["claude"]` og `["codex"]`, uden argumenter. Begge agenter kører
derfor under din egen opsætning. Har du konfigureret codex i en permissiv tilstand, hvor
den ikke spørger om lov, **arver kortene den tilstand**, og Talminal kan hverken se eller
ændre det.

Codex CLI er i øvrigt en **valgfri** forudsætning: bruger du kun Claude-kort, sendes der
intet til OpenAI ad den vej. Stemmevejen bruger OpenAI til tale-til-tekst uanset hvilken
agent du kører.

## Sig ikke dine hemmeligheder højt

Push-to-talk sender lyd til OpenAI, og transskriptet videre til din router-udbyder.
**Dikter derfor ikke API-nøgler, adgangskoder eller andet fortroligt.** Det samme gælder
det du skriver til et kort: teksten går til agentens udbyder på helt normal vis.

## Ændringer

Dette dokument beskriver **det der er bygget i dag**, ikke det der er planlagt. Ændrer
dataflowet sig, ændres filen her i samme pull request.
