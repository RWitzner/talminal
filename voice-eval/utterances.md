# Voice-eval: 48-utterance eval-sæt (schema v4 + kæder + agent)

Eval-sættet er fundamentet for spec §7's fulde regression: **transskription** (STT mod rigtige
indtalinger i `audio/`), **intent** (Realtime-klassifikation) og **mål** (schema v4-target-resolution).
Dansk/engelsk blandet som reel daglig brug — danske
bæresætninger med engelske tech-termer, og hele engelske sætninger imellem.

**Mål:** intent ≥90 %, target ≥95 %; alle 21 tvetydige cases SKAL give HUD-fejl,
IKKE handling. Nævnerne beregnes fra rækkerne i sektionerne nedenfor.

**T6-tillæg (u42-u48):** `new_card` bærer nu et valgfrit `agent`-felt
(`"claude"` | `"codex"`, kun ved eksplicit ord i ytringen — spec N2/agent-adapter).
Absolut-tærskel ved eval-kørsel: de oprindelige 41 cases FASTHOLDT + u42-u48 ≥ 7/7 i
mindst én kørsel og ≥ 6/7 i begge (Gate 1-mønsteret dækker fortsat kun de oprindelige
41 procentvise tærskler — `run.mjs`'s tællere er eksplicit scopet væk fra u42-u48 via
`T6_NEW_IDS`). u42-u48's egen tærskel kan IKKE gates af en enkelt kørsel — den er en
vurdering på tværs af to kørsler — så `run.mjs` printer den som en separat,
ikke-gatende linje ("Nye agent-cases (T6, u42-u48, IKKE-gatende...)") som operatøren
selv sammenholder mellem de to kørsler.

## Frossen eval-kontekst

Kort på canvas (kort 4 er lukket og IKKE genåbnet i den frosne kontekst — allokatoren
genbruger laveste ledige nummer, men ingen oprettelse er sket siden):

| Kort | Projekt/cwd |
|---|---|
| 1 | `C:\projekter\demo` |
| 2 | `C:\projekter\webshop` |
| 3 | `C:\projekter\brain-site` |
| 5 | `C:\projekter\demo\canvas` |

Kontekst-varianter (angivet pr. utterance):

- **A:** kort 2 er fokuseret (præcis ét).
- **B:** INTET kort er fokuseret.

Kortlisten givet til resolveren: `[{number: 1}, {number: 2}, {number: 3}, {number: 5}]`.

## Utterances

Kolonnen **Forventet intent** er schema v4-`VoiceIntent`-kindet + slots.
**Mål-kort** er resultatet EFTER resolveren — eller den forventede fejl-reason ved HUD-fejl.

### Gyldige kommandoer (u01-u06, u10-u13, u22-u25, u31-u32, u34, u42, u45, u47)

| ID | Ktx | Utterance | Sprog | Forventet intent | Mål-kort |
|---|---|---|---|---|---|
| u01 | A | "Kør testene igen og fix det der fejler." | da | `send_prompt { card: null, text: "Kør testene igen og fix det der fejler" }` | 2 (fokus) |
| u02 | A | "Sig til kort tre at den skal skrive en README for projektet." | da | `send_prompt { card: 3, text: ~"skriv en README for projektet" }` | 3 |
| u03 | A | "Ask card one to run npm run build and report any errors." | en | `send_prompt { card: 1, text: ~"run npm run build and report any errors" }` | 1 |
| u04 | B | "Kort fem: commit det du har med beskeden fix pty resize." | da+en | `send_prompt { card: 5, text: ~"commit det du har med beskeden fix pty resize" }` | 5 |
| u05 | A | "Fortsæt hvor du slap og deploy til Vercel bagefter." | da+en | `send_prompt { card: null, text: ~"fortsæt hvor du slap og deploy til Vercel bagefter" }` | 2 (fokus) |
| u06 | A | "Tell card two to add dark mode to the settings panel." | en | `send_prompt { card: 2, text: ~"add dark mode to the settings panel" }` | 2 |
| u10 | A | "Luk kort tre." | da | `close_cards { cards: [3] }` | 3 (udføres straks) |
| u11 | B | "Close card five." | en | `close_cards { cards: [5] }` | 5 (udføres straks) |
| u12 | A | "Genstart kort et." | da | `restart_card { card: 1 }` | 1 (udføres straks) |
| u13 | A | "Restart card three, it seems stuck." | en | `restart_card { card: 3 }` | 3 (udføres straks) |
| u22 | A | "Åbn 4 terminaler." | da | `new_card { count: 4 }` | projekt-rod (`C:\projekter\demo`) |
| u23 | B | "Ny terminal." | da | `new_card { count: 1 }` | projekt-rod (`C:\projekter\demo`) |
| u24 | A | "Åbn tre kort mere." | da | `new_card { count: 3 }` | projekt-rod (`C:\projekter\demo`) |
| u25 | A | "Luk kort 2 og 3." | da | `close_cards { cards: [2, 3] }` | 2 og 3 (udføres straks) |
| u31 | B | "Åbn en browser." | da | `open_browser { url_hint: null }` | intet kortmål — nyt browser-kort, url_hint null |
| u32 | A | "Åbn en browser på GitHub." | da | `open_browser { url_hint: "github" }` | intet kortmål — nyt browser-kort, url_hint github |
| u34 | A | "Spørg kort tre hvor langt den er." | da | `send_prompt { card: 3, text: ~"Hvor langt er du?" }` | 3 |
| u42 | A | "Nyt codex-kort." | da | `new_card { count: 1, agent: "codex" }` | projekt-rod (`C:\projekter\demo`), agent codex |
| u45 | B | "Åbn et kort med gpt." | da+en | `new_card { count: 1 }` (agent null — "gpt" mapper IKKE til et agent-gæt) | projekt-rod (`C:\projekter\demo`) |
| u47 | A | "Åbn tre kort mere." | da | `new_card { count: 3 }` (regressionsvagt: agent forbliver null efter T6-schemaet) | projekt-rod (`C:\projekter\demo`) |

