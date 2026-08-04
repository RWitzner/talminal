# Medieaktiver og deres rettighedsgrundlag

Filen ligger i roden ved siden af [LICENSE](LICENSE) og [NOTICE](NOTICE), fordi det er
dér en licenslæser kigger.

**Kildekoden er under Apache-2.0. Lydklippene er det ikke.** Læs afsnittet om ElevenLabs
nedenfor, før du redistribuerer dem.

## Lydklip (ElevenLabs)

### Rettighedsgrundlag

De 15 lydfiler nedenfor er genereret med ElevenLabs' tekst-til-tale og er **købt med
kommerciel brugsret** på et betalt abonnement (ejer-bekræftet 2026-07-24). De medfølger
i repoet, så produktet virker ud af boksen.

**Klippene sublicenseres IKKE under Apache-2.0.** Apache-licensen dækker kildekoden.
Lydfilerne er dækket af ElevenLabs' egne vilkår for genereret tale på det abonnement de
blev lavet på.

### Hvad du må — den korte version

Uden en udtalt tilladelse ville en fork stå helt uden grundlag for at have filerne med.
Derfor, eksplicit:

> **Du må redistribuere klippene uændret som en del af Talminal** — i en fork, i en
> release, i npm-tarballen, i en byggeartefakt. Det gælder også ændrede versioner af
> Talminal, så længe klippene selv er uændrede.
>
> **Du må ikke bruge dem til noget andet.** Ikke i et andet produkt, ikke som lydbibliotek,
> ikke som træningsdata, ikke løsrevet fra Talminal. Til dét skal du forholde dig til
> ElevenLabs' vilkår — ikke til denne fil og ikke til Apache-2.0.

Er du i tvivl, eller vil du gøre noget der ikke klart er dækket ovenfor: **regenerér dem
selv.** `voice-eval/gen-reply-clips-elevenlabs.mjs` er generatoren; sæt
`ELEVENLABS_API_KEY` og kør den. Så er klippene dine, under din egen konto, og hele
spørgsmålet bortfalder. Scriptet er en service, ikke en nødløsning.

### Generationsparametre

Fælles for alle: model `eleven_turbo_v2_5`, stemme **Jane**
(`RILOU7YmBhvwJGDGjNmP`, ejer-audition 2026-07-20), `language_code: da`,
`output_format: pcm_24000`.

`language_code` er ikke valgfri: uden den får korte danske sætninger engelsk accent, fordi
modellen har for lidt kontekst til sprogdetektion. `eleven_v3` blev fravalgt — den vakler
på korte sætninger, den er bygget til 250+ tegn.

### Svar-klip — `public/reply-clips/`

14 filer, WAV 24 kHz mono 16-bit PCM. Én fil pr. sætning. Filnavnene er 1:1 med
`ClipKey` i `src/voice/replies.ts`, og vitest håndhæver parret.

| Fil | Talt tekst | Bytes | SHA-256 (16) |
|---|---|---|---|
| `alle-lukket.wav` | Alle lukket | 55772 | `e35f59551d352623` |
| `browser-aabnet.wav` | Browser åbnet | 58002 | `15239112be33bb4a` |
| `det-fangede-jeg-ikke.wav` | Det fangede jeg ikke | 66918 | `b3939679ef325a06` |
| `det-kort-er-en-browser.wav` | Det kort er en browser | 71376 | `dc98b9678465e996` |
| `ingen-kort-aabne.wav` | Ingen kort åbne | 62460 | `b6cf6737cdcbb87e` |
| `ingen-lyd-fanget-proev-igen.wav` | Ingen lyd fanget — prøv igen | 89208 | `dcad5425e27f7cd7` |
| `kort-aabnet.wav` | Kort åbnet | 53542 | `d3352bcde7e61242` |
| `kort-genstartet.wav` | Kort genstartet | 69146 | `4baa7d1366a5d336` |
| `kort-lukket.wav` | Kort lukket | 46856 | `552bafdc4a0a038e` |
| `kortet-findes-ikke.wav` | Kortet findes ikke | 60230 | `ee014ab0c97bf81e` |
| `noget-gik-galt-se-skaermen.wav` | Noget gik galt — se skærmen | 91438 | `65eb1dc04d048b12` |
| `sendt.wav` | Sendt | 40168 | `ef3d2abdd8825673` |
| `sig-det-med-et-kortnummer.wav` | Sig det med et kortnummer | 75834 | `cb871077b5d10ed5` |
| `udfoert.wav` | Udført | 46856 | `e45c9250ba1daf7b` |

### Stemme-probe — `src/assets/stt-probe-da.pcm`

