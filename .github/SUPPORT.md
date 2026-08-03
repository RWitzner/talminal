# Hjælp og spørgsmål

Talminal vedligeholdes af én person i fritiden. Der er ingen supportaftale og intet
svartidsløfte — men spørgsmål er velkomne, og de bliver læst.

## Hvor skal du gå hen

| Har du... | Så brug |
|---|---|
| Et spørgsmål om brug | Læs videre her først — ellers [åbn et blankt issue](https://github.com/RWitzner/talminal/issues/new) |
| Fundet en fejl | [Åbn et issue](https://github.com/RWitzner/talminal/issues/new/choose) |
| Et ønske til en feature | Et issue — sig gerne hvad du prøvede at opnå, ikke kun hvad du vil have bygget |
| Fundet et **sikkerhedsproblem** | **Ikke et issue.** Se [SECURITY.md](SECURITY.md) |
| Lyst til at bidrage | [CONTRIBUTING.md](CONTRIBUTING.md) |

Dansk og engelsk er lige velkomne.

## Før du åbner et issue om en fejl

Tre ting gør forskellen mellem et issue der kan lukkes, og et der ligger:

1. **Kør ritualet.** [CONTRIBUTING.md](CONTRIBUTING.md) har kommandoerne. Er noget rødt
   dér, så sig hvad.
2. **Sig hvilken agent.** Claude Code og Codex CLI opfører sig forskelligt, og
   fejlklasserne er ikke de samme. Tag versionen med (`claude --version` /
   `codex --version`).
3. **Windows-version og hvordan du installerede.** Bygget fra kilde eller hentet?

## Hvad der sandsynligvis er årsagen

Nogle få fejl udgør de fleste rapporter:

- **Et kort spawner ikke.** Ligger `claude.exe` eller `codex` på din PATH? Talminal
  slår binæren op på PATH og siger det ligeud i fejlbeskeden, hvis den ikke er der.
- **Stemmen gør ingenting.** Mikrofon-tilladelse i Windows' privatlivsindstillinger er
  den hyppigste årsag, og ingen app-side kan omgå den. Dernæst: mangler der en
  OpenAI-nøgle i Indstillinger?
- **Browser-kort åbner ikke.** De starter `npx @playwright/mcp` ved kørsel — Node er
  altså et runtime-krav, ikke kun et byggeværktøj.
- **Appen starter ikke.** Kører der allerede en Talminal for samme projekt? Der er
  præcis én proces pr. projekt, med vilje.

## Hvad der ikke er understøttet

- **Andet end Windows 11 x64.** ConPTY, Credential Manager og WebView2 er alle i den
  kritiske sti.
- **Andre sprog end dansk i stemmevejen.** STT'en er låst til dansk (`language: "da"`,
  `src/voice/stt.ts:136`).
  Engelsk er ønsket, men ikke bygget.
- **Andre agenter end Claude Code og Codex CLI.** Mekanismen til at tilføje en er
  beskrevet i [ARCHITECTURE.md](../docs/ARCHITECTURE.md), hvis du vil prøve.
