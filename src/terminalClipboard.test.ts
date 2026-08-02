/**
 * @vitest-environment happy-dom
 */

import { describe, expect, it, vi } from "vitest";
import {
  attachTerminalClipboard,
  routeClipboardKey,
  toClipboardKey,
  type ClipboardKeyLike,
} from "./terminalClipboard";

function key(overrides: Partial<ClipboardKeyLike> = {}): ClipboardKeyLike {
  return {
    type: "keydown",
    key: "a",
    ctrlKey: false,
    altKey: false,
    metaKey: false,
    altGraph: false,
    isComposing: false,
    precededByDeadKey: false,
    ...overrides,
  };
}

describe("routeClipboardKey", () => {
  it("raekke 1: kun keydown traeffer beslutningen", () => {
    expect(routeClipboardKey(key({ type: "keypress", key: "c", ctrlKey: true }))).toBe("pty");
    expect(routeClipboardKey(key({ type: "keyup", key: "c", ctrlKey: true }))).toBe("pty");
  });

  it("raekke 2: doedtast-vaernet lader tasten passere til xterm", () => {
    expect(
      routeClipboardKey(key({ key: "c", ctrlKey: true, precededByDeadKey: true })),
    ).toBe("pty");
    expect(
      routeClipboardKey(key({ key: "v", ctrlKey: true, precededByDeadKey: true })),
    ).toBe("pty");
  });

  it("raekke 3: en igangvaerende IME-komposition ejer tasten", () => {
    expect(routeClipboardKey(key({ key: "c", ctrlKey: true, isComposing: true }))).toBe("pty");
  });

  it("raekke 4: AltGr-vaernet — Windows rapporterer AltGr som Ctrl+Alt", () => {
    expect(routeClipboardKey(key({ key: "@", ctrlKey: true, altKey: true }))).toBe("pty");
    expect(routeClipboardKey(key({ key: "@", ctrlKey: true, altGraph: true }))).toBe("pty");
    expect(routeClipboardKey(key({ key: "[", ctrlKey: true, altKey: true }))).toBe("pty");
  });

  it("raekke 5: Win+tast tilhoerer OS'et", () => {
    expect(routeClipboardKey(key({ key: "c", ctrlKey: true, metaKey: true }))).toBe("pty");
  });

  it("raekke 6: almindelig skrivning er terminalens", () => {
    expect(routeClipboardKey(key({ key: "c" }))).toBe("pty");
    expect(routeClipboardKey(key({ key: "v" }))).toBe("pty");
  });

  it("raekke 7 og 8: Ctrl+C kopierer, Ctrl+V indsaetter", () => {
    expect(routeClipboardKey(key({ key: "c", ctrlKey: true }))).toBe("copy");
    expect(routeClipboardKey(key({ key: "v", ctrlKey: true }))).toBe("paste");
  });

  it("shiftKey indgaar ikke — Ctrl+Shift+C/V er aliaser", () => {
    expect(routeClipboardKey(key({ key: "C", ctrlKey: true }))).toBe("copy");
    expect(routeClipboardKey(key({ key: "V", ctrlKey: true }))).toBe("paste");
  });

  it("raekke 9: alle oevrige Ctrl-kombinationer er agentens", () => {
    for (const k of ["a", "u", "r", "d", "l", "z"]) {
      expect(routeClipboardKey(key({ key: k, ctrlKey: true }))).toBe("pty");
    }
  });
});

describe("toClipboardKey", () => {
  it("laeser AltGraph via getModifierState", () => {
    const ev = {
      type: "keydown",
      key: "@",
      ctrlKey: true,
      altKey: false,
      metaKey: false,
      isComposing: false,
      keyCode: 50,
      getModifierState: (name: string) => name === "AltGraph",
    } as unknown as KeyboardEvent;
    expect(toClipboardKey(ev, false).altGraph).toBe(true);
  });

  it("behandler keyCode 229 som komposition", () => {
    const ev = {
      type: "keydown",
      key: "Process",
      ctrlKey: false,
      altKey: false,
      metaKey: false,
      isComposing: false,
      keyCode: 229,
      getModifierState: () => false,
    } as unknown as KeyboardEvent;
    expect(toClipboardKey(ev, false).isComposing).toBe(true);
  });

  it("baerer precededByDeadKey ind udefra", () => {
    const ev = {
      type: "keydown",
      key: "c",
      ctrlKey: true,
      altKey: false,
      metaKey: false,
      isComposing: false,
      keyCode: 67,
      getModifierState: () => false,
    } as unknown as KeyboardEvent;
    expect(toClipboardKey(ev, true).precededByDeadKey).toBe(true);
  });
});

interface FakeTerm {
  getSelection(): string;
  clearSelection(): void;
  attachCustomKeyEventHandler(handler: (ev: KeyboardEvent) => boolean): void;
  handler: ((ev: KeyboardEvent) => boolean) | null;
  selection: string;
  cleared: number;
}

function fakeTerm(selection = ""): FakeTerm {
  const state: FakeTerm = {
    handler: null,
    selection,
    cleared: 0,
    getSelection: () => state.selection,
    clearSelection: () => {
      state.cleared += 1;
      state.selection = "";
    },
    attachCustomKeyEventHandler: (handler) => {
      state.handler = handler;
    },
  };
  return state;
}