| Fil | Talt tekst | Bytes | SHA-256 (16) | Genereret |
|---|---|---|---|---|
| `stt-probe-da.pcm` | Luk kort to og tre. | 73560 | `e605ba1c0dea8dc0` | 2026-07-29 |

Rå 24 kHz mono PCM16, brugt til at prøve STT-vejen uden en mikrofon. Samme
rettighedsgrundlag som svar-klippene. Genereret via
`voice-eval/gen-reply-clips-elevenlabs.mjs --stt-probe`.

## Baggrunde — `src/assets/*.webp`

Seks baggrunde, alle genereret med **gpt-image 2.0** (OpenAI) og leveret som WebP ved
kvalitet 82.

De originale filer bar C2PA-proveniensmetadata. **WebP-konverteringen fjerner den**, så
proveniensen er ført videre i tekst i stedet: `src/assets/WALLPAPERS.md` indeholder
genereringstidspunkt og den **ordret gengivne prompt** for hver enkelt baggrund.

| Slug | Genereret |
|---|---|
| `blue-folds` | 2026-07-19T19:54:23Z |
| `ember-dunes` | 2026-07-24T13:44:06Z |
| `violet-tide` | 2026-07-24T13:44:35Z |
| `jade-ripples` | 2026-07-24T13:45:04Z |
| `aurora-mist` | 2026-07-24T13:46:00Z |
| `golden-strata` | 2026-07-24T13:46:45Z |

For `blue-folds` blev den oprindelige prompt ikke bevaret i historikken;
`WALLPAPERS.md` siger det ligeud i stedet for at rekonstruere en. `liquid-only` er ikke
et billede, men en syntetisk baggrund tegnet af CSS-gradienter.

Prompterne i `WALLPAPERS.md` er gengivet ordret fra genereringen, **alene med
produktnavnet opdateret** ved omdøbningen til Talminal (august 2026). Optegnelsen siger
det om sig selv, i stedet for at foregive at være urørt.

Baggrundene indgår i produktet under samme vilkår som resten af repoet.

## Terminalfonten — `src/assets/JetBrainsMonoNL-*.woff2`

Terminalen sætter sin tekst i **JetBrains Mono**, ikke i en Windows-font. Den er bundlet,
fordi Windows 11 reelt kun garanterer Cascadia Mono og Consolas, og begge er smallere og
lavere end terminalen har brug for. Uden filerne her ville alle andre end den der
tilfældigvis har fonten installeret falde tilbage til præcis det udgangspunkt bundlingen
er til for at løse.

### Rettighedsgrundlag

JetBrains Mono er udgivet under **SIL Open Font License 1.1**. Licensteksten følger med i
`src/assets/JetBrainsMono-OFL.txt`, sådan som OFL'en kræver. OFL tillader bundling og
redistribution som del af et produkt — også kommercielt — så længe licensen følger med og
fonten ikke sælges løsrevet for sig selv.

**Reserved Font Name:** OFL'en reserverer navnet "JetBrains Mono". Der er ikke ændret
noget i selve fonten, så reservationen er ikke i vejen her. Ændrer du derimod outlines
eller tabeller, skal den afledte font hedde noget andet.

### Filerne

