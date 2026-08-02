import { afterEach, describe, expect, it, vi } from "vitest";
import {
  CAST_BURST_INITIAL,
  CAST_BURST_RESET_MS,
  CAST_STAGGER_MS,
  emitCast,
  nextCastDelay,
  subscribeCast,
  type CastEvent,
} from "./cast";

describe("cast-bus", () => {
  const unsubscribers: Array<() => void> = [];
  afterEach(() => {
    while (unsubscribers.length) unsubscribers.pop()?.();
  });

  function listen(): CastEvent[] {
    const events: CastEvent[] = [];
    unsubscribers.push(subscribeCast((event) => events.push(event)));
    return events;
  }

  it("leverer events til abonnenter og stopper efter unsubscribe", () => {
    const events: CastEvent[] = [];
    const unsubscribe = subscribeCast((event) => events.push(event));
    emitCast({ card: 3, landing: "prompt" });
    unsubscribe();
    emitCast({ card: 4, landing: "card" });
    expect(events).toEqual([{ card: 3, landing: "prompt" }]);
  });

  it("en abonnent der kaster vaelter hverken emit eller andre abonnenter", () => {
    const boom = vi.fn(() => {
      throw new Error("pynt-fejl");
    });
    unsubscribers.push(subscribeCast(boom));
    const events = listen();
    expect(() => emitCast({ card: 1, landing: "card" })).not.toThrow();
    expect(boom).toHaveBeenCalledOnce();
    expect(events).toEqual([{ card: 1, landing: "card" }]);
  });

  it("emit uden abonnenter er en no-op", () => {
    expect(() => emitCast({ card: 7, landing: "card" })).not.toThrow();
  });
});

describe("nextCastDelay", () => {
  it("foerste cast i et burst har ingen forsinkelse", () => {
    const { delay } = nextCastDelay(CAST_BURST_INITIAL, 1_000);
    expect(delay).toBe(0);
  });

  it("efterfoelgende casts i samme burst forskydes et trin ad gangen", () => {
    const first = nextCastDelay(CAST_BURST_INITIAL, 1_000);
    const second = nextCastDelay(first.state, 1_000);
    const third = nextCastDelay(second.state, 1_010);
    expect(second.delay).toBe(CAST_STAGGER_MS);
    expect(third.delay).toBe(2 * CAST_STAGGER_MS);
  });

  it("bursten nulstilles efter stilhed", () => {
    const first = nextCastDelay(CAST_BURST_INITIAL, 1_000);
    const second = nextCastDelay(
      first.state,
      1_000 + CAST_BURST_RESET_MS + 1,
    );
    expect(second.delay).toBe(0);
  });
});
