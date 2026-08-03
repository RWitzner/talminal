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
selv.** Så er de dine, under din egen konto, og hele spørgsmålet bortfalder.

**Du kan regenerere dem selv.** `voice-eval/gen-reply-clips-elevenlabs.mjs` er
generatoren; sæt `ELEVENLABS_API_KEY` og kør den. Så er klippene dine, under din egen
konto. Scriptet er en service, ikke en nødløsning.

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

## Ikoner — `src-tauri/icons/`

App-ikonerne er lavet til projektet og er dækket af [LICENSE](LICENSE) som resten af
repoet.

## Hvis du tilføjer et aktiv

Skriv det ind her med kilde, tidspunkt og rettighedsgrundlag i samme pull request. Et
medieaktiv uden proveniens er en licens-gæld, og den skal ikke opdages af den næste der
kigger.
