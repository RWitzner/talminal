import { describe, expect, it } from "vitest";
import { createMicArbiter } from "./micArbiter";

describe("createMicArbiter", () => {
  it("giver mikrofonen til den foerste der beder om den", () => {
    const mic = createMicArbiter();
    expect(mic.acquire("ptt")).toBe(true);
    expect(mic.owner()).toBe("ptt");
  });

  it("afviser den anden mens den foerste holder", () => {
    const mic = createMicArbiter();
    mic.acquire("ptt");
    expect(mic.acquire("dictation")).toBe(false);
    expect(mic.owner()).toBe("ptt");
  });

  it("lader et afvist trykks SLIP staa helt uden virkning", () => {
    // Kernen i hele konstruktionen: beslutningen huskes fra press til
    // release. Uden den ville dikteringens slip rydde ejerskabet — og saa
    // stod PTT'ens hold tilbage uden mikrofon, midt i en optagelse.
    const mic = createMicArbiter();
    mic.acquire("ptt");
    expect(mic.acquire("dictation")).toBe(false);

    expect(mic.release("dictation")).toBe(false);
    expect(mic.owner()).toBe("ptt");

    expect(mic.release("ptt")).toBe(true);
    expect(mic.owner()).toBeNull();
  });

  it("er ledig igen med det samme efter et slip", () => {
    // Ejerskabet slippes ved RELEASE og ikke naar sessionen lander idle:
    // mikrofonen ER lukket dér, uanset at pipelinen stadig afspiller et klip.
    const mic = createMicArbiter();
    mic.acquire("ptt");
    mic.release("ptt");
    expect(mic.acquire("dictation")).toBe(true);
  });

  it("lader den samme ejer tage den igen efter sit eget slip", () => {
    const mic = createMicArbiter();
    mic.acquire("dictation");
    mic.release("dictation");
    expect(mic.acquire("dictation")).toBe(true);
  });

  it("et dobbelt-slip aabner ikke for at stjaele mikrofonen", () => {
    const mic = createMicArbiter();
    mic.acquire("ptt");
    mic.release("ptt");
    mic.acquire("dictation");
    // PTT'ens residuale slip (fx en sen native release-kant) maa ikke rive
    // mikrofonen fra dikteringen.
    expect(mic.release("ptt")).toBe(false);
    expect(mic.owner()).toBe("dictation");
  });
});
