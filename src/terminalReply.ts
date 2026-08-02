// Terminal-protokol-auto-svar (fix F1, udvidet efter M0b-dogfood-FUND 2,
// 2026-07-17): xterm.js besvarer selv terminal-queries via onData — det er
// PROTOKOL-trafik, ikke menneskeligt input, og maa ALDRIG udloese auto-pause.
// FUND 2: et fantom-pause-signal uden tastetryk (control_owner_change
// 17:42:28.949Z) viste at den oprindelige liste (CPR/DA/fokus/kitty) var
// smallere end xterm.js' faktiske auto-svar-flade. Kataloget nedenfor er den
// FULDE flade (InputHandler-svarene), saa en ukendt query-form aldrig igen
// bliver et fantom — plus diagnostik i Card.tsx for alt der stadig falder
// igennem.
//
//   CPR      \x1b[<r>;<c>R         svar paa DSR 6 (ESC[6n) — Rust-kernen
//                                  besvarer den FOERSTE query selv (spike-
//                                  FUND 1); senere queries naar xterm
//   DECXCPR  \x1b[?<r>;<c>(;<p>)R  svar paa DSR ?6 (evt. med side-nr.)
//   DSR-ok   \x1b[0n               svar paa DSR 5 (status-forespoergsel)
//   DA1      \x1b[?<..>c           primary device attributes
//   DA2      \x1b[><..>c           secondary device attributes
//   Fokus    \x1b[I / \x1b[O       focus in/out (mode 1004 — CC enabler den,
//                                  spike-FUND 6)
//   Kitty    \x1b[?<n>u            kitty-keyboard-flags-svar
//   DECRPM   \x1b[?<m>;<v>$y       svar paa DECRQM-mode-forespoergsel (fx
//                                  synchronized output 2026); ogsaa
//                                  ANSI-formen uden '?'
//   CSI-t    \x1b[<..>t            vindues-/tekstareal-rapporter (svar paa
//                                  14t/16t/18t mv.)
//   OSC      \x1b]..(BEL|ST)       farve-query-svar (OSC 10/11/12 "rgb:..",
//                                  CC's tema-detektion)
//   DCS      \x1bP..ST             DECRQSS-/XTGETTCAP-svar
//
// Bevidst IKKE med: SGR-mus-rapporter (\x1b[<..M/m) og hjul-til-piletaster
// (\x1b[A/B — byte-identisk med menneskelige piletaster). Fysisk mus-/hjul-
// interaktion i kortet er en SEMANTISK afgoerelse (tager mennesket roret?),
// ikke en klassifikationsfejl.
//
// HJUL-EKSPERIMENTET ER AFGJORT (2026-07-28, spec § 0.3). Konverteringen kom
// ikke fra mode 1007, men fra xterms egen regel "ingen scrollback => hjul
// bliver til pil op/ned". Med scrollback: 2000 rammer den kun alt-screen-
// buffere, og de agenter der bruger dem (CC) rapporterer selv mus, saa
// grenen springes over. Codex' normal-buffer ruller nu rigtigt.
//
// Bracketed paste er derimod en KENDT form og hoerer til i kataloget — se
// isBracketedPaste nederst.
//
// Human-siden er sikker: ingen tastetryk-sekvens (piletaster \x1b[A..D,
// modifier-varianter \x1b[1;5C, SS3 \x1bO?, funktionstaster \x1b[<n>~,
// shift+tab \x1b[Z, bracketed paste \x1b[200~..\x1b[201~, bare ESC) matcher
// nogen af formerne — og en chunk der BLANDER svar og tastetryk matcher
// heller ikke (fuld-match-anker), saa den pauser korrekt.
export const TERMINAL_REPLY_RE = new RegExp(
  "^(?:" +
    [
      "\\x1b\\[\\d+;\\d+R", // CPR
      "\\x1b\\[\\?\\d+;\\d+(?:;\\d+)*R", // DECXCPR
      "\\x1b\\[0n", // DSR-ok
      "\\x1b\\[\\?\\d+(?:;\\d+)*c", // DA1
      "\\x1b\\[>\\d+(?:;\\d+)*c", // DA2
      "\\x1b\\[I", // fokus ind
      "\\x1b\\[O", // fokus ud
      "\\x1b\\[\\?\\d+u", // kitty-flags
      "\\x1b\\[\\??\\d+;\\d+\\$y", // DECRPM (privat + ANSI-form)
      "\\x1b\\[\\d+(?:;\\d+)*t", // CSI-t-rapporter
      "\\x1b\\][^\\x07\\x1b]*(?:\\x07|\\x1b\\\\)", // OSC-svar (BEL/ST)
      "\\x1bP[^\\x1b]*\\x1b\\\\", // DCS-svar (ST-termineret)
    ].join("|") +
    ")+$",
);

/** true = xterm.js-protokol-auto-svar (sendes med source:"terminal" — ingen
 *  pause); false = menneskeligt input (auto-pause ved foerste tast). */
export function isTerminalReply(data: string): boolean {
  return TERMINAL_REPLY_RE.test(data);
}

/** Hex-dump til FUND 2-diagnostikken i Card.tsx: en ESC-indledt chunk der
 *  klassificeres human, logges som hex saa en evt. NY auto-svar-form kan
 *  identificeres praecist i devtools i stedet for at vaere et tavst fantom. */
export function hexDump(data: string): string {
  return Array.from(data, (c) =>
    c.codePointAt(0)!.toString(16).padStart(2, "0"),
  ).join(" ");
}

/** Bracketed paste (DECSET 2004) indledes med ESC [ 200 ~. Det er en KENDT
 *  ESC-form, og den maa derfor ikke ende i Card.tsx' FUND 2-diagnostik: den
 *  hex-dumper hver ESC-indledt chunk der ikke er et auto-svar, og en indsat
 *  blok ville lande i devtools i fuld laengde — baade som stoej og som en
 *  kopi af alt hvad ejeren indsaetter. Klassifikationen er uaendret: indsat
 *  tekst er og bliver source:"human". */
export function isBracketedPaste(data: string): boolean {
  return data.startsWith("\x1b[200~");
}
