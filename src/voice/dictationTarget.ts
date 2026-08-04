// Hvor lander en diktering? Ren funktion, ingen DOM og ingen Tauri — den
// tager de to fokus-kilder som data, saa hele beslutningstabellen kan testes
// uden at rejse et canvas.
//
// | # | Betingelse                                  | Maal      |
// | 1 | en chat-composer har DOM-fokus              | chat      |
// | 2 | et fokuseret terminal-kort koerer           | terminal  |
// | 3 | ellers                                      | none      |
//
// RAEKKE 1 STAAR FOERST, og det er ikke en smagssag. `getFocusedCard()` kan
// nemlig ALDRIG returnere et chat-kort: `CanvasSurface.tsx` haenger kun
// `onPointerDown={onBodyPointerDown}` paa terminal-grenen af renderen, og
// `focusCard`s chat-gren fokuserer inputtet og returnerer FOER `enterTypeMode`.
// Klikker man ind i et chat-kort, staar `focused` altsaa uaendret — enten null
// eller nummeret paa det terminalkort man sidst roerte. Uden raekke 1 ville en
// diktering i et chat-kort lande i et HELT andet kort.
//
// At rette det i fokus-modellen i stedet var fravalgt: `focused !== null` ER
// definitionen af type-mode (`CanvasSurface.tsx`' `mode`), saa at saette den
// for chat-kort ville aendre tast-arbitrationen i keyRouting.ts — en
// adfaerdsaendring der ikke hoerer til dikteringen.
//
// Til gengaeld er DOM-fokus den mest DIREKTE sandhed om "hvor er markoeren",
// og det er praecis det spoergsmaal en diktering stiller.

import { isBrowserCard, isChatCard, type CardInfo } from "../types";

export type DictationTarget =
  | { kind: "terminal"; name: string }
  | { kind: "chat"; name: string }
  | { kind: "none"; reason: DictationBlockedReason };

export type DictationBlockedReason =
  | "no_focus"
  | "not_running"
  | "unsupported_card";

export interface DictationFocus {
  /**
   * Traad-id'et paa det chat-kort hvis composer har DOM-fokus, ellers null.
   *
   * TRAAD og ikke kortnavn, fordi det er hvad DOM'en faktisk baerer:
   * `ChatCard.tsx` maerker sin rod med `data-chat-card={card.thread_id}`, og
   * kortnavnet staar ingen steder i chat-kortets traeer. At oversaette her —
   * hvor kortlisten alligevel er i haanden — er billigere end at tilfoeje en
   * attribut hvis eneste formaal var at spare dette opslag.
   */
  chatInputThread: string | null;
  /** Type-mode-kortets nummer (CanvasController.getFocusedCard()). */
  focusedCard: number | null;
  cards: CardInfo[];
}

/** Brugervendt forklaring — HUD'en viser den ordret, saa en tavs diktering
 *  aldrig ligner en fejl i mikrofonen. */
export function describeBlockedTarget(reason: DictationBlockedReason): string {
  switch (reason) {
    case "no_focus":
      return "Klik ind i et kort først — dikteringen skriver i det kort du står i.";
    case "not_running":
      return "Kortet kører ikke — der er ingen terminal at skrive i.";
    case "unsupported_card":
      return "Browser-kort har intet tekstfelt at diktere i.";
  }
}

export function resolveDictationTarget(focus: DictationFocus): DictationTarget {
  const { chatInputThread, focusedCard, cards } = focus;

  if (chatInputThread !== null) {
    const chat = cards.find(
      (card) => isChatCard(card) && card.thread_id === chatInputThread,
    );
    // Kortet kan vaere lukket i samme oejeblik: falder vi igennem her, gaar vi
    // videre til type-mode-grenen frem for at melde en fejl paa et kort der
    // ikke laengere findes.
    if (chat) return { kind: "chat", name: chat.name };
  }

  if (focusedCard === null) return { kind: "none", reason: "no_focus" };

  const card = cards.find((entry) => entry.number === focusedCard);
  if (!card) return { kind: "none", reason: "no_focus" };
  if (isBrowserCard(card)) return { kind: "none", reason: "unsupported_card" };
  // Et chat-kort KAN ikke naa hertil i dag (se hoved-kommentaren), men grenen
  // staar for det tilfaelde at fokus-modellen en dag omfatter dem.
  if (isChatCard(card)) return { kind: "chat", name: card.name };
  if (!card.running) return { kind: "none", reason: "not_running" };
  return { kind: "terminal", name: card.name };
}
