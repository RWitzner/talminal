/**
 * @vitest-environment happy-dom
 */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Card } from "./Card";
import type { CardState, TerminalCardInfo } from "./types";
import { flushMicrotasks } from "./testHelpers";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  writeClipboardText: vi.fn(async () => true),
  options: [] as Record<string, unknown>[],
  terminals: [] as {
    keyHandler: ((ev: KeyboardEvent) => boolean) | null;
    selection: string;
    cleared: number;
  }[],
  dataHandlers: [] as ((data: string) => void)[],
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("./clipboard", () => ({ writeClipboardText: mocks.writeClipboardText }));

vi.mock("@xterm/xterm", () => ({
  Terminal: class TerminalMock {
    cols = 80;
    rows = 24;
    keyHandler: ((ev: KeyboardEvent) => boolean) | null = null;
    selection = "";
    cleared = 0;

    constructor(options: Record<string, unknown>) {
      mocks.options.push(options);
      mocks.terminals.push(this);
    }

    loadAddon(): void {}
    open(): void {}
    onData(handler: (data: string) => void): { dispose(): void } {
      mocks.dataHandlers.push(handler);
      return { dispose() {} };
    }
    write(): void {}
    writeln(): void {}
    reset(): void {}
    focus(): void {}
    dispose(): void {}
    attachCustomKeyEventHandler(handler: (ev: KeyboardEvent) => boolean): void {
      this.keyHandler = handler;
    }
    getSelection(): string {
      return this.selection;
    }
    clearSelection(): void {
      this.cleared += 1;
      this.selection = "";
    }
  },
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class FitAddonMock {
    activate(): void {}
    fit(): void {}
  },
}));

const card: TerminalCardInfo = {
  kind: "terminal",
  number: 2,
  name: "card-2",
  cwd: "C:\\project",
  profile: "codex",
  running: true,
  exited: null,
  restore_action: null,
  opened_by: null,
  url: null,
  title: null,
};

const runningState: CardState = {
  running: true,
  exited: null,
  owner: "persona",
  epoch: 0,
};


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

describe("Card terminal-opsaetning", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.invoke.mockImplementation(async (command: string) =>
      command === "get_card_state" ? runningState : undefined,
    );
    mocks.listen.mockReset();
    mocks.listen.mockImplementation(async () => () => undefined);
    mocks.writeClipboardText.mockClear();
    mocks.options.length = 0;
    mocks.terminals.length = 0;
    mocks.dataHandlers.length = 0;
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  async function render(): Promise<void> {
    await act(async () => {
      root.render(<Card card={card} />);
      await flushMicrotasks();
    });
  }

  it("giver terminalen scrollback, saa hjulet ruller i stedet for at sende piletaster", async () => {
    await render();
    expect(mocks.options[0]?.scrollback).toBe(2000);
  });

  it("registrerer udklipsholder-handleren", async () => {
    await render();
    expect(mocks.terminals[0]?.keyHandler).toBeTypeOf("function");
  });

  it("Ctrl+C med markering kopierer, rydder markeringen og sender intet til pty'en", async () => {
    await render();
    const term = mocks.terminals[0]!;
    term.selection = "output fra codex";
    const ev = domKey({ key: "c", ctrlKey: true });

    expect(term.keyHandler!(ev)).toBe(false);
    await flushMicrotasks();

    expect(ev.preventDefault).toHaveBeenCalledOnce();
    expect(mocks.writeClipboardText).toHaveBeenCalledWith("output fra codex");
    expect(term.cleared).toBe(1);
  });

  it("Ctrl+C uden markering goer ingenting", async () => {
    await render();
    const term = mocks.terminals[0]!;
    const ev = domKey({ key: "c", ctrlKey: true });

    expect(term.keyHandler!(ev)).toBe(false);
    await flushMicrotasks();

    expect(mocks.writeClipboardText).not.toHaveBeenCalled();
  });

  it("Ctrl+V overlader arbejdet til browseren", async () => {
    await render();
    const term = mocks.terminals[0]!;
    const ev = domKey({ key: "v", ctrlKey: true });

    expect(term.keyHandler!(ev)).toBe(false);
    expect(ev.preventDefault).not.toHaveBeenCalled();
  });

  it("AltGr-tegn naar uroert til terminalen", async () => {
    await render();
    const term = mocks.terminals[0]!;
    const ev = domKey({ key: "@", ctrlKey: true, altKey: true, keyCode: 50 });

    expect(term.keyHandler!(ev)).toBe(true);
    expect(ev.preventDefault).not.toHaveBeenCalled();
  });

  it("indsat tekst hex-dumpes ikke, men ukendte ESC-chunks goer stadig", async () => {
    const debug = vi.spyOn(console, "debug").mockImplementation(() => undefined);
    await render();
    const onData = mocks.dataHandlers[0]!;

    onData("\x1b[200~en lang indsat blok\x1b[201~");
    expect(debug).not.toHaveBeenCalled();

    onData("\x1b[A");
    expect(debug).toHaveBeenCalled();

    debug.mockRestore();
  });
});
