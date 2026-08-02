/**
 * @vitest-environment happy-dom
 */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Card, PREPARE_FRESH_SPAWN_EVENT } from "./Card";
import type { CardInfo, CardState } from "./types";

interface TerminalDouble {
  cols: number;
  rows: number;
  container: HTMLElement | null;
  screen: string;
}

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  fit: vi.fn(),
  reset: vi.fn(),
  focus: vi.fn(),
  terminals: [] as TerminalDouble[],
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

vi.mock("@xterm/xterm", () => ({
  Terminal: class TerminalMock implements TerminalDouble {
    cols = 80;
    rows = 24;
    container: HTMLElement | null = null;
    screen = "";

    constructor() {
      mocks.terminals.push(this);
    }

    loadAddon(addon: { activate?(terminal: TerminalDouble): void }): void {
      addon.activate?.(this);
    }

    open(container: HTMLElement): void {
      this.container = container;
    }

    onData(): { dispose(): void } {
      return { dispose() {} };
    }

    write(data: string | Uint8Array, callback?: () => void): void {
      this.screen +=
        typeof data === "string" ? data : new TextDecoder().decode(data);
      callback?.();
    }

    writeln(): void {}

    reset(): void {
      this.screen = "";
      mocks.reset();
    }

    focus(): void {
      mocks.focus();
    }

    dispose(): void {}
    attachCustomKeyEventHandler(): void {}
    getSelection(): string {
      return "";
    }
    clearSelection(): void {}
  },
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class FitAddonMock {
    private terminal: TerminalDouble | null = null;

    activate(terminal: TerminalDouble): void {
      this.terminal = terminal;
    }

    fit(): void {
      mocks.fit();
      const terminal = this.terminal;
      const container = terminal?.container;
      if (!terminal || !container) return;
      terminal.cols = Math.max(2, Math.floor(container.clientWidth / 10));
      terminal.rows = Math.max(1, Math.floor(container.clientHeight / 20));
    }
  },
}));

class ResizeObserverMock {
  static instances: ResizeObserverMock[] = [];

  constructor(private readonly callback: ResizeObserverCallback) {
    ResizeObserverMock.instances.push(this);
  }

  observe(): void {}

  unobserve(): void {}

  disconnect(): void {}

  trigger(): void {
    this.callback([], this as unknown as ResizeObserver);
  }
}