### Kæde-cases (gyldige, u35-u37 + u39 + u41 + u43-u44)

| ID | Ktx | Utterance | Sprog | Forventet kæde | Mål |
|---|---|---|---|---|---|
| u35 | A | "Åbn en terminal og en browser." | da | `[new_card {count:1}, open_browser {url_hint:null}]` | 2 nye kort |
| u36 | A | "Luk kort 2 og genstart kort 3." | da | `[close_cards {cards:[2]}, restart_card {card:3}]` | 2 → luk, 3 → genstart |
| u37 | B | "Åbn tre terminaler og en browser på GitHub. Send en prompt til kort et om status." | da | `[new_card {count:3}, open_browser {url_hint:"github"}, send_prompt {card:1, text:~"Hvad er status?"}]` | 4 nye kort + prompt til 1 |
| u39 | A | "Luk kort 2 og spørg kort 3 hvor langt den er." | da | `[close_cards {cards:[2]}, send_prompt {card:3, text:~"Hvor langt er du?"}]` | 2 → luk, 3 → prompt |
| u41 | B | "Åbn 3 terminaler og 1 browser på GitHub. Send prompt til kort 1 om status, kort 2 start task 11, kort 3 start task 12." | da | `[new_card {count:3}, open_browser {url_hint:"github"}, send_prompt {card:1, text:~"status"}, send_prompt {card:2, text:~"task 11"}, send_prompt {card:3, text:~"task 12"}]` | multi-spec §7.1-morgenkæden, 5 elementer i talt rækkefølge |
| u43 | B | "Åben fire terminaler, to med claude og to med codex." | da | `[new_card {count:2, agent:"claude"}, new_card {count:2, agent:"codex"}]` | 4 nye kort — 2 claude + 2 codex |
| u44 | A | "Åbn to codex-terminaler og send en prompt til kort et om status." | da | `[new_card {count:2, agent:"codex"}, send_prompt {card:1, text:~"Hvad er status?"}]` | 2 nye codex-kort + prompt til 1 |

### Bevidst tvetydige — SKAL give HUD-fejl, ikke handling (u07-u09, u14-u21, u26-u30, u33, u38, u40, u46, u48)

