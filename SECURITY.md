# Sikkerhed

## Sådan rapporterer du en sårbarhed

**Åbn ikke et almindeligt issue.** Brug GitHubs private sårbarhedsrapportering:

→ Fanen **Security** i dette repo → **Report a vulnerability**

Det giver en privat tråd med vedligeholderen, en historik og en vej til et CVE, hvis det
bliver relevant. Der er ingen mailadresse at skrive til, og det er et bevidst valg:
projektet har intet domæne, og en privat mail i en offentlig fil er både en spam-magnet
og et dårligt arkiv.

Skriv gerne på dansk eller engelsk. Sig hvad du gjorde, hvad du forventede, og hvad der
skete. Jeg er én person og svarer så hurtigt jeg kan — ikke inden for et SLA.

## Trusselsmodellen, ærligt

Det her afsnit er skrevet for at du kan træffe en informeret beslutning, ikke for at
berolige dig.

### Et kort er ikke en sandkasse

**Et kort giver agenten præcis samme adgang som en terminal åbnet i det workspace** —
hverken mere eller mindre. Talminal eskalerer intet: der er ingen indeslutning, ingen
filsystem-jail og ingen rettighedsbegrænsning oven på det, din bruger allerede har.
Agenten kan læse og skrive alt hvad du kan.

Det er en **egenskab**, ikke en mangel vi ikke er nået til. Værktøjet findes for at give
en agent en terminal; en terminal der ikke kan noget, er ikke en terminal. Men du skal
vide det, før du åbner et browser-kort på en side du ikke stoler på.

Vi er faktisk lidt strammere end en almindelig terminal: ved spawn fjernes en deny-liste
af credential-variabler fra child-miljøet (`CLAUDE*`, `OPENAI_*`, `AWS_ACCESS/SECRET/SESSION`,
`VERCEL_*` plus fem præcise tokens), og en ubetinget scrub fjerner de variabler der ville
få en agent til at tro, den er sin egen under-session. **Sælg det ikke som sikkerhed.**
En agent der kan køre `cat`, er lige langt uanset om `DATABASE_URL` står i miljøet eller
i en `.env.local` to mapper væk. Grænsen der tæller, er filsystemet og brugertokenet, og
den er identisk med terminalens.

### Talminal tilføjer ingen tilladelses-flag

`spawn_command` er `["claude"]` og `["codex"]` — uden flag (`src-tauri/src/profiles.rs`).
Begge agenter kører derfor under **din egen konfiguration**. Har du sat codex eller Claude
Code op i en permissiv tilstand, arver kortene den. Talminal hverken strammer eller løsner
den indstilling, og kan ikke se hvad du har valgt.

### Hvad Credential Manager beskytter mod — og hvad den ikke gør

Dine API-nøgler ligger i Windows Credential Manager under service `Talminal`, ikke i en
fil i klartekst, og nøgleværdierne krydser aldrig WebView-grænsen — panelet får
`true`/`null`, ikke værdien.

**Credential Manager beskytter mod lagring i klartekst. Den beskytter ikke mod andre
processer under den samme Windows-bruger** — herunder enhver agent du selv har givet
shell-adgang i et kort. Kører en agent amok, er dine nøgler inden for dens rækkevidde,
præcis som de ville være for ethvert andet program du selv startede.

### Den ærlige restforskel fra en terminal

Det er **inputkanaler, ikke rettigheder**. En terminal modtager kun tekst fra dig. Et kort
modtager også tekst fra de websider agenten selv styrer, og fra en anden agent via
`card_say`. Det er en gradsforskel — Claude Code i en terminal har også WebFetch — men det
er dét, du skal kunne se.

### Netværk

Appen taler med de endpoints der står i `src-tauri/src/providers.rs`, og med de sider du
selv åbner i browser-kort. Der er ingen telemetri og ingen phone-home. Se
[PRIVACY.md](PRIVACY.md) for hvad der sendes hvorhen.

## Kendte, uadresserede svagheder

Dette er en v0.1 fra ét menneske. Følgende er kendt og ikke lukket:

- **MCP-transporten** håndterer requests sekventielt uden read-timeout. En lokal proces —
  eller et prompt-injiceret kort — kan holde loopet og dermed fryse MCP for de andre kort.
  Origin-gaten afviser browsere før body-læsningen, så en fremmed webside er ikke den
  realistiske angriber; et kompromitteret kort er.
- **CSP** er ikke strammet for den bundlede frontend.
- **Worker-config-filerne** bærer et levende Bearer-token på disken for Claude-profilen.

De er på listen. Finder du noget der ikke er, så rapportér det ad kanalen ovenfor.

## Understøttede versioner

Kun `main`. Der er endnu ingen udgivne versioner at bagudpatche.
