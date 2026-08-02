import { describe, expect, it } from "vitest";
import { isBracketedPaste, isTerminalReply } from "./terminalReply";

// Terminal-siden: xterm.js' fulde auto-svar-katalog (fix F1 + M0b-FUND 2) —
// alle skal klassificeres som protokol-trafik (ingen auto-pause).
const TERMINAL: [string, string][] = [
  ["CPR", "\x1b[24;80R"],
  ["DECXCPR med side-nr", "\x1b[?24;80;1R"],
  ["DECXCPR uden side-nr", "\x1b[?24;80R"],
  ["DSR-ok", "\x1b[0n"],
  ["DA1", "\x1b[?6c"],
  ["DA1 med params", "\x1b[?1;2c"],
  ["DA2", "\x1b[>0;276;0c"],
  ["fokus ind", "\x1b[I"],
  ["fokus ud", "\x1b[O"],
  ["kitty-flags", "\x1b[?1u"],
  ["DECRPM privat mode (sync output 2026)", "\x1b[?2026;2$y"],
  ["DECRPM ANSI-form", "\x1b[4;1$y"],
  ["CSI-t tegnrapport", "\x1b[8;30;120t"],
  ["CSI-t pixelrapport", "\x1b[4;630;1440t"],
  ["OSC 11 baggrundsfarve (BEL-termineret)", "\x1b]11;rgb:1a1a/1d1d/2121\x07"],
  ["OSC 10 forgrundsfarve (ST-termineret)", "\x1b]10;rgb:ffff/ffff/ffff\x1b\\"],
  ["DCS XTGETTCAP-svar", "\x1bP1+r544e=787465726d\x1b\\"],
  ["konkateneret: fokus + CPR", "\x1b[I\x1b[24;80R"],
  ["konkateneret: DA1 + kitty", "\x1b[?6c\x1b[?1u"],
];

// Human-siden: intet et menneske kan taste/indsaette maa nogensinde
// klassificeres som terminal-svar (saa ville auto-pause udeblive).
const HUMAN: [string, string][] = [
  ["bogstav", "h"],
  ["ord + enter", "hi\r"],
  ["enter", "\r"],
  ["backspace/DEL", "\x7f"],
  ["bare ESC (Escape-tasten)", "\x1b"],
  ["pil op", "\x1b[A"],
  ["pil ned", "\x1b[B"],
  ["ctrl+pil", "\x1b[1;5C"],
  ["SS3-pil (application cursor mode)", "\x1bOA"],
  ["F1 (SS3)", "\x1bOP"],
  ["delete-tasten", "\x1b[3~"],
  ["shift+tab", "\x1b[Z"],
  ["bracketed paste", "\x1b[200~indsat tekst\x1b[201~"],
  ["svar EFTERFULGT af tastetryk (blandet chunk)", "\x1b[Ih"],
  ["ufuldstaendig OSC (ingen terminator)", "\x1b]11;rgb:0000/0000/0000"],
];

describe("isTerminalReply (fix F1 + M0b-FUND 2)", () => {
  it.each(TERMINAL)("terminal-svar: %s", (_navn, seq) => {
    expect(isTerminalReply(seq)).toBe(true);
  });

  it.each(HUMAN)("menneske-input: %s", (_navn, seq) => {
    expect(isTerminalReply(seq)).toBe(false);
  });
});

describe("isBracketedPaste", () => {
  it("genkender bracketed paste", () => {
    expect(isBracketedPaste("\x1b[200~hej\x1b[201~")).toBe(true);
  });

  it("er falsk for piletaster, auto-svar og ren tekst", () => {
    expect(isBracketedPaste("\x1b[A")).toBe(false);
    expect(isBracketedPaste("\x1b[0n")).toBe(false);
    expect(isBracketedPaste("hej")).toBe(false);
    expect(isBracketedPaste("")).toBe(false);
  });

  it("aendrer ikke source-klassifikationen — indsat tekst er stadig human", () => {
    expect(isTerminalReply("\x1b[200~hej\x1b[201~")).toBe(false);
  });
});
