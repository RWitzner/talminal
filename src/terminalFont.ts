// Terminalens font — bundlet, ikke systemets.
//
// HVORFOR BUNDLET: Windows 11 garanterer reelt kun Cascadia Mono og Consolas.
// Maalt med fontTools paa selve fontfilerne er Cascadia 0,586 em bred med
// 0,518 em x-hoejde; Consolas er endnu smallere (0,550 / 0,490). JetBrains
// Mono er 0,600 / 0,550 — bredere celle OG hoejere smaabogstaver, som er
// praecis de to akser "for lille og for sammenpresset" bestaar af. En
// system-font kunne altsaa ikke loese det, og uden en bundlet fil ville alle
// andre end den der tilfaeldigvis har fonten installeret falde tilbage til
// noejagtig det udgangspunkt vi ville vaek fra.
//
// HVORFOR "NL"-VARIANTEN: JetBrains Mono har ligaturer slaaet til som
// standard (=>, !=, ->). xterm v6 tegner med sin DomRenderer, der samler
// ens-stylede tegn i ét span og ikke undertrykker ligaturer — to celler
// kunne altsaa smelte til én glyf og skubbe resten af linjen ud af gitteret.
// JetBrains' NL-variant ("no ligatures") har ingen liga/calt-features
// overhovedet, saa problemet FINDES ikke i stedet for at blive holdt nede af
// en CSS-regel nogen kan komme til at fjerne. Metrikken er identisk med den
// almindelige variant; kun ligatur-glyfferne mangler.
//
// Familienavnet baerer "NL" med vilje: registrerer vi den som "JetBrains
// Mono", kolliderer den med en bruger-installeret JetBrains Mono der HAR
// ligaturer, og saa er det tilfaeldigt hvilken der vinder.
//
// Licens (SIL OFL 1.1) og konverterings-proveniens: ASSETS.md.

interface FontSource {
  url: string;
  weight: string;
  style: string;
}

// Statiske new URL()-kald pr. fil — samme moenster som wallpapers.ts. Vite
// kan kun emitte aktivet naar stien staar som en literal; en template med en
// variabel ville i stedet trigge en glob over hele assets-mappen.
const FONT_SOURCES: FontSource[] = [
  {
    url: new URL("./assets/JetBrainsMonoNL-Regular.woff2", import.meta.url).href,
    weight: "400",
    style: "normal",
  },
  {
    url: new URL("./assets/JetBrainsMonoNL-Bold.woff2", import.meta.url).href,
    weight: "700",
    style: "normal",
  },
  {
    url: new URL("./assets/JetBrainsMonoNL-Italic.woff2", import.meta.url).href,
    weight: "400",
    style: "italic",
  },
];

/** Familienavnet alene — til document.fonts-opslag og FontFace-registrering. */
export const TERMINAL_FONT_FAMILY = "JetBrains Mono NL";

/** Kaeden bagefter er det terminalen SAA ud som foer: kan woff2'en ikke
 *  hentes, bliver kortene smallere, men de virker. */
export const TERMINAL_FONT_STACK = `"${TERMINAL_FONT_FAMILY}", "Cascadia Mono", Consolas, monospace`;

/** 13 var for lavt: Windows Terminal koerer 12 pt ≈ 16 px, og vi laa ~20 %
 *  under. 14 px paa JetBrains Monos hoejere x-hoejde laeser omtrent som 15 px
 *  Cascadia uden at aede for mange kolonner af kortet. */
export const TERMINAL_FONT_SIZE = 14;

/** xterm's default er 1.0 — cellen er da fontens naturlige linjehoejde uden
 *  en eneste pixel luft. Det var "sammenpresset"-halvdelen af problemet. */
export const TERMINAL_LINE_HEIGHT = 1.2;

let loading: Promise<void> | null = null;

/**
 * Registrerer og indlaeser terminalfonten. Idempotent.
 *
 * SKAL vaere afsluttet FOER det foerste kort aabner sin xterm. xterm maaler
 * celle-bredden én gang i term.open() og gen-maaler kun naar fontFamily eller
 * fontSize skifter til en ANDEN vaerdi — OptionsService.ts:134 fyrer ikke paa
 * en identisk tildeling, saa det gaengse "saet fontFamily igen"-trick er et
 * no-op her. Maales cellen paa fallback-fonten, staar kortet med for mange
 * kolonner resten af sessionen.
 *
 * Afviser aldrig. En manglende font er et daarligere udseende, ikke en
 * doed app, og maa ikke kunne blokere opstarten.
 */
export function loadTerminalFont(): Promise<void> {
  if (loading) return loading;
  loading = Promise.all(
    FONT_SOURCES.map(async ({ url, weight, style }) => {
      const face = new FontFace(
        TERMINAL_FONT_FAMILY,
        `url(${url}) format("woff2")`,
        { weight, style },
      );
      document.fonts.add(await face.load());
    }),
  ).then(
    () => undefined,
    (err) => {
      console.error(
        "terminalfonten kunne ikke indlaeses — falder tilbage til Cascadia:",
        err,
      );
    },
  );
  return loading;
}
