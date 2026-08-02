import { describe, expect, it } from "vitest";
import { cardColor } from "./colors";

// Task 4 (MVP-pathen): kort-farve er en funktion af LOKAL kort-tilstand alene
// (running/exited fra spawn-flow + card-exit + get_card_state-hydrering).
// Presence-/pause-strengene er ude: ingen controller-heartbeat, ingen
// stale-regel, ingen blaa pause-farve, ingen gul attention-farve.

describe("cardColor (lokal running/exited-tilstand)", () => {
  it("green: kortet koerer", () => {
    expect(cardColor({ running: true, exited: null })).toBe("green");
  });

  it("green: running vinder over en gammel exit-kode (frisk respawn)", () => {
    expect(cardColor({ running: true, exited: 1 })).toBe("green");
  });

  it("red: exited med kode != 0 (crash/kill)", () => {
    expect(cardColor({ running: false, exited: 1 })).toBe("red");
  });

  it("red: exited med anden ikke-nul kode", () => {
    expect(cardColor({ running: false, exited: 130 })).toBe("red");
  });

  it("neutral: exited med kode 0 (rent afsluttet)", () => {
    expect(cardColor({ running: false, exited: 0 })).toBe("neutral");
  });

  it("gray: ikke startet (hverken running eller exit-historik)", () => {
    expect(cardColor({ running: false, exited: null })).toBe("gray");
  });
});
