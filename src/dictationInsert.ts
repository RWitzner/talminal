// Dikteringens indsaettelses-kontrakt: ét window-event, to modtagere.
//
// Formen er laant fra `PREPARE_FRESH_SPAWN_EVENT` (Card.tsx), og af samme
// grund: begge composere er React-EJEDE. Chat-kortets `<textarea>` er
// kontrolleret (`value={draft}`), saa den kan ikke skrives udefra — React
// ville overskrive vaerdien ved naeste render. Og terminalen bor i en
// xterm-instans der kun findes inde i kortets egen effekt. Et event lader
// hver modtager indsaette PAA SIN EGEN maade, uden at afsenderen behoever en
// ref til nogen af dem.
//
// Modulet er bevidst uden imports, saa hverken Card.tsx eller ChatCard.tsx
// traekker stemme-laget ind for at kende et strengnavn.

export const DICTATION_INSERT_EVENT = "talminal:dictation-insert";

export interface DictationInsertDetail {
  /** Kortets wire-navn (`card-N`) — modtagerne gater paa det. */
  name: string;
  /** Den transskriberede tekst. Altid trimmet og aldrig tom. */
  text: string;
  /**
   * Skal modtageren ogsaa sende?
   *
   * KUN chat-kortet ser nogensinde `true`. Terminal-kort med auto-send gaar
   * slet ikke gennem dette event: dér ejer Rust-sidens `submit_prompt`
   * koreografien (tekst og `\r` som to writes med en verificeret TUI-redraw
   * imellem, `submit.rs`), og den skriver SELV teksten. Et paste efterfulgt af
   * et submit ville lande ordene to gange.
   */
  submit: boolean;
}

export function emitDictationInsert(detail: DictationInsertDetail): void {
  window.dispatchEvent(
    new CustomEvent<DictationInsertDetail>(DICTATION_INSERT_EVENT, { detail }),
  );
}
