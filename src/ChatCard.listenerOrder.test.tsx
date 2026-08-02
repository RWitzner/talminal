/**
 * @vitest-environment happy-dom
 */

// M24: chat-kortet laeste traaden ved mount og registrerede FOERST derefter
// `chat-thread-updated`. `listen()` er en asynkron round-trip til Rust, saa der
// laa et reelt vindue imellem — og tabet i det vindue er varigt.
//
// Emitteren er nemlig en LEVEL-baseret aendringsdetektor, ikke en retry:
// `heartbeat::beat` sammenligner beskedantallet med `tracker.seen` og gemmer det
// nye tal med det samme, saa den samme tilstand emittes aldrig igen. Et event
// der falder i registreringsvinduet kommer ALDRIG tilbage, og kortet ville staa
// med sit mount-snapshot til naeste gang traaden voksede. Ejeren saa kun sine
// EGNE replikker — og hele §5.4's samtykke-argument haenger paa at udvekslingen
// ER synlig.
//
// Grebet er porten: den mockede `listen` resolver foerst naar testen aabner den,
// og imens ER vi i registreringsvinduet.

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  handlers: new Map<string, (event: { payload: { thread: string } }) => void>(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

import { ChatCard } from "./ChatCard";
import type { ChatCardInfo } from "./types";

const baseCard: ChatCardInfo = {
  kind: "chat",
  number: 3,
  name: "card-3",
  cwd: "",
  profile: "",
  running: true,
  exited: null,
  restore_action: null,
  opened_by: null,
  url: null,
  title: null,
  thread_id: "t7",
  purpose: "spar om submit",
};

const openThread = {
  state: "open",
  purpose: "spar om submit",
  hops_used: 4,
  hops_left: 16,
  messages: [
    {
      seq: 1,
      from_card: "card-1",
      from_kind: "agent",
      intent: "sparring",
      text: "Jeg foreslaar",
      ts_ms: 1,
    },
  ],
};

describe("ChatCard — foerste laesning ligger efter registreringen (M24)", () => {
  let host: HTMLDivElement;
  let root: Root;
  let aabnPorten: () => void;

  beforeEach(() => {
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    mocks.handlers.clear();
    const port = new Promise<void>((open) => {
      aabnPorten = open;
    });
    mocks.listen.mockReset().mockImplementation(
      async (
        event: string,
        handler: (event: { payload: { thread: string } }) => void,
      ) => {
        await port;
        mocks.handlers.set(event, handler);
        return () => mocks.handlers.delete(event);
      },
    );
    mocks.invoke.mockReset().mockImplementation(async (cmd: string) =>
      cmd === "chat_thread_read" ? openThread : undefined,
    );
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    aabnPorten();
    await act(async () => root.unmount());
    host.remove();
  });

  function render(): Promise<void> {
    return act(async () => {
      root.render(
        <ChatCard card={baseCard} fullscreen={false} onToggleFullscreen={() => {}} />,
      );
    });
  }

  // Makrotask-graense: den draener alle ventende mikrotasks, og kaeden
  // port -> listen -> refresh -> chat_thread_read er flere led lang.
  function tik(): Promise<void> {
    return act(async () => {
      await new Promise<void>((done) => setTimeout(done, 0));
    });
  }

  function laesekald(): number {
    return mocks.invoke.mock.calls.filter(
      (call) => call[0] === "chat_thread_read",
    ).length;
  }

  it("laeser ikke traaden mens registreringen stadig er i luften", async () => {
    await render();
    await tik();

    expect(mocks.handlers.size).toBe(0);
    expect(laesekald()).toBe(0);
  });

  it("laeser traaden naar registreringen er gennemfoert, saa et tabt event ikke koster friskhed", async () => {
    await render();
    await tik();
    // Backenden emitter i vinduet: ingen lytter, og `heartbeat::beat` gentager
    // det aldrig. Uden en laesning bagefter stod kortet tomt for evigt.
    expect(mocks.handlers.get("chat-thread-updated")).toBeUndefined();
    const foer = laesekald();

    aabnPorten();
    await tik();

    expect(mocks.handlers.has("chat-thread-updated")).toBe(true);
    expect(laesekald()).toBeGreaterThan(foer);
    // Frisk DATA, ikke bare et kald: beskeden staar i loggen.
    expect(host.textContent).toContain("Jeg foreslaar");
  });

  // Bagsiden af den bindende raekkefoelge: foerste laesning haenger nu paa at
  // registreringen lykkes, saa et afvist `listen()` efterlader kortet HELT tomt
  // — hvor det foer naaede at vise sit mount-snapshot. Uden en `.catch` var det
  // oveni tavst: rejection'en forsvandt som en uhaandteret promise.
  it("et afvist listen-kald logges i stedet for at forsvinde", async () => {
    const konsol = vi.spyOn(console, "error").mockImplementation(() => {});
    mocks.listen.mockReset().mockImplementation(async () => {
      throw new Error("plugin:event|listen afvist");
    });
    try {
      await render();
      await tik();

      expect(laesekald()).toBe(0);
      expect(konsol).toHaveBeenCalledWith(
        `chat-thread-updated listener(${baseCard.thread_id}) fejlede:`,
        expect.any(Error),
      );
    } finally {
      konsol.mockRestore();
    }
  });

  it("laeser ikke hvis kortet forsvinder mens registreringen er i luften", async () => {
    await render();
    await act(async () => {
      root.render(<div />);
    });

    aabnPorten();
    await tik();

    expect(laesekald()).toBe(0);
  });
});