| ID | Ktx | Utterance | Sprog | Forventet resultat | Forventet reason |
|---|---|---|---|---|---|
| u07 | A | "Nyt kort i demo-projektet." | da | HUD-fejl — kommandoen findes ikke i v4 (status/fokus/mappenavn-create); routeren må IKKE mappe til send_prompt eller new_card | router-reject |
| u08 | B | "Open a new card in the webshop project." | en | HUD-fejl — kommandoen findes ikke i v4 (status/fokus/mappenavn-create); routeren må IKKE mappe til send_prompt eller new_card | router-reject |
| u09 | A | "Start et nyt kort i canvas-mappen." | da | HUD-fejl — kommandoen findes ikke i v4 (status/fokus/mappenavn-create); routeren må IKKE mappe til send_prompt eller new_card | router-reject |
| u14 | A | "Gå til kort fem." | da | HUD-fejl — kommandoen findes ikke i v4 (status/fokus/mappenavn-create); routeren må IKKE mappe til send_prompt eller new_card | router-reject |
| u15 | B | "Focus card two." | en | HUD-fejl — kommandoen findes ikke i v4 (status/fokus/mappenavn-create); routeren må IKKE mappe til send_prompt eller new_card | router-reject |
| u16 | A | "Skift til kort tre." | da | HUD-fejl — kommandoen findes ikke i v4 (status/fokus/mappenavn-create); routeren må IKKE mappe til send_prompt eller new_card | router-reject |
| u17 | A | "Gå til kort to." | da | HUD-fejl — kommandoen findes ikke i v4 (status/fokus/mappenavn-create); routeren må IKKE mappe til send_prompt eller new_card | router-reject |
| u18 | B | "Focus card one." | en | HUD-fejl — kommandoen findes ikke i v4 (status/fokus/mappenavn-create); routeren må IKKE mappe til send_prompt eller new_card | router-reject |
| u19 | A | "Hvad laver kort tre lige nu?" | da | HUD-fejl — kommandoen findes ikke i v4 (status/fokus/mappenavn-create); routeren må IKKE mappe til send_prompt eller new_card | router-reject |
| u20 | A | "What's the status on card five?" | en | HUD-fejl — kommandoen findes ikke i v4 (status/fokus/mappenavn-create); routeren må IKKE mappe til send_prompt eller new_card | router-reject |
| u21 | A | "Status på kort et." | da | HUD-fejl — kommandoen findes ikke i v4 (status/fokus/mappenavn-create); routeren må IKKE mappe til send_prompt eller new_card | router-reject |
| u26 | B | "Luk kortet." | da | HUD-fejl — destruktiv intent uden kortnummer og uden fokus; routeren må IKKE gætte et mål | `no_target` |
| u27 | A | "Luk kort fire." | da | HUD-fejl — kort 4 findes ikke (lukket; intet kort 4 i den frosne kontekst); ingen handling mod andet kort | `no_such_card` |
| u28 | B | "Fix lige den bug vi snakkede om." | da | HUD-fejl — `send_prompt { card: null }` uden fokuseret kort | `ambiguous_focus` |
| u29 | A | "Øhm… det der med Vercel deploy… eller nej, hm, vent." | da+en | HUD-fejl — ingen klassificérbar kommando (fyldord + selvafbrudt); routeren må IKKE fabrikere en `send_prompt` | router-reject |
| u30 | A | "Genstart… nej, luk… altså kort to eller tre." | da | HUD-fejl — selvmodsigende handling OG tvetydigt mål; hverken genstart eller luk må udføres | router-reject |
| u33 | A | "Åbn en browser… nej vent, luk kort to." | da | HUD-fejl — selvkorrigeret handlingskæde (åbn browser → luk kort); routeren må IKKE fabrikere hverken `open_browser` eller `close_cards` | router-reject |
| u38 | A | "Luk kort 2 og hvad laver kort 3?" | da | HUD-fejl — status-segmentet forgifter kæden (alt-eller-intet) | router-reject |
| u40 | A | "Genstart kort 2… nej, luk kort 3." | da | HUD-fejl — korrektions-markør trumfer kædning; må IKKE blive en 2-kæde | router-reject |
| u46 | B | "Genstart codex-kortet." | da | HUD-fejl — beskrivelse ("codex-kortet") er ikke et kortnummer; routeren skal returnere `card: null` uden at gætte, og resolveren spørger (restart_card falder ALDRIG tilbage på fokus, jf. `resolveTarget`) | `no_target` |
| u48 | B | "Luk codex-kortene." | da | HUD-fejl — destruktiv intent uden kortnumre; routeren må IKKE opfinde numre fra agent-ordet — resolveren spørger (samme mønster som `u26`) | `no_target` |

## Noter til eval-kørsel

- **Transskription:** kør `audio/u01.*`–`audio/u30.*` gennem den valgte STT (verdict 4) og journalfør
  word-accuracy pr. utterance. Tech-termer (`npm run build`, `README`, `Vercel`,
  `pty resize`, `commit`) og danske talord ("tre", "fem") er de forventede fejlkilder — de er
  bevidst med, fordi kortnumre og kommandoord er det, routeren faktisk afhænger af.
- **Intent+mål:** kør alle 48 rækker gennem router-LLM'en med den frosne kontekst ovenfor — de 30
  transskriptioner plus ground-truth-teksten for `u31`–`u48` (ved isoleret router-eval bruges
  ground-truth-teksten hele vejen); sammenlign med kolonnerne.
- **Hint-løse creates:** `new_card` bar oprindeligt ingen felter udover `count` — `u22`–`u24` sender
  fortsat ingen andre felter. Fra T6 kan `new_card` også bære et valgfrit `agent`
  (`"claude"`/`"codex"`, kun ved eksplicit ord — `u42`-`u44`); et andet model-/agent-ord som "gpt"
  mapper ALDRIG til et agent-gæt (`u45`). En create der udtaler en mappe eller et projekt
  ("Nyt kort i demo-projektet") er i v4 en reject, aldrig et gæt.
- **Kæde-cases:** evaluerer intent-sekvensen (kind + slots i rækkefølge); target-check pr.
  kort-bærende element mod den frosne kortliste.
- **u46 (kontekstvalg):** `restart_card`s target-resolution falder ALDRIG tilbage på
  fokuseret kort (kun `send_prompt` gør det, jf. `resolveTarget` i `src/voice/intents.ts`) —
  `card: null` giver derfor `no_target` uanset kontekst A eller B. u46 kører i kontekst B for
  at gøre pointen utvetydig: testen bekræfter at routeren ikke opfinder et kortnummer fra
  ordet "codex", ikke en fokus-fallback-mekanik der ikke findes for denne kommando.
- `text`-slots markeret `~"…"` kræver kun semantisk ækvivalens (omformulering OK), ikke ordret match.
