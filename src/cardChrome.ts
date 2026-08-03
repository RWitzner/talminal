import type { CSSProperties } from "react";

/**
 * Kort-chromet — skallen, headerbjaelken og nummerbadgen — som de tre
 * korttyper deler dem.
 *
 * De tre komponenter (`Card`, `BrowserCard`, `ChatCard`) havde hver sin kopi
 * af de samme styles, og "de skal ligne hinanden" stod som en KOMMENTAR i
 * Card.tsx frem for i koden. Kopierne var allerede drevet fra hinanden — kun
 * to af tre badges havde `flex: "0 0 auto"` — saa tre kort side om side paa
 * ét canvas kroeb ikke ens.
 *
 * Her staar KUN det der er faelles. Hver komponent spreder disse og laegger
 * sine egne tilfoejelser ovenpaa, saa en bevidst forskel er synlig som en
 * forskel i stedet for at gemme sig i en tredje kopi.
 */

/** Skallen: form, baggrund og den indvendige lyskant. Hoejde/farve/fontsize
 *  er IKKE med — de er reelt forskellige pr. korttype. */
export const CARD_SHELL: CSSProperties = {
  border: "none",
  borderRadius: 13,
  background:
    "linear-gradient(145deg, rgba(13, 22, 34, 0.98), rgba(2, 7, 14, 0.99))",
  display: "flex",
  flexDirection: "column",
  overflow: "hidden",
  boxShadow: "inset 0 1px 0 rgba(229, 244, 255, 0.1)",
};

/** Headerbjaelken. `fontSize` saettes af kortet selv: terminal- og
 *  browser-kortet saetter den her, chat-kortet paa sin rod. */
export const CARD_HEADER: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 7,
  minHeight: 35,
  padding: "0 8px 0 9px",
  borderBottom: "1px solid rgba(207, 232, 255, 0.08)",
  background:
    "linear-gradient(180deg, rgba(31, 42, 56, 0.98), rgba(12, 18, 27, 0.98))",
};

/** Nummerbadgen. Tallet er kortets identitet paa canvas og i stemmen ("luk
 *  kort tre"), saa den skal se ens ud uanset korttype. */
export const CARD_NUMBER_BADGE: CSSProperties = {
  minWidth: 19,
  boxSizing: "border-box",
  padding: "1px 5px",
  border: "1px solid rgba(187, 211, 233, 0.14)",
  borderRadius: 6,
  background: "rgba(112, 137, 163, 0.15)",
  color: "#9eafc1",
  textAlign: "center",
  fontSize: 10,
  fontWeight: 700,
  fontVariantNumeric: "tabular-nums",
};