const card: CardInfo = {
  kind: "terminal",
  number: 1,
  name: "card-1",
  cwd: "C:\\project",
  profile: "claude",
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

async function flushMicrotasks(): Promise<void> {
  for (let index = 0; index < 6; index += 1) {
    await Promise.resolve();
  }
}

/** Knapper i kortet UNDEN den permanente ✕ i headeren — altsaa knapperne i
 *  Start-/exit-overlayet. Flere tests brugte "ingen knapper overhovedet" som
 *  stedfortraeder for "overlayet vises ikke"; da luk-knappen kom (ejer-
 *  beslutning 2026-07-26) skulle den paastand skaerpes, ikke slaekkes. */
function overlayButtons(host: HTMLElement): HTMLButtonElement[] {
  return Array.from(host.querySelectorAll<HTMLButtonElement>("section button")).filter(
    (button) => !button.hasAttribute("data-card-close-action"),
  );
}

describe("Card responsive PTY resize", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal("ResizeObserver", ResizeObserverMock);
    (
      globalThis as typeof globalThis & {
        IS_REACT_ACT_ENVIRONMENT: boolean;
      }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    ResizeObserverMock.instances = [];
    mocks.terminals.length = 0;
    mocks.fit.mockClear();
    mocks.reset.mockClear();
    mocks.focus.mockClear();
    mocks.listeners.clear();
    mocks.listen.mockReset().mockImplementation(
      async (event: string, handler: (event: { payload: unknown }) => void) => {
        mocks.listeners.set(event, handler);
        return () => mocks.listeners.delete(event);
      },
    );
    mocks.invoke.mockReset().mockImplementation(async (command: string) => {
      if (command === "get_card_state") return runningState;
      return undefined;
    });
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("ignorerer 0x0 og fitter/resizer PTY ved efterfølgende reelle dimensionsændringer", async () => {
    await act(async () => {
      root.render(<Card card={card} />);
      await flushMicrotasks();
    });

    const section = host.querySelector("section");
    const termWrap = section?.children.item(1);
    const container = termWrap?.children.item(0) as HTMLElement | null;
    expect(container).not.toBeNull();
    expect(ResizeObserverMock.instances).toHaveLength(1);

    let width = 0;
    let height = 0;
    Object.defineProperties(container!, {
      clientWidth: { configurable: true, get: () => width },
      clientHeight: { configurable: true, get: () => height },
    });
    const observer = ResizeObserverMock.instances[0];

    await act(async () => {
      observer.trigger();
      vi.advanceTimersByTime(150);
      await flushMicrotasks();
    });
    expect(mocks.fit).not.toHaveBeenCalled();
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "resize_pty"),
    ).toEqual([]);

    width = 800;
    height = 400;
    await act(async () => {
      observer.trigger();
      vi.advanceTimersByTime(150);
      await flushMicrotasks();
    });
    expect(mocks.fit).toHaveBeenCalledTimes(1);
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "resize_pty"),
    ).toEqual([
      ["resize_pty", { name: "card-1", cols: 79, rows: 20 }],
      ["resize_pty", { name: "card-1", cols: 80, rows: 20 }],
    ]);

    width = 600;
    height = 300;
    await act(async () => {
      observer.trigger();
      vi.advanceTimersByTime(150);
      await flushMicrotasks();
    });
    expect(mocks.fit).toHaveBeenCalledTimes(2);
    expect(mocks.invoke).toHaveBeenLastCalledWith("resize_pty", {
      name: "card-1",
      cols: 60,
      rows: 15,
    });
  });

  it("samler kortmetadata i én header uden stop-knap", async () => {
    await act(async () => {
      root.render(<Card card={card} focused />);
      await flushMicrotasks();
    });

    const headers = host.querySelectorAll("section > header");
    const cwd = Array.from(
      headers[0].querySelectorAll<HTMLElement>("[title]"),
    ).find((element) => element.title === card.cwd);

    expect(headers).toHaveLength(1);
    expect(
      host.querySelector("[data-card-number-badge]")?.textContent,
    ).toBe("1");
    // Headeren laeses paa dansk; kortets wire-navn er stadig card-1 (det er
    // det close_card/spawn kaldes med).
    expect(headers[0].textContent).toContain("Kort 1");
    expect(headers[0].textContent).not.toContain("card-1");
    expect(cwd?.textContent).toBe("project");
    // Ejer-beslutning 2026-07-20 STÅR VED MAGT: ingen STOP-knap i kortet — en
    // kørende proces dræbes via voice (kill_card), ikke fra headeren.
    expect(host.querySelector("[data-card-stop-action]")).toBeNull();
    // Ejer-beslutning 2026-07-26 tilføjer en LUK-knap. De to er forskellige
    // handlinger: stop dræber processen, luk fjerner kortet. Baggrunden er at
    // stemmen var den ENESTE luk-vej for et terminalkort, mens browser-kortet
    // havde sit ✕ — og en luk-vej man ikke kan se, findes ikke for den der
    // ikke kender den i forvejen. Headerens eneste knap er derfor ✕.
    const buttons = host.querySelectorAll("section button");
    expect(buttons).toHaveLength(1);
    expect(buttons[0].hasAttribute("data-card-close-action")).toBe(true);
  });

  it("synkroniserer en ny running snapshot-identitet efter voice restart", async () => {
    await act(async () => {
      root.render(<Card card={card} />);
      await flushMicrotasks();
    });

    const stoppedState = { ...runningState, running: false, exited: 0 };
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_card_state") return stoppedState;
      return undefined;
    });
    await act(async () => {
      mocks.listeners.get("card-exit")?.({ payload: { name: card.name } });
      await flushMicrotasks();
    });
    expect(host.textContent).toContain("Kortet er afsluttet.");

    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_card_state") return runningState;
      return undefined;
    });
    await act(async () => {
      root.render(<Card card={{ ...card }} />);
      await flushMicrotasks();
    });
    expect(host.textContent).not.toContain("Kortet er afsluttet.");
    expect(overlayButtons(host)).toHaveLength(0);
  });

  it("afviser et sent gammelt exit-event via aktuel Rust-state", async () => {
    await act(async () => {
      root.render(<Card card={card} />);
      await flushMicrotasks();
    });

    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_card_state") return runningState;
      return undefined;
    });
    await act(async () => {
      mocks.listeners.get("card-exit")?.({ payload: { name: card.name } });
      await flushMicrotasks();
    });

    expect(host.textContent).not.toContain("Kortet er afsluttet.");
    expect(overlayButtons(host)).toHaveLength(0);
    expect(mocks.reset).not.toHaveBeenCalled();
  });

  it("resetter før frisk voice spawn og bevarer ny output gennem snapshot-read", async () => {
    const stoppedCard = { ...card, running: false, exited: 0 };
    const stoppedState = { ...runningState, running: false, exited: 0 };
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_card_state") return stoppedState;
      return undefined;
    });
    await act(async () => {
      root.render(<Card card={stoppedCard} />);
      await flushMicrotasks();
    });
    const container = host.querySelector("section > div > div") as HTMLElement;
    Object.defineProperties(container, {
      clientWidth: { configurable: true, get: () => 800 },
      clientHeight: { configurable: true, get: () => 400 },
    });
    mocks.invoke.mockClear();
    mocks.reset.mockClear();
    let resolveState!: (state: CardState) => void;
    const stateRead = new Promise<CardState>((resolve) => {
      resolveState = resolve;
    });
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "get_card_state") return stateRead;
      return undefined;
    });

    await act(async () => {
      window.dispatchEvent(
        new CustomEvent(PREPARE_FRESH_SPAWN_EVENT, {
          detail: { name: card.name },
        }),
      );
    });
    expect(mocks.reset).toHaveBeenCalledTimes(1);

    await act(async () => {
      root.render(<Card card={{ ...card }} />);
      await flushMicrotasks();
    });
    expect(mocks.reset).toHaveBeenCalledTimes(1);

    await act(async () => {
      mocks.listeners.get("pty-output")?.({
        payload: {
          name: card.name,
          data_b64: btoa("NEW-RUN-SENTINEL"),
        },
      });
      resolveState(runningState);
      await flushMicrotasks();
    });

    expect(mocks.reset).toHaveBeenCalledTimes(1);
    expect(mocks.terminals[0].screen).toContain("NEW-RUN-SENTINEL");
    expect(mocks.invoke).toHaveBeenCalledWith("resize_pty", {
      name: "card-1",
      cols: 80,
      rows: 20,
    });
  });

  it("resizer ny PTY når PREPARE overhaler gammelt exit-event", async () => {
    await act(async () => {
      root.render(<Card card={card} />);
      await flushMicrotasks();
    });
    const container = host.querySelector("section > div > div") as HTMLElement;
    Object.defineProperties(container, {
      clientWidth: { configurable: true, get: () => 800 },
      clientHeight: { configurable: true, get: () => 400 },
    });
    mocks.invoke.mockClear();
    mocks.reset.mockClear();
    let resolveState!: (state: CardState) => void;
    const stateRead = new Promise<CardState>((resolve) => {
      resolveState = resolve;
    });
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "get_card_state") return stateRead;
      return Promise.resolve();
    });

    await act(async () => {
      window.dispatchEvent(
        new CustomEvent(PREPARE_FRESH_SPAWN_EVENT, {
          detail: { name: card.name },
        }),
      );
      root.render(<Card card={{ ...card }} />);
      await flushMicrotasks();
    });
    expect(mocks.reset).toHaveBeenCalledTimes(1);
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "resize_pty"),
    ).toEqual([]);

    await act(async () => {
      mocks.listeners.get("card-exit")?.({ payload: { name: card.name } });
      await flushMicrotasks();
    });
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "resize_pty"),
    ).toEqual([]);

    await act(async () => {
      resolveState(runningState);
      await flushMicrotasks();
    });
    expect(mocks.reset).toHaveBeenCalledTimes(1);
    expect(mocks.invoke).toHaveBeenCalledWith("resize_pty", {
      name: "card-1",
      cols: 80,
      rows: 20,
    });
  });

  it("ignorerer lifecycle-read der startede før manuel frisk spawn", async () => {
    const stoppedCard = { ...card, running: false, exited: 0 };
    const pendingReads: Array<(state: CardState) => void> = [];
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "get_card_state") {
        return new Promise<CardState>((resolve) => pendingReads.push(resolve));
      }
      return Promise.resolve();
    });

    await act(async () => {
      root.render(<Card card={stoppedCard} />);
      await flushMicrotasks();
    });
    const freshButton = Array.from(
      host.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent === "Frisk session");
    expect(freshButton).toBeDefined();

    await act(async () => {
      freshButton!.click();
      await flushMicrotasks();
    });
    expect(host.textContent).not.toContain("Kortet er afsluttet.");

    await act(async () => {
      pendingReads.forEach((resolve) =>
        resolve({ ...runningState, running: false, exited: 0 }),
      );
      await flushMicrotasks();
    });
    expect(host.textContent).not.toContain("Kortet er afsluttet.");
    expect(mocks.reset).toHaveBeenCalledTimes(1);
  });

  it("fokuserer xterm efter vellykket Fortsæt fra exit-overlayet", async () => {
    const stoppedCard = { ...card, running: false, exited: 0 };
    const stoppedState = { ...runningState, running: false, exited: 0 };
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_card_state") return stoppedState;
      return undefined;
    });
    await act(async () => {
      root.render(<Card card={stoppedCard} focused />);
      await flushMicrotasks();
    });
    mocks.invoke.mockClear();
    mocks.focus.mockClear();
    const continueButton = Array.from(
      host.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent === "Fortsæt (--continue)");
    expect(continueButton).toBeDefined();

    await act(async () => {
      continueButton!.click();
      await flushMicrotasks();
      vi.runOnlyPendingTimers();
    });

    expect(mocks.invoke).toHaveBeenCalledWith("respawn_card", {
      name: "card-1",
    });
    expect(mocks.focus).toHaveBeenCalledTimes(1);
  });

  it("stjæler ikke fokus tilbage efter type-mode forlades under spawn", async () => {
    const stoppedCard = { ...card, running: false, exited: 0 };
    const stoppedState = { ...runningState, running: false, exited: 0 };
    let resolveSpawn!: () => void;
    const spawn = new Promise<void>((resolve) => {
      resolveSpawn = resolve;
    });
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "get_card_state") return Promise.resolve(stoppedState);
      if (command === "respawn_card") return spawn;
      return Promise.resolve();
    });
    await act(async () => {
      root.render(<Card card={stoppedCard} focused />);
      await flushMicrotasks();
    });
    mocks.focus.mockClear();
    const continueButton = Array.from(
      host.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent === "Fortsæt (--continue)");

    await act(async () => {
      continueButton!.click();
      root.render(<Card card={stoppedCard} focused={false} />);
      resolveSpawn();
      await flushMicrotasks();
      vi.runOnlyPendingTimers();
    });

    expect(mocks.focus).not.toHaveBeenCalled();
  });

  it("fokuserer ikke xterm når frisk manuel spawn fejler", async () => {
    const stoppedCard = { ...card, running: false, exited: 0 };
    const stoppedState = { ...runningState, running: false, exited: 0 };
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_card_state") return stoppedState;
      if (command === "spawn_card") throw new Error("spawn fejlede");
      return undefined;
    });
    await act(async () => {
      root.render(<Card card={stoppedCard} />);
      await flushMicrotasks();
    });
    mocks.focus.mockClear();
    const freshButton = Array.from(
      host.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent === "Frisk session");
    expect(freshButton).toBeDefined();

    await act(async () => {
      freshButton!.click();
      await flushMicrotasks();
      vi.runOnlyPendingTimers();
    });

    expect(mocks.focus).not.toHaveBeenCalled();
  });
});
