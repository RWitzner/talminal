/**
 * @vitest-environment happy-dom
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CardInfo, TerminalCardInfo } from "./types";
import type { ContextSnapshot } from "./contextHud";
import type { CardProps } from "./Card";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  cardProps: [] as Array<{ name: string; contextPercent: number | null | undefined }>,
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

// Kort-dublet: registrerer modtagne props — badgen selv testes i
// ContextBadge.test.tsx, her testes KUN poll + join-tildeling.
vi.mock("./Card", () => ({
  PREPARE_FRESH_SPAWN_EVENT: "talminal:prepare-fresh-spawn",
  Card: (props: CardProps) => {
    mocks.cardProps.push({
      name: props.card.name,
      contextPercent: props.contextPercent,
    });
    return <div data-card-double={props.card.name} />;
  },
}));

import { CanvasSurface } from "./CanvasSurface";

function terminalCard(overrides: Partial<TerminalCardInfo> = {}): CardInfo {
  return {
    kind: "terminal",
    number: 1,
    name: "kort-1",
    cwd: "C:\\proj",
    profile: "claude",
    running: true,
    exited: null,
    restore_action: null,
    opened_by: null,
    url: null,
    title: null,
    ...overrides,
  } as CardInfo;
}

function snapshot(overrides: Partial<ContextSnapshot> = {}): ContextSnapshot {
  return {
    version: 1,
    writtenAt: "2026-07-22T10:00:00.000Z",
    cardName: "kort-1",
    runId: null,
    sessionId: null,
    cwd: "C:\\proj",
    usedPercent: 18,
    windowSize: null,
    modelDisplayName: null,
    ...overrides,
  };
}

describe("CanvasSurface — context-poll og join", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    vi.useFakeTimers();
    mocks.invoke.mockReset();
    mocks.cardProps.length = 0;
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    vi.useRealTimers();
  });

  async function render(cards: CardInfo[]) {
    await act(async () => {
      root.render(<CanvasSurface cards={cards} />);
    });
  }

  function contextCalls(): number {
    return mocks.invoke.mock.calls.filter(
      ([command]) => command === "read_context_snapshots",
    ).length;
  }

  it("én samlet poll for flere kort — og procenter joines pr. kort", async () => {
    mocks.invoke.mockImplementation(async (command: string) =>
      command === "read_context_snapshots"
        ? [snapshot(), snapshot({ cardName: "kort-2", usedPercent: 55 })]
        : undefined,
    );
    await render([
      terminalCard(),
      terminalCard({ number: 2, name: "kort-2" }),
      terminalCard({ number: 3, name: "kort-3" }),
    ]);
    expect(contextCalls()).toBe(1);
    const latest = new Map(
      mocks.cardProps.map((p) => [p.name, p.contextPercent]),
    );
    expect(latest.get("kort-1")).toBe(18);
    expect(latest.get("kort-2")).toBe(55);
    expect(latest.get("kort-3")).toBeNull();
  });

  it("poller igen efter intervallet", async () => {
    mocks.invoke.mockResolvedValue([]);
    await render([terminalCard()]);
    expect(contextCalls()).toBe(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(contextCalls()).toBe(2);
  });

  it("poller ikke uden kørende terminal-kort", async () => {
    mocks.invoke.mockResolvedValue([]);
    await render([terminalCard({ running: false, exited: 0 })]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3 * 30_000);
    });
    expect(contextCalls()).toBe(0);
  });

  it("cwd-mismatch giver null (navnekollisions-værnet)", async () => {
    mocks.invoke.mockImplementation(async (command: string) =>
      command === "read_context_snapshots"
        ? [snapshot({ cwd: "C:\\andet-projekt" })]
        : undefined,
    );
    await render([terminalCard()]);
    const latest = new Map(
      mocks.cardProps.map((p) => [p.name, p.contextPercent]),
    );
    expect(latest.get("kort-1")).toBeNull();
  });
});