function domKey(overrides: Record<string, unknown> = {}): KeyboardEvent & {
  preventDefault: ReturnType<typeof vi.fn>;
} {
  return {
    type: "keydown",
    key: "a",
    ctrlKey: false,
    altKey: false,
    metaKey: false,
    isComposing: false,
    keyCode: 0,
    getModifierState: () => false,
    preventDefault: vi.fn(),
    ...overrides,
  } as unknown as KeyboardEvent & { preventDefault: ReturnType<typeof vi.fn> };
}

describe("attachTerminalClipboard", () => {
  it("kopierer markeringen, rydder den og preventDefault'er", () => {
    const term = fakeTerm("hej fra terminalen");
    const writeText = vi.fn(async () => true);
    attachTerminalClipboard(term, writeText);
    const ev = domKey({ key: "c", ctrlKey: true });

    expect(term.handler!(ev)).toBe(false);
    expect(ev.preventDefault).toHaveBeenCalledOnce();
    expect(writeText).toHaveBeenCalledWith("hej fra terminalen");
    expect(term.cleared).toBe(1);
  });

  it("uden markering sker der ingenting — heller ikke 0x03", () => {
    const term = fakeTerm("");
    const writeText = vi.fn(async () => true);
    attachTerminalClipboard(term, writeText);
    const ev = domKey({ key: "c", ctrlKey: true });

    expect(term.handler!(ev)).toBe(false);
    expect(ev.preventDefault).toHaveBeenCalledOnce();
    expect(writeText).not.toHaveBeenCalled();
    expect(term.cleared).toBe(0);
  });

  it("indsaet returnerer false UDEN preventDefault — browseren skal gøre arbejdet", () => {
    const term = fakeTerm("");
    attachTerminalClipboard(term, vi.fn(async () => true));
    const ev = domKey({ key: "v", ctrlKey: true });

    expect(term.handler!(ev)).toBe(false);
    expect(ev.preventDefault).not.toHaveBeenCalled();
  });

  it("pty-vejen returnerer true og roerer ikke eventet", () => {
    const term = fakeTerm("noget markeret");
    attachTerminalClipboard(term, vi.fn(async () => true));
    const ev = domKey({ key: "a", ctrlKey: true });

    expect(term.handler!(ev)).toBe(true);
    expect(ev.preventDefault).not.toHaveBeenCalled();
    expect(term.cleared).toBe(0);
  });

  it("doedtast-spejlet: Ctrl+C efter Dead + Control passerer til xterm", () => {
    const term = fakeTerm("markeret");
    const writeText = vi.fn(async () => true);
    attachTerminalClipboard(term, writeText);

    expect(term.handler!(domKey({ key: "Dead" }))).toBe(true);
    expect(term.handler!(domKey({ key: "Control", ctrlKey: true }))).toBe(true);
    expect(term.handler!(domKey({ key: "c", ctrlKey: true }))).toBe(true);
    expect(writeText).not.toHaveBeenCalled();
    expect(term.handler!(domKey({ key: "Control", ctrlKey: true }))).toBe(true);
    expect(term.handler!(domKey({ key: "c", ctrlKey: true }))).toBe(false);
    expect(writeText).toHaveBeenCalledOnce();
  });

  it("AltGr taeller som doed tast i spejlet", () => {
    const term = fakeTerm("markeret");
    const writeText = vi.fn(async () => true);
    attachTerminalClipboard(term, writeText);

    expect(term.handler!(domKey({ key: "AltGraph", altKey: true }))).toBe(true);
    expect(term.handler!(domKey({ key: "Control", ctrlKey: true }))).toBe(true);
    expect(term.handler!(domKey({ key: "c", ctrlKey: true }))).toBe(true);
    expect(writeText).not.toHaveBeenCalled();
  });

  it("modifier-tryk uden forudgaaende doedtast rammer ikke spejlet", () => {
    const term = fakeTerm("markeret");
    const writeText = vi.fn(async () => true);
    attachTerminalClipboard(term, writeText);

    expect(term.handler!(domKey({ key: "Shift", shiftKey: true }))).toBe(true);
    expect(term.handler!(domKey({ key: "Control", ctrlKey: true }))).toBe(true);
    expect(term.handler!(domKey({ key: "c", ctrlKey: true }))).toBe(false);
    expect(writeText).toHaveBeenCalledOnce();
  });

  it("spejlet opdateres kun paa keydown", () => {
    const term = fakeTerm("markeret");
    attachTerminalClipboard(term, vi.fn(async () => true));

    term.handler!(domKey({ key: "Dead", type: "keyup" }));
    expect(term.handler!(domKey({ key: "c", ctrlKey: true }))).toBe(false);
  });

  it("logger naar udklipsholderen afviser", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const term = fakeTerm("markeret");
    attachTerminalClipboard(term, vi.fn(async () => false));

    term.handler!(domKey({ key: "c", ctrlKey: true }));
    await Promise.resolve();
    await Promise.resolve();

    expect(error).toHaveBeenCalled();
    error.mockRestore();
  });
});
