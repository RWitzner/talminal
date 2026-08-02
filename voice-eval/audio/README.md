# Audio-indtalinger til voice-eval

Mikrofon-optagelser af `u01`–`u30` i [`../utterances.md`](../utterances.md) — det er dem `run.mjs`
kræver ved `--audio`; de øvrige rækker (`u31`–`u48`) har ingen indspilning og køres på
ground-truth-teksten. Filerne ligger KUN lokalt (`.gitignore` i denne mappe) — de er test-input til
STT-regressionen, ikke kildekode.

**Navnekonvention (bindende):** `u01.<ext>` … `u30.<ext>` — ID'et SKAL matche utterance-ID'et i
`utterances.md`. Tilladte formater: `.wav` (foretrukket, 16 kHz+ mono), `.m4a`, `.mp3`, `.ogg`,
`.flac`, `.webm`. Én utterance pr. fil, indtalt ordret fra tabellen, i normalt taletempo med
den sædvanlige mikrofon (spec §5-kravet: eval med rigtig mikrofon, ikke syntetisk lyd).

**Fyldord og selvafbrud indtales ordret** — fx u29 (`"Øhm… det der med Vercel deploy… eller nej, hm,
vent."`) og u30. De tester at routeren afviser i stedet for at fabrikere en kommando, og den
egenskab forsvinder hvis tøven glattes ud under indtalingen.
