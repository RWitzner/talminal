import { describe, expect, it } from "vitest";
import { cardLabel } from "./cardLabel";

// Kortnavnet er to ting på én gang: en wire-identifier backend'en slår op på,
// og en streng brugeren læser i headeren. Kun den anden er dansk. Testene her
// låser BEGGE retninger — at card-N oversættes, og at alt andet slipper
// uændret igennem, så et toml-seedet kort ikke får påhæftet et opdigtet nummer.

describe("cardLabel (visning; wire-identifieren er uroert)", () => {
  it("oversaetter card-N til Kort N", () => {
    expect(cardLabel("card-1")).toBe("Kort 1");
    expect(cardLabel("card-7")).toBe("Kort 7");
  });

  it("holder flercifrede numre intakte", () => {
    expect(cardLabel("card-12")).toBe("Kort 12");
    expect(cardLabel("card-100")).toBe("Kort 100");
  });

  it("lader toml-seedede navne staa uroert", () => {
    // registry.rs:24 — seedede kort beholder deres eget navn og har intet
    // nummer i navnet. Et gaet ville vaere en paastand uden daekning.
    expect(cardLabel("master")).toBe("master");
    expect(cardLabel("build-watcher")).toBe("build-watcher");
  });

  it("kraever den praecise card-N-form (ingen loese delmatch)", () => {
    expect(cardLabel("card-")).toBe("card-");
    expect(cardLabel("card-abc")).toBe("card-abc");
    expect(cardLabel("discard-3")).toBe("discard-3");
    expect(cardLabel("card-3-old")).toBe("card-3-old");
  });

  it("er en ren funktion af navnet alene", () => {
    expect(cardLabel("")).toBe("");
  });
});
