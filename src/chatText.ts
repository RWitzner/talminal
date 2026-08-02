// Rene hjaelpere til chat-kortets praesentation. Ingen React, ingen Tauri —
// hele formatterings-logikken ligger her, saa den kan testes uden en DOM.
//
// Den vigtigste regel i hele filen: teksten muteres ALDRIG. Runbookens §2.8
// beviser at et flerlinjet svar med kodeblok krydser traaden uaendret, og
// kortet er det eneste sted ejeren kan aflaese det. Vi opdeler derfor kun
// teksten i segmenter og lader hvert segments krop staa ord for ord.

/** Et stykke beskedtekst: enten prosa eller en indhegnet kodeblok. */
export interface TextSegment {
  kind: "text" | "code";
  /** Ordret indhold. Hegnslinjerne (```) er ikke med — alt andet er. */
  body: string;
  /** Sproget fra aabningshegnet (`\`\`\`rust`), ellers tom streng. */
  lang: string;
}

const FENCE = /^```([^\n]*)$/;

/**
 * Deler en besked op ved kodehegn i linjestart.
 *
 * - Ingen hegn ⇒ ét text-segment med hele teksten.
 * - Et uafsluttet aabningshegn ⇒ resten er kode. Beskeder ankommer hele (der
 *   er ingen streaming i traaden), saa et hegn uden makker er forfatterens
 *   fejl — og det er mere loyalt at vise resten som den kode den skulle have
 *   vaeret end at vise et hegn som prosa.
 * - Segmenter der kun er blanktegn droppes, saa der ikke opstaar tomme huller
 *   omkring en kodeblok. Kode-segmenter beholdes altid: en blank kodeblok er
 *   stadig et resultat.
 */
export function splitFencedSegments(text: string): TextSegment[] {
  const segments: TextSegment[] = [];
  let buffer: string[] = [];
  let inCode = false;
  let lang = "";

  const flush = () => {
    if (buffer.length === 0) return;
    const body = buffer.join("\n");
    buffer = [];
    if (!inCode && body.trim() === "") return;
    segments.push({ kind: inCode ? "code" : "text", body, lang: inCode ? lang : "" });
  };

  for (const line of text.split("\n")) {
    const fence = FENCE.exec(line);
    if (fence === null) {
      buffer.push(line);
      continue;
    }
    flush();
    inCode = !inCode;
    lang = inCode ? fence[1].trim() : "";
  }
  flush();
  return segments;
}

/**
 * Lokal ur-tid `HH:MM`. Traaden har et 5-minutters idle-ur og et absolut ur
 * paa 20 minutter, saa hvornaar en besked landede er reel driftsinformation —
 * ikke pynt. Bygget af Date-felter frem for toLocaleTimeString, saa formatet
 * er det samme uanset vaertens locale.
 */
export function formatClock(tsMs: number): string {
  const at = new Date(tsMs);
  if (Number.isNaN(at.getTime())) return "";
  const hh = String(at.getHours()).padStart(2, "0");
  const mm = String(at.getMinutes()).padStart(2, "0");
  return `${hh}:${mm}`;
}

/**
 * Giver hver agent i traaden en fast plads (0, 1, …) efter hvornaar den
 * foerste gang siger noget. Pladsen vaelger accentfarven.
 *
 * Rangordnet efter foerste optraeden — ikke efter kortnummer — af to grunde:
 * traaden har praecis to medlemmer, saa 0 og 1 er altid nok, og en hash over
 * kortnummeret kunne give BEGGE agenter samme farve, hvilket ville fjerne
 * hele pointen. Farven er i oevrigt kun et supplement: afsenderen staar altid
 * ogsaa som tekst (spec §6 — farve alene er ikke en label).
 */
export function assignAgentSlots(agentCards: readonly string[]): Map<string, number> {
  const slots = new Map<string, number>();
  for (const card of agentCards) {
    if (!slots.has(card)) slots.set(card, slots.size);
  }
  return slots;
}

/** Hvor braendt er traadens hop-budget. Ved loftet lukker traaden. */
export type HopTone = "calm" | "warn" | "spent";

/**
 * Advarslen begynder ved fire tilbage. Det er ikke et rundt tal for
 * rundhedens skyld: en delegering plus dens svar koster to hop, saa fire er
 * det sidste punkt hvor der er plads til baade en opklaring og et svar.
 */
export function hopTone(used: number, cap: number): HopTone {
  if (used >= cap) return "spent";
  return cap - used <= 4 ? "warn" : "calm";
}
