import { describe, expect, it } from "vitest";
import {
  assignAgentSlots,
  formatClock,
  hopTone,
  splitFencedSegments,
} from "./chatText";

describe("splitFencedSegments", () => {
  it("giver ét text-segment naar der ikke er hegn", () => {
    expect(splitFencedSegments("bare prosa\nover to linjer")).toEqual([
      { kind: "text", body: "bare prosa\nover to linjer", lang: "" },
    ]);
  });

  it("bevarer kodeblokken ORDRET — tabulator, citationstegn og tom linje", () => {
    // Praecis runbook §2.8's paastand: teksten krydser traaden uaendret.
    const code = 'fn main() {\n\tlet s = "quotes";\n\n\tprintln!("{s}");\n}';
    const segments = splitFencedSegments(`Her:\n\`\`\`rust\n${code}\n\`\`\`\nSlut.`);
    expect(segments).toEqual([
      { kind: "text", body: "Her:", lang: "" },
      { kind: "code", body: code, lang: "rust" },
      { kind: "text", body: "Slut.", lang: "" },
    ]);
    const block = segments[1].body;
    expect(block).toContain("\t");
    expect(block).toContain('"quotes"');
    expect(block.split("\n")).toHaveLength(5);
    expect(block.split("\n")[2]).toBe("");
  });

  it("laeser et uafsluttet hegn som kode helt til enden", () => {
    expect(splitFencedSegments("se her\n```\nlet x = 1;")).toEqual([
      { kind: "text", body: "se her", lang: "" },
      { kind: "code", body: "let x = 1;", lang: "" },
    ]);
  });

  it("dropper blanke prosa-huller, men beholder en tom kodeblok", () => {
    expect(splitFencedSegments("```\n\n```")).toEqual([
      { kind: "code", body: "", lang: "" },
    ]);
    expect(splitFencedSegments("   \n\n  ")).toEqual([]);
  });

  it("rammer kun hegn i linjestart", () => {
    const inline = "brug ```kode``` midt i en linje";
    expect(splitFencedSegments(inline)).toEqual([
      { kind: "text", body: inline, lang: "" },
    ]);
  });
});

describe("formatClock", () => {
  it("nulpolstrer til HH:MM i lokal tid", () => {
    expect(formatClock(new Date(2026, 6, 26, 9, 5).getTime())).toBe("09:05");
    expect(formatClock(new Date(2026, 6, 26, 22, 41).getTime())).toBe("22:41");
  });

  it("giver tom streng for et ugyldigt tidsstempel", () => {
    expect(formatClock(Number.NaN)).toBe("");
  });
});

describe("assignAgentSlots", () => {
  it("tildeler plads efter foerste optraeden, ikke efter kortnummer", () => {
    const slots = assignAgentSlots(["card-9", "card-2", "card-9", "card-2"]);
    expect(slots.get("card-9")).toBe(0);
    expect(slots.get("card-2")).toBe(1);
    expect(slots.size).toBe(2);
  });

  it("giver ALDRIG to agenter samme plads", () => {
    const slots = assignAgentSlots(["card-1", "card-21", "card-41"]);
    expect(new Set(slots.values()).size).toBe(3);
  });
});

describe("hopTone", () => {
  it("advarer fra fire tilbage og melder braendt ved loftet", () => {
    expect(hopTone(0, 20)).toBe("calm");
    expect(hopTone(15, 20)).toBe("calm");
    expect(hopTone(16, 20)).toBe("warn");
    expect(hopTone(19, 20)).toBe("warn");
    expect(hopTone(20, 20)).toBe("spent");
  });
});
