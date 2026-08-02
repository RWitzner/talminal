/**
 * @vitest-environment happy-dom
 */

// Luk-knappen i terminalkortets header. Indtil nu var stemmen ("Luk kort N")
// den ENESTE luk-vej for et terminalkort — browser-kortet havde sit ✕, men
// terminalkortet havde ingenting. Det er ikke en lille mangel: en luk-vej man
// ikke kan SE, findes ikke for den der ikke allerede kender den. Runbookens
// §2.4 beder om "kortets normale luk-vej", og der var ingen at pege paa.
//
// Samme render-rig som Card.agentLabel.test.tsx (ingen @testing-library i
// repoet — DOM'en inspiceres direkte).

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Card } from "./Card";
import type { CardState, TerminalCardInfo } from "./types";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

vi.mock("@xterm/xterm", () => ({
  Terminal: class TerminalMock {
    cols = 80;
    rows = 24;
    loadAddon(): void {}
    open(): void {}
    onData(): { dispose(): void } {
      return { dispose() {} };
    }
    write(): void {}
    writeln(): void {}
    reset(): void {}
    focus(): void {}
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
    activate(): void {}
    fit(): void {}
  },
}));

const card: TerminalCardInfo = {
  kind: "terminal",
  number: 2,
  name: "card-2",
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
  for (let index = 0; index < 6; index += 1) await Promise.resolve();
}

describe("Card luk-knap", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.invoke.mockImplementation(async (command: string) =>
      command === "get_card_state" ? runningState : undefined,
    );
    mocks.listen.mockReset();
    mocks.listen.mockImplementation(async () => () => undefined);
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

  async function render(onClosed?: () => void): Promise<void> {
    await act(async () => {
      root.render(<Card card={card} onClosed={onClosed} />);
      await flushMicrotasks();
    });
  }

  it("har en synlig luk-knap i headeren", async () => {
    await render();
    const button = host.querySelector<HTMLButtonElement>("[data-card-close-action]");
    expect(button).not.toBeNull();
    expect(button?.getAttribute("aria-label")).toBe("Luk kort");
  });

  it("lukker kortet ved navn og melder tilbage til fladen", async () => {
    const onClosed = vi.fn();
    await render(onClosed);
    await act(async () => {
      host
        .querySelector<HTMLButtonElement>("[data-card-close-action]")!
        .dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await flushMicrotasks();
    });
    expect(mocks.invoke).toHaveBeenCalledWith("close_card", { name: "card-2" });
    // Uden tilbagemeldingen ville kortet blive staaende i gridden indtil en
    // urelateret refresh — samme klasse som `cards-changed`-fundet.
    expect(onClosed).toHaveBeenCalled();
  });

  it("VISER en afvist lukning i stedet for at sluge den", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_card_state") return runningState;
      if (command === "close_card") throw new Error("card is busy");
      return undefined;
    });
    const onClosed = vi.fn();
    await render(onClosed);
    await act(async () => {
      host
        .querySelector<HTMLButtonElement>("[data-card-close-action]")!
        .dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await flushMicrotasks();
    });
    const error = host.querySelector("[data-card-error]");
    expect(error?.textContent).toContain("card is busy");
    // Fejlede lukninger maa ikke fortaelle fladen at kortet er vaek.
    expect(onClosed).not.toHaveBeenCalled();
  });

  it("stopper klikket, saa det ikke ogsaa saetter type-mode paa kortet", async () => {
    // Spejler CanvasSurface: kortet ligger i en [data-card-body] med en
    // REACT-handler der saetter type-mode. Uden stopPropagation ville et klik
    // paa ✕ foerst fokusere det kort der er ved at forsvinde — og blive
    // haengende i type-mode hvis lukningen fejler.
    const enterTypeMode = vi.fn();
    await act(async () => {
      root.render(
        <div data-card-body onPointerDown={enterTypeMode}>
          <Card card={card} />
        </div>,
      );
      await flushMicrotasks();
    });
    await act(async () => {
      host
        .querySelector<HTMLButtonElement>("[data-card-close-action]")!
        .dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      await flushMicrotasks();
    });
    expect(enterTypeMode).not.toHaveBeenCalled();
  });
});
