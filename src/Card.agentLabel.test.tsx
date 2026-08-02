/**
 * @vitest-environment happy-dom
 */

// Agent-label i kort-headeren (Task 7): profil-slug'et skal kunne aflaeses
// paa BEGGE agenter (claude/codex) — blandede workspaces skal vaere
// laesbare uden at aabne et kort. Samme render-rig som Card.resize.test.tsx
// (ingen @testing-library/react i repoet — DOM'en inspiceres direkte).

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Card } from "./Card";
import type { CardState, TerminalCardInfo } from "./types";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

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

function cardWithProfile(
  profile: string,
  overrides: Partial<TerminalCardInfo> = {},
): TerminalCardInfo {
  return {
    kind: "terminal",
    number: 1,
    name: "card-1",
    cwd: "C:\\project",
    profile,
    running: true,
    exited: null,
    restore_action: null,
    opened_by: null,
    url: null,
    title: null,
    ...overrides,
  };
}

const runningState: CardState = {
  running: true,
  exited: null,
  owner: "persona",
  epoch: 0,
};

const notStartedState: CardState = {
  running: false,
  exited: null,
  owner: "persona",
  epoch: 0,
};

/** Hydreringens get_card_state-svar — saettes pr. test foer render. */
let cardState: CardState = runningState;

async function flushMicrotasks(): Promise<void> {
  for (let index = 0; index < 6; index += 1) {
    await Promise.resolve();
  }
}

describe("Card agent-label", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    cardState = runningState;
    mocks.invoke.mockReset().mockImplementation(async (command: string) => {
      if (command === "get_card_state") return cardState;
      return undefined;
    });
    mocks.listen.mockReset().mockImplementation(async () => () => {});
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  it("viser kortets profil-slug som agent-label i headeren (codex)", async () => {
    await act(async () => {
      root.render(<Card card={cardWithProfile("codex")} />);
      await flushMicrotasks();
    });
    expect(
      host.querySelector("[data-card-agent-label]")?.textContent,
    ).toBe("codex");
  });

  it("viser kortets profil-slug som agent-label i headeren (claude)", async () => {
    await act(async () => {
      root.render(<Card card={cardWithProfile("claude")} />);
      await flushMicrotasks();
    });
    expect(
      host.querySelector("[data-card-agent-label]")?.textContent,
    ).toBe("claude");
  });

  // Slut-review fix 4: context-snapshots joines KUN paa kortnavn+cwd
  // (contextHud.ts) — intet agent-felt, ingen friskhed. Kortnavne genbruges
  // (registry'ets laveste-ledige), saa et codex-kort i samme cwd ville arve
  // et gammelt Claude-snapshot og vise et frossent, fremmed CTX-tal.
  // Badgen er derfor gatet paa profilen, ikke kun paa "har data".
  it("viser CTX-badgen paa et koerende claude-kort", async () => {
    await act(async () => {
      root.render(<Card card={cardWithProfile("claude")} contextPercent={42} />);
      await flushMicrotasks();
    });
    expect(host.querySelector("[data-context-badge]")?.textContent).toBe(
      "CTX 42%",
    );
  });

  it("viser ALDRIG CTX-badgen paa et codex-kort — heller ikke med snapshot", async () => {
    await act(async () => {
      root.render(<Card card={cardWithProfile("codex")} contextPercent={42} />);
      await flushMicrotasks();
    });
    expect(host.querySelector("[data-context-badge]")).toBeNull();
  });

  // T7-review-efterslaeb: resumeLabel (Card.tsx) var utestet — den vises kun
  // i start-overlayet paa et IKKE-startet kort.
  it("start-overlayet viser codex' resume-tekst paa et ikke-startet kort", async () => {
    cardState = notStartedState;
    await act(async () => {
      root.render(
        <Card card={cardWithProfile("codex", { running: false })} />,
      );
      await flushMicrotasks();
    });
    expect(host.textContent).toContain("Kortet er ikke startet.");
    expect(host.textContent).toContain("Fortsæt (resume --last)");
  });

  it("start-overlayet viser claudes resume-tekst paa et ikke-startet kort", async () => {
    cardState = notStartedState;
    await act(async () => {
      root.render(
        <Card card={cardWithProfile("claude", { running: false })} />,
      );
      await flushMicrotasks();
    });
    expect(host.textContent).toContain("Fortsæt (--continue)");
  });
});