Konverteret fra den officielle **v2.304**-release
(<https://github.com/JetBrains/JetBrainsMono/releases/tag/v2.304>). Releasen leverer
`NL`-varianten som TTF men ikke som woff2, så filerne her er komprimeret med fontTools
4.60.0 (`TTFont.flavor = "woff2"`). Det er en ren formatkonvertering — ingen glyffer,
tabeller eller metrikker er rørt. Efterprøvet på de konverterede filer: 0,600 em
cellebredde, 0,550 em x-højde, ingen `liga`/`calt`-features, fuld dækning af boks- og
bloktegn.

| Fil | Vægt | Bytes | SHA-256 (16) |
|---|---|---|---|
| `JetBrainsMonoNL-Regular.woff2` | 400 | 71948 | `bb56c897ef518add` |
| `JetBrainsMonoNL-Bold.woff2` | 700 | 73288 | `d47d58370f22efdf` |
| `JetBrainsMonoNL-Italic.woff2` | 400 kursiv | 75192 | `046c1afdfe18575f` |

**Hvorfor `NL`-varianten** — JetBrains' egen "no ligatures"-udgave: den almindelige
JetBrains Mono har ligaturer slået til, og xterms DomRenderer samler ens-stylede tegn i
ét span uden at undertrykke dem. En ligatur kunne altså smelte to celler til én glyf og
skubbe resten af linjen ud af gitteret. NL-varianten har ingen liga/calt-features
overhovedet, så problemet findes ikke frem for at blive holdt nede af en CSS-regel nogen
kan komme til at fjerne. Den fulde begrundelse står i `src/terminalFont.ts`.

Fed og kursiv følger med fordi agenterne bruger begge dele. Uden dem ville browseren
syntetisere dem, og syntetisk fed flyder ud over cellekanten i et monospace-gitter.

## Ikoner — `src-tauri/icons/`

App-ikonerne er lavet til projektet og er dækket af [LICENSE](LICENSE) som resten af
repoet.

## README-hero — `assets/hero.png`

1672×941 PNG. Sidder øverst i [README.md](README.md).

- Motor: **gpt-image 2.0** (OpenAI Media Service API)
- Genereret: 2026-08-03
- C2PA: **bevaret i filen** (31 forekomster). Billedet er kopieret råt ind i repoet —
  ingen beskæring, ingen resize, ingen re-encode. Derfor validerer manifesten stadig, og
  proveniensen kan læses af filen selv frem for kun af denne optegnelse.
- Rettighedsgrundlag: genereret til projektet, dækket af [LICENSE](LICENSE) som resten af
  repoet.

**Billedet er en genereret gengivelse af brugerfladen — ikke et skærmbillede.** Modellen
har tegnet UI'et efter et rigtigt skærmbillede. Det ligner produktet, men hvert pixel er
syntetisk, og detaljer kan afvige fra den kørende app. Står det uden forbehold øverst i
READMEen, læses det som dokumentation af hvordan appen ser ud.

Motivet viser fire kort: to Claude-kort, et Codex-kort og et browser-kort.

**Delvist saniteret — ejer-godkendt 2026-08-03.** Launch-spec'ens §4 beder om
demomateriale uden rigtige stier, navne og projektdata. Ejerens mailadresse er redigeret
ud, og resten er en bevidst beslutning frem for en forglemmelse. Optegnet her, så den
næste der kigger ved at det er vejet og ikke overset:

- Ejerens hjemmemappe-sti (`C:\Users\x\talminal` — her med repoets neutrale pladsholder)
  i titelbjælken, og fornavn i begge Claude-kort. Navnet står i forvejen i
  [NOTICE](NOTICE) og `package.json` — Apache-2.0 kræver tilskrivning.
- Abonnementsniveau (`Claude Max`), kontotilstand (*"3 usage limit resets available"*) og
  en konkret `PR #12`. Alt sammen uden betydning for en udenforstående.
- En advarsel om en MCP-server der mangler autentificering, og `permissions: YOLO mode` i
  Codex-kortet. **Det sidste er værd at kende:** billedet viser en agent der kører med
  tilladelser slået fra. Det er brugerens egen codex-opsætning — Talminal sender ingen
  tilladelses-flag, og [SECURITY.md](.github/SECURITY.md) siger det ligeud — men det er altså ikke
  en anbefaling, det er et øjebliksbillede af én maskine.

Skiftes billedet ud, opdateres denne liste i samme pull request.

## Demo-optagelsen — `assets/demo.gif`

880×548 GIF, 72 frames ved 12 fps, seks sekunder, 1,7 MB.
SHA-256 (16) `603d41c60864b9cc`. Sidder øverst i [README.md](README.md)s afsnit
*"Sådan virker det"*.

**Det her er en ægte optagelse af den kørende app** — modsat heroen. Ejerens egen
skærmoptagelse fra 2026-07-22 (2084×1344, to minutter, med lyd), hvoraf udsnittet
62,4–68,4 s er brugt. Lyden er kasseret; en GIF kan ikke bære den.

Motivet: tomt canvas, to kort tegnes ind med cast-animationen, Claude Code booter i begge,
prompten lander, og begge agenter går i gang.

**Beskåret — og det var ikke valgfrit.** De øverste 68 px er skåret af. Titelbjælken stod
med `personlighed  C:\Users\x\personlighed`, altså projektnavn og fuld hjemmemappe-sti.
`repo-scan` er bygget til at fælde netop den streng, men **den kan ikke se ind i en GIF** —
ingen gate ville have fanget det. Skiftes optagelsen ud, skal beskæringen efterprøves
manuelt igen.

**Delvist saniteret — ejer-godkendt 2026-08-03.** Resten kunne ikke beskæres væk uden at
ødelægge optagelsen, og er derfor et bevidst valg. Optegnet her, så den næste der kigger
ved at det er vejet:

- Projektnavnet `personlighed` i kortfanerne og i Claude Codes arbejdsmappe.
- `Persona OS` nævnt i prompten (*"Tilgå dmi.dk via Persona OS-browseren…"*) — et andet af
  ejerens projekter.
- Abonnementsniveau (`Claude Max`) og en advarsel om en MCP-server der mangler
  autentificering. Begge dele står i forvejen i hero-billedet, med samme begrundelse.

Rettighedsgrundlag: optaget af ejeren til projektet, dækket af [LICENSE](LICENSE).

Skiftes optagelsen ud, opdateres denne liste i samme pull request.

## READMEens ikoner — `assets/sections/*.svg`

Ni håndtegnede SVG'er lavet til projektet, dækket af [LICENSE](LICENSE) som resten af
repoet. Ingen generator, intet bibliotek, ingen tredjepartskilde.

`sections/{how,install,req,keys,hud,warn,todo,docs,license}.svg` — ét ikon pr.
H2-overskrift, vist ved 26 px.

**Farverne er ikke valgt frit.** Stregfarven er `#718297` (`Settings.tsx:1198`), og hver
accent er appens egen med sin kodede betydning: `#7ab6e8` starter, `#4dd6b7` kører,
`#e8b046` venter på dig, `#e06058` fejlet (`WorkspaceRail.tsx:358-369`,
`UsageHud.tsx:68-71`). En læser lærer altså appens statussprog af READMEen.

**Én binding der ikke må brydes ved redigering: ingen `<defs>`, gradienter eller
`<pattern>`.** Filerne er flade fyld og streger med vilje. GitHubs SVG-sanitizer kan
fjerne definitioner, og et `fill="url(#…)"` der ikke kan opløses, bliver til en sort flade
i stedet for at fejle synligt.

Ikonerne er tegnet til at overleve 26 px og til at bære på både lyst og mørkt tema, så der
findes **med vilje ingen dark-varianter** — det er derfor stregen er `#718297` og ikke den
lysere `#9eafc1`. Kontrolleret ved at rendere hele sættet i begge temaer ved 26 px før de
blev lagt ind.

## HUD-udsnit — `assets/hud.png`

231×67 PNG, 13687 B, SHA-256 (16) `1a35a2d94942e5ac`. Sidder i [README.md](README.md)s
afsnit om statusline-tap'en.

**Det her er et ægte skærmbillede** — modsat heroen ovenfor. Udsnit af den kørende app,
taget på ejerens maskine 2026-08-03, kopieret råt ind uden beskæring eller re-encode.

Motivet er de to forbrugsbjælker som `statusline-tap/` fylder i HUD'en: 5-timers-vinduet
og uge-vinduet, med procent og resttid. Tallene er ejerens eget Claude Code-forbrug i det
øjeblik billedet blev taget — de siger intet om hvad du selv vil se.

Billedet er saniteret ved at være et udsnit: der er ingen stier, navne, mailadresser eller
projektdata i det. Rettighedsgrundlag: lavet til projektet, dækket af [LICENSE](LICENSE)
som resten af repoet.

## Windows 11-logoet — `assets/windows11.svg`

Sidder i [README.md](README.md)s undertitel efter ordene *"Kun til"*. Det er hele
wordmark'et — de fire firkanter og skrifttrækket *Windows 11* som vektorpaths.

- Kilde: [Wikimedia Commons, `Windows_11_logo.svg`](https://commons.wikimedia.org/wiki/File:Windows_11_logo.svg)
- Hentet: 2026-08-03, 3154 B, uændret
- SHA-256 (16): `e8a33c4612b2ade2`
- Ophavsret: **ingen.** Commons fører filen som *public domain* (`Copyrighted = False`) —
  værket er under tærsklen for værkshøjde.
- Varemærke: **ja.** Commons' egen optegnelse siger `Restrictions = trademarked`.
  Ordmærket og logoet tilhører Microsoft Corporation.

**Derfor står det som det gør.** Logoet bruges nominativt — til at oplyse hvilken platform
Talminal kører på, hvilket er den ene brug et varemærke ikke kan forhindre. Af samme grund
er filen kopieret råt ind: farven er Microsofts egen (`#0078d4`), og den skal blive dér.
Et omfarvet eller ombygget varemærke er en dårligere idé end at lade være.

**Talminal er ikke tilknyttet, godkendt af eller sponsoreret af Microsoft.** Logoet siger
hvor appen kører — ikke hvem der står bag den.

## Hvis du tilføjer et aktiv

Skriv det ind her med kilde, tidspunkt og rettighedsgrundlag i samme pull request. Et
medieaktiv uden proveniens er en licens-gæld, og den skal ikke opdages af den næste der
kigger.
