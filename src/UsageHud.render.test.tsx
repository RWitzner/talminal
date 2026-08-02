/**
 * @vitest-environment happy-dom
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CardInfo, TerminalCardInfo } from "./types";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

import {
  UsageHud,
  deriveUsageView,
  formatReset,
  type UsageSnapshot,
} from "./UsageHud";

const NOW = Date.parse("2026-07-21T19:00:00.000Z");

function snapshot(overrides: Partial<UsageSnapshot> = {}): UsageSnapshot {
  return {
    version: 1,
    writtenAt: new Date(NOW - 60_000).toISOString(),
    fiveHourPercent: 5,
    fiveHourResetsAt: new Date(NOW + 3 * 3_600_000 + 20 * 60_000).toISOString(),
    weeklyPercent: 1,
    weeklyResetsAt: new Date(NOW + 6 * 86_400_000 + 13 * 3_600_000).toISOString(),
    sessionId: null,
    ...overrides,
  };
}

function terminalCard(overrides: Partial<TerminalCardInfo> = {}): CardInfo {
  return {
    kind: "terminal",
    number: 1,
    name: "kort-1",
    cwd: "C:\\p",
    profile: "default",
    running: true,
    exited: null,
    restore_action: null,
    opened_by: null,
    url: null,
    title: null,
    ...overrides,
  } as CardInfo;
}

function browserCard(): CardInfo {
  return {
    kind: "browser",
    number: 2,
    name: "web",
    cwd: "",
    profile: "",
    running: true,
    exited: null,
    restore_action: null,
    opened_by: null,
    url: "https://example.com",
    title: "Web",
  } as CardInfo;
}

describe("deriveUsageView", () => {
  it("frisk snapshot → to bjælker med nedtælling", () => {
    expect(deriveUsageView(snapshot(), NOW)).toEqual({
      kind: "data",
      stale: false,
      bars: [
        { label: "5t", percent: 5, resetLabel: "3t20m" },
        { label: "uge", percent: 1, resetLabel: "6d13t" },
      ],
    });
  });

  it("15-60 min gammel → stale", () => {
    const view = deriveUsageView(
      snapshot({ writtenAt: new Date(NOW - 20 * 60_000).toISOString() }),
      NOW,
    );
    expect(view.kind).toBe("data");
    expect((view as { stale: boolean }).stale).toBe(true);
  });

  it(">60 min gammel eller null → empty", () => {
    expect(
      deriveUsageView(
        snapshot({ writtenAt: new Date(NOW - 61 * 60_000).toISOString() }),
        NOW,
      ),
    ).toEqual({ kind: "empty" });
    expect(deriveUsageView(null, NOW)).toEqual({ kind: "empty" });
  });

  it("manglende weekly → kun 5t-bjælken", () => {
    const view = deriveUsageView(
      snapshot({ weeklyPercent: null, weeklyResetsAt: null }),
      NOW,
    );
    expect(view.kind).toBe("data");
    expect((view as { bars: unknown[] }).bars).toHaveLength(1);
  });
});

describe("formatReset", () => {
  it("timer+minutter, dage+timer, passeret og null", () => {
    expect(formatReset(new Date(NOW + 3 * 3_600_000 + 20 * 60_000).toISOString(), NOW)).toBe("3t20m");
    expect(formatReset(new Date(NOW + 6 * 86_400_000 + 13 * 3_600_000).toISOString(), NOW)).toBe("6d13t");
    expect(formatReset(new Date(NOW - 1000).toISOString(), NOW)).toBeNull();
    expect(formatReset(null, NOW)).toBeNull();
  });

  it("under én time → rene minutter uden 0t-præfiks", () => {
    expect(formatReset(new Date(NOW + 42 * 60_000).toISOString(), NOW)).toBe("42m");
  });
});

describe("UsageHud — synlighed og polling", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    // Repo-præcedens i ALLE DOM-render-tests (review-fund 4): uden flaget
    // støjer React 19's act() med console.error i hver test.
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    vi.useFakeTimers({ now: NOW });
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue(snapshot());
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    vi.useRealTimers();
  });

  async function render(cards: CardInfo[] | null) {
    await act(async () => {
      root.render(<UsageHud cards={cards} />);
    });
  }

  it("skjult (opacity 0) uden kørende terminal-kort — og poller ikke", async () => {
    await render([browserCard(), terminalCard({ running: false, exited: 0 })]);
    const hud = host.querySelector("[data-usage-hud]") as HTMLElement;
    expect(hud.style.opacity).toBe("0");
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("synlig med kørende terminal-kort og viser begge bjælker", async () => {
    await render([terminalCard()]);
    const hud = host.querySelector("[data-usage-hud]") as HTMLElement;
    expect(hud.style.opacity).toBe("1");
    expect(mocks.invoke).toHaveBeenCalledWith("read_usage_snapshot");
    expect(hud.textContent).toContain("5%");
    expect(hud.textContent).toContain("uge");
    expect(hud.textContent).toContain("3t20m");
  });

  it("null-snapshot → pladsholder '—'", async () => {
    mocks.invoke.mockResolvedValue(null);
    await render([terminalCard()]);
    expect(host.querySelector("[data-usage-hud]")!.textContent).toContain("—");
  });

  it("poller igen efter intervallet", async () => {
    await render([terminalCard()]);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });

  it("poll stopper når synligheden slår fra", async () => {
    await render([terminalCard()]);
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    await render([terminalCard({ running: false, exited: 0 })]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3 * 30_000);
    });
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
  });
});
