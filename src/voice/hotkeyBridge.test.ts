import { describe, expect, it } from "vitest";
import { createHotkeyBridge } from "./hotkeyBridge";

function makeBridge() {
  const calls: string[] = [];
  const debug: string[] = [];
  const warn: string[] = [];
  const bridge = createHotkeyBridge({
    onPress: () => calls.push("press"),
    onRelease: () => calls.push("release"),
    onDebug: (message) => debug.push(message),
    onWarn: (message) => warn.push(message),
  });
  return { bridge, calls, debug, warn };
}

describe("createHotkeyBridge — to bindinger, én mikrofon", () => {
  it("aabner ikke mikrofonen to gange naar begge bindinger trykkes", () => {
    // En slot kan have to keyboard-bindinger, og BEGGE registrerer sig paa den
    // samme bridge. Uden ejerskabslaasen ville ét hold give to onPress, og
    // STT-sessionen blev startet to gange.
    const h = makeBridge();
    h.bridge.domPress(0);
    h.bridge.domPress(1);
    expect(h.calls).toEqual(["press"]);
    expect(h.debug).toContain("voice.ptt.dom_second_binding_ignored");
  });

  it("lader kun EJEREN lukke holdet", () => {
    // Slipper den anden binding midt i holdet, holder brugeren stadig fysisk
    // paa den foerste — et release her ville afbryde midt i en saetning.
    const h = makeBridge();
    h.bridge.domPress(0);
    h.bridge.domPress(1);
    h.bridge.domRelease(1);
    expect(h.calls).toEqual(["press"]);
    h.bridge.domRelease(0);
    expect(h.calls).toEqual(["press", "release"]);
  });

  it("lader den anden binding eje et NYT hold", () => {
    const h = makeBridge();
    h.bridge.domPress(0);
    h.bridge.domRelease(0);
    h.calls.length = 0;
    h.bridge.domPress(1);
    h.bridge.domRelease(1);
    expect(h.calls).toEqual(["press", "release"]);
  });

  it("selvhealingen rydder ogsaa ejerskabet", () => {
    // En native release under aktivt DOM-hold beviser at DOM'ens keyup gik
    // tabt. Blev ejerskabet staaende, ville den binding aldrig kunne fyre igen.
    const h = makeBridge();
    h.bridge.domPress(1);
    h.bridge.nativeEdge("release");
    expect(h.calls).toEqual(["press", "release"]);
    h.calls.length = 0;
    h.bridge.nativeEdge("press");
    expect(h.calls).toEqual(["press"]);
  });
});

describe("createHotkeyBridge", () => {
  it("lader DOM'en foere indtil polleren er klar", () => {
    const h = makeBridge();
    h.bridge.domPress();
    h.bridge.domRelease();
    expect(h.calls).toEqual(["press", "release"]);
  });

  it("giver polleren primatet efter det DOM-hold der spaendte over handoff'et", () => {
    const h = makeBridge();
    // Holdet begynder mens kun DOM'en fungerer...
    h.bridge.domPress();
    h.bridge.markNativeReady();
    // ...og slippet kommer stadig fra DOM'en, fordi det var DEN der greb det.
    h.bridge.domRelease();
    expect(h.calls).toEqual(["press", "release"]);

    // Naeste hold: DOM'en er traadt tilbage, polleren foerer.
    h.calls.length = 0;
    h.bridge.domPress();
    expect(h.calls).toEqual([]);
    expect(h.debug).toEqual(["voice.ptt.dom_saw_combo_native_primacy"]);

    h.bridge.nativeEdge("press");
    h.bridge.nativeEdge("release");
    expect(h.calls).toEqual(["press", "release"]);
  });

  it("tager primatet med det samme naar polleren melder klar uden aktivt hold", () => {
    const h = makeBridge();
    h.bridge.markNativeReady();
    h.bridge.nativeEdge("press");
    h.bridge.nativeEdge("release");
    expect(h.calls).toEqual(["press", "release"]);
    expect(h.debug).toEqual([]);
  });

  it("lader et hold der ALLEREDE koerer i DOM'en gøre sig faerdigt", () => {
    // Handoff-vinduet: polleren bliver klar midt i et DOM-hold. Trykket maa
    // ikke tabes, og slippet skal stadig komme fra DOM'en.
    const h = makeBridge();
    h.bridge.domPress();
    h.bridge.markNativeReady();
    expect(h.bridge.isDomActive()).toBe(true);
    h.bridge.domRelease();
    expect(h.calls).toEqual(["press", "release"]);
  });

  it("healer et stale DOM-hold naar polleren melder release", () => {
    // Regressionen "flere doede tryk i traek indtil naeste blur": DOM'ens
    // keyup gik tabt i WebView2-leverancen, saa domActive stod laast — og
    // hver senere native kant blev slugt.
    const h = makeBridge();
    h.bridge.domPress();
    expect(h.calls).toEqual(["press"]);

    h.bridge.nativeEdge("release");
    expect(h.calls).toEqual(["press", "release"]);
    expect(h.warn).toEqual(["voice.ptt.dom_hold_stale_selfhealed"]);
    expect(h.bridge.isDomActive()).toBe(false);

    // Og efter healingen foerer polleren.
    h.calls.length = 0;
    h.bridge.nativeEdge("press");
    h.bridge.nativeEdge("release");
    expect(h.calls).toEqual(["press", "release"]);
  });

  it("ignorerer en native PRESS under et aktivt DOM-hold", () => {
    // Den kan kun vaere samme holds duplikat inden for pollerens 5 ms.
    const h = makeBridge();
    h.bridge.domPress();
    h.bridge.nativeEdge("press");
    expect(h.calls).toEqual(["press"]);
    expect(h.bridge.isDomActive()).toBe(true);
  });

  it("to broer deler ingen tilstand", () => {
    // Hele grunden til at logikken blev et modul: PTT og diktering skal
    // kunne staa i hver sin tilstand uden at paavirke hinanden.
    const a = makeBridge();
    const b = makeBridge();
    a.bridge.markNativeReady();
    a.bridge.nativeEdge("press");

    expect(a.calls).toEqual(["press"]);
    expect(b.calls).toEqual([]);

    b.bridge.domPress();
    expect(b.calls).toEqual(["press"]);
    expect(b.debug).toEqual([]);
  });
});
