import { describe, expect, it } from "vitest";
import {
  describeBlockedTarget,
  resolveDictationTarget,
  type DictationFocus,
} from "./dictationTarget";
import type { CardInfo } from "../types";

function terminal(number: number, running = true): CardInfo {
  return {
    kind: "terminal",
    number,
    name: `card-${number}`,
    cwd: "C:\\repo",
    profile: "claude",
    running,
    exited: running ? null : 0,
    restore_action: null,
    opened_by: null,
    url: null,
    title: null,
  };
}

function chat(number: number): CardInfo {
  return {
    kind: "chat",
    number,
    name: `card-${number}`,
    cwd: "",
    profile: "",
    running: true,
    exited: null,
    restore_action: null,
    opened_by: null,
    url: null,
    title: null,
    thread_id: `thread-${number}`,
    purpose: "sparring",
  };
}

function browser(number: number): CardInfo {
  return {
    kind: "browser",
    number,
    name: `card-${number}`,
    cwd: "",
    profile: "",
    running: true,
    exited: null,
    restore_action: null,
    opened_by: null,
    url: "https://example.com",
    title: "Example",
  };
}

function focus(partial: Partial<DictationFocus>): DictationFocus {
  return {
    chatInputThread: null,
    focusedCard: null,
    cards: [],
    ...partial,
  };
}

describe("resolveDictationTarget", () => {
  it("rammer det fokuserede terminal-kort", () => {
    expect(
      resolveDictationTarget(
        focus({ focusedCard: 2, cards: [terminal(1), terminal(2)] }),
      ),
    ).toEqual({ kind: "terminal", name: "card-2" });
  });

  it("lader chat-composerens DOM-fokus vinde over type-mode", () => {
    // Regressionsvaernet for hele modulets eksistens: `focused` peger paa et
    // terminalkort (det sidste man klikkede i), MENS markoeren staar i et
    // chat-korts composer. Uden praecedensen ville dikteringen lande i
    // card-1's terminal — et helt andet kort end det brugeren skriver i.
    expect(
      resolveDictationTarget(
        focus({
          chatInputThread: "thread-4",
          focusedCard: 1,
          cards: [terminal(1), chat(4)],
        }),
      ),
    ).toEqual({ kind: "chat", name: "card-4" });
  });

  it("falder tilbage til type-mode hvis chat-kortet forsvandt under turen", () => {
    expect(
      resolveDictationTarget(
        focus({
          chatInputThread: "thread-9",
          focusedCard: 1,
          cards: [terminal(1)],
        }),
      ),
    ).toEqual({ kind: "terminal", name: "card-1" });
  });

  it("afviser browser-kort", () => {
    expect(
      resolveDictationTarget(focus({ focusedCard: 3, cards: [browser(3)] })),
    ).toEqual({ kind: "none", reason: "unsupported_card" });
  });

  it("afviser et kort der ikke koerer — der er ingen pty at skrive i", () => {
    expect(
      resolveDictationTarget(
        focus({ focusedCard: 5, cards: [terminal(5, false)] }),
      ),
    ).toEqual({ kind: "none", reason: "not_running" });
  });

  it("melder no_focus uden fokus og for et ukendt kortnummer", () => {
    expect(resolveDictationTarget(focus({ cards: [terminal(1)] }))).toEqual({
      kind: "none",
      reason: "no_focus",
    });
    expect(
      resolveDictationTarget(focus({ focusedCard: 7, cards: [terminal(1)] })),
    ).toEqual({ kind: "none", reason: "no_focus" });
  });

  it("rammer et chat-kort ogsaa gennem type-mode-grenen", () => {
    // Grenen er utilgaengelig i dag, men skal opfoere sig rigtigt den dag
    // fokus-modellen omfatter chat-kort.
    expect(
      resolveDictationTarget(focus({ focusedCard: 6, cards: [chat(6)] })),
    ).toEqual({ kind: "chat", name: "card-6" });
  });
});

describe("describeBlockedTarget", () => {
  it("giver en brugervendt sætning for hver aarsag", () => {
    for (const reason of ["no_focus", "not_running", "unsupported_card"] as const) {
      expect(describeBlockedTarget(reason).length).toBeGreaterThan(0);
    }
  });
});
