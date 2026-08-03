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

### Hvorfor et browser-kort ikke kan nå appens kommandoer

Løftet ovenfor — *"hverken mere eller mindre"* — hviler på en grænse det er værd at kende,
fordi den ikke er indlysende fra koden.

Browser-kort er child-webviews **inde i** hovedvinduet, og Tauri injicerer sit
`__TAURI_INTERNALS__`-objekt ubetinget i dem alle. Det der afviser en fremmed side er
Tauris egen kontrol af om origin er lokal: en side på `example.com` får ingen ACL og
afvises ved kommando-dispatch. Men appens capability er målrettet **vinduet**, ikke den
enkelte webview — så et kort der stod på appens *egen* origin ville være lokalt og få hele
kommandofladen. Det ville være en eskalering forbi terminal-grænsen, altså præcis dét
løftet siger ikke sker.

Derfor afvises appens egne origins nu eksplicit — både når et kort oprettes og ved enhver
senere navigation, inklusive redirects. Listen står i `browser_host.rs` som `APP_ORIGINS`
og dækker både den bundlede app og dev-serveren; en test håndhæver at den ikke kan drifte
fra `tauri.conf.json`.

**Det er stadig ét lag, ikke to.** Et app-ACL-manifest med webview-scopede capabilities
ville gøre grænsen strukturel frem for en allowlist. Det er ikke bygget — se de kendte
svagheder nedenfor.

### Netværk

Appen taler med de endpoints der står i `src-tauri/src/providers.rs`, og med de sider du
selv åbner i browser-kort. Der er ingen telemetri og ingen phone-home. Se
[PRIVACY.md](../docs/PRIVACY.md) for hvad der sendes hvorhen.

## Kendte, uadresserede svagheder

Dette er en v0.1 fra ét menneske. Listen her er ikke en undskyldning — den er så du kan
regne dit eget vindue ud. Hvert punkt siger **hvad konsekvensen er**, ikke bare hvad der
mangler.

### Tilgængelighed

- **MCP-transporten håndterer requests sekventielt uden read-timeout.** Én TCP-forbindelse
  der åbnes og så tier, holder det ene request-loop og fryser MCP for **alle** kort på
  ubestemt tid. Det kræver hverken token eller forudgående adgang, fordi identitets-gaten
  ligger *efter* body-læsningen. Der er ingen datavej — det er lammelse, ikke lækage. Den
  realistiske angriber er ikke en webside (Origin-gaten afviser dem før body-læsningen),
  men en lokal proces eller et prompt-injiceret kort.

### Indeslutning

- **Grænsen om appens egen origin er en allowlist, ikke en struktur.** Browser-kort må ikke
  stå på appens origin — det håndhæves nu både ved oprettelse og ved navigation. Men
  capability'en er stadig målrettet vinduet frem for den enkelte webview, og der er intet
  app-ACL-manifest. Et webview-scopet manifest ville gøre grænsen strukturel, så en fremtidig
  kodeændring ikke kan åbne den ved et uheld.
- **Der findes ingen automatiseret fjendtlig-side-test.** Grænsen ovenfor er dækket af
  unit-tests på politik-funktionerne, men ingen test kører en rigtig fjendtlig side mod en
  kørende app. Halvdelen af den test kan ikke være en almindelig `cargo`-test, fordi
  kommandofladen er registreret i binær-craten som `tests/` ikke linker.

### Hemmeligheder på disken

- **Worker-config-mappen arver sine rettigheder fra `%LOCALAPPDATA%`.** Filen
  `projects/<slug>/worker-mcp/<kort>.json` bærer et Bearer-token til appens egen
  loopback-MCP-server. Filen slettes nu når kortet lukkes, og hele mappen ryddes ved
  opstart — men **mens et kort kører**, kan enhver proces under samme Windows-bruger læse
  den, inklusive de andre agent-kort, som pr. definition har shell. Med tokenet kan et kort
  sende `card_say` ind i et andet korts tråde. Der er ingen ACL-stramning på mappen.

### Protokol

- **`initialize` ekkoer klientens protokolversion** i stedet for at svare den vi faktisk
  understøtter, og `MCP-Protocol-Version`-headeren valideres ikke på efterfølgende requests.
  Det er ikke en vej ind — kun appens egne workers når `initialize` — men det betyder at
  serveren lover en version den ikke nødvendigvis taler.

### Frontend og binærer

- **CSP'en kan ikke undvære `style-src 'unsafe-inline'`.** Politikken er sat for den
  bundlede frontend, men xterm 6 injicerer selv runtime-`<style>`-elementer og kalder
  `setAttribute("style", …)` — det ligger i biblioteket, ikke i vores egen CSS, så
  direktivet kan ikke strammes uden at gøre terminalen ulæselig. En injektion i
  hoved-webview'et ville derfor kunne style-bombe fladen. Der er ingen kendt vej ind
  (agent-output skrives som tekstnoder, ikke HTML, og browser-kort er separate webviews),
  så dette er en grænse for dybdeforsvaret, ikke en åben sårbarhed.
- **Release-binærerne indeholder byggemaskinens brugernavn.** `scripts/release-build.mjs`
  fjerner ~94 % (talminal-canvas.exe: 930 → 84 forekomster), men to kilder kan et
  rustc-flag ikke nå: strenge en crate selv har bygget af `env!("CARGO_MANIFEST_DIR")`
  eller skrevet ud fra et build-script, og `aws-lc`'s C-oversættelsesenheder, der bærer
  deres egne `__FILE__`. Det udleverer et brugernavn og et mappe-layout — og det navn står
  i forvejen i `authors`, i repo-URL'en og på npm, så den marginale afsløring er lille.
  **Den fulde løsning er ikke et flag, men en byggesti uden brugernavn** (klon til fx
  `C:\src\talminal` med `CARGO_HOME=C:\cargo`); så bliver tallet nul af sig selv.

Finder du noget der ikke står her, så rapportér det ad kanalen ovenfor.

## Understøttede versioner

Kun `main`. Der er endnu ingen udgivne versioner at bagudpatche.
