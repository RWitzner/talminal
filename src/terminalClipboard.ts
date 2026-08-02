// Terminalens udklipsholder-lag (spec 2026-07-28 § 2.1-2.3). Samme form som
// keyRouting.ts: en NORMATIV tabel som ren funktion, plus en tynd binding der
// kun oversaetter DOM-eventet og udfoerer beslutningen.
//
// | # | Betingelse            | Resultat |
// | 1 | type !== "keydown"    | pty      |
// | 2 | precededByDeadKey     | pty      |
// | 3 | isComposing           | pty      |
// | 4 | altKey || altGraph    | pty      |
// | 5 | metaKey               | pty      |
// | 6 | !ctrlKey              | pty      |
// | 7 | key c/C               | copy     |
// | 8 | key v/V               | paste    |
// | 9 | ellers                | pty      |
//
// RAEKKE 4 er hele "europaeisk keyboard"-pointen. Windows implementerer AltGr
// som hoejre-Alt = Ctrl+Alt, saa AltGr+2 paa dansk layout ankommer som
// { ctrlKey: true, altKey: true, key: "@" }. Uden raekken aad reglen det
// danske @ — og med den ogsaa £ $ { [ ] } \ | ~ €. Faelden opdages ALDRIG paa
// et US-layout: der findes ingen AltGr, og testen er groen.
//
// RAEKKE 2 er doedtast-vaernet. xterm saetter sit private _unprocessedDeadKey
// naar key er "Dead" eller "AltGraph", og rydder det foerst i en gren vi
// springer over naar vi returnerer false. Staar flaget armet, aedes den
// NAESTE tast — og er den Enter, bliver den ogsaa preventDefault'et, saa den
// naar hverken PTY'en eller browseren.
//
// SPEJLET SKAL SPRINGE MODIFIERS OVER, og det er ikke en detalje. Mellem
// doedtasten og Ctrl+C ligger der ALTID et bart Control-keydown — man kan
// ikke trykke Ctrl+C uden foerst at trykke Ctrl. For det keydown saetter
// xterms Il() ingen r.key, saa _keyDown returnerer true FOER flag-grenen og
// rydder ikke sit flag. Nulstiller vi spejlet dér, divergerer de to, og
// resultatet er praecis det tab vaernet skulle forhindre. Maalt mod den
// installerede xterm (Dead -> Control -> Ctrl+C -> Enter):
//
//   uden handler (i dag)        pty modtager  ["\r"]
//   spejl uden modifier-undtag. pty modtager  []        <- Enter TABT
//   spejl med modifier-undtag.  pty modtager  ["\r"]
//
// Kommer der et TEGN imellem, rydder xterms eget _keyPress/_inputEvent
// flaget, og spejlet er irrelevant — derfor rydder vi netop paa tegn-taster.
//
// shiftKey indgaar bevidst IKKE: Ctrl+Shift+C/V bliver aliaser uden en eneste
// ekstra betingelse.

export type ClipboardAction = "copy" | "paste" | "pty";

export interface ClipboardKeyLike {
  type: string;
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  altGraph: boolean;
  isComposing: boolean;
  precededByDeadKey: boolean;
}

export function routeClipboardKey(ev: ClipboardKeyLike): ClipboardAction {
  if (ev.type !== "keydown") return "pty";
  if (ev.precededByDeadKey) return "pty";
  if (ev.isComposing) return "pty";
  if (ev.altKey || ev.altGraph) return "pty";
  if (ev.metaKey) return "pty";
  if (!ev.ctrlKey) return "pty";
  const key = ev.key.toLowerCase();
  if (key === "c") return "copy";
  if (key === "v") return "paste";
  return "pty";
}

/** DOM -> ren form. `precededByDeadKey` kommer udefra, fordi det er
 *  sekvens-viden bindingen ejer — eventet baerer den ikke selv.
 *  `keyCode === 229` er IME-processing i browsere der ikke saetter
 *  `isComposing`, og `getModifierState` findes ikke paa syntetiske events. */
export function toClipboardKey(
  ev: KeyboardEvent,
  precededByDeadKey: boolean,
): ClipboardKeyLike {
  return {
    type: ev.type,
    key: ev.key,
    ctrlKey: ev.ctrlKey,
    altKey: ev.altKey,
    metaKey: ev.metaKey,
    altGraph: ev.getModifierState?.("AltGraph") ?? false,
    isComposing: ev.isComposing || ev.keyCode === 229,
    precededByDeadKey,
  };
}

/** Praecis den del af xterms Terminal bindingen roerer. Struktureltyped, saa
 *  hverken modulet eller dets test importerer xterm. */
export interface TerminalClipboardHost {
  getSelection(): string;
  clearSelection(): void;
  attachCustomKeyEventHandler(handler: (ev: KeyboardEvent) => boolean): void;
}

/** Taster der ikke maa nulstille doedtast-spejlet, fordi xterm heller ikke
 *  rydder sit eget flag paa dem. Control er den kritiske: den ligger altid
 *  mellem doedtasten og Ctrl+C. */
const MIRROR_TRANSPARENT_KEYS = new Set([
  "Control",
  "Shift",
  "Alt",
  "Meta",
  "CapsLock",
]);

/** `false` betyder "xterm sender intet, og browseren maa selv om det" — xterm
 *  kalder IKKE preventDefault paa vores vegne. Det er hele indsaet-vejen:
 *  WebView2's egen kommando fyrer xterms paste-handler, som bevarer bracketed
 *  paste, saa en flerlinjet blok lander som ÉT stykke i agentens composer.
 *  Kopiér-vejen preventDefault'er derimod selv: clearSelection() koerer
 *  synkront, saa et efterfoelgende copy-event ville ramme en terminal uden
 *  markering. */
export function attachTerminalClipboard(
  term: TerminalClipboardHost,
  writeText: (text: string) => Promise<boolean>,
): void {
  let precededByDeadKey = false;

  term.attachCustomKeyEventHandler((ev) => {
    const action = routeClipboardKey(toClipboardKey(ev, precededByDeadKey));
    if (ev.type === "keydown" && !MIRROR_TRANSPARENT_KEYS.has(ev.key)) {
      precededByDeadKey = ev.key === "Dead" || ev.key === "AltGraph";
    }

    if (action === "pty") return true;
    if (action === "paste") return false;

    ev.preventDefault();
    const selection = term.getSelection();
    if (selection !== "") {
      term.clearSelection();
      void writeText(selection).then((ok) => {
        if (!ok) {
          console.error(
            "[canvas] kopiering til udklipsholderen fejlede — se clipboard.ts",
          );
        }
      });
    }
    return false;
  });
}
