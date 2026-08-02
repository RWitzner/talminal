/**
 * @vitest-environment happy-dom
 */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatCard } from "./ChatCard";
import type { ChatCardInfo } from "./types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => undefined) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

// Wiren er snake_case, og CardInfoBase kraever exited + restore_action.
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
    { seq: 1, from_card: "card-1", from_kind: "agent", intent: "sparring", text: "Jeg foreslaar", ts_ms: 1 },
    { seq: 2, from_card: "owner", from_kind: "human", intent: "sparring", text: "Hold jer til v1", ts_ms: 2 },
    { seq: 3, from_card: "system", from_kind: "system", intent: "status", text: "Du stoppede samarbejdet.", ts_ms: 3 },
  ],
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("ChatCard", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === "chat_thread_read" ? openThread : undefined,
    );
    vi.mocked(listen).mockClear();
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

  async function render(card: ChatCardInfo = baseCard): Promise<void> {
    await act(async () => {
      root.render(<ChatCard card={card} fullscreen={false} onToggleFullscreen={() => {}} />);
      await Promise.resolve();
    });
  }

  it("labeller afsender og art i TEKST, ikke kun med farve", async () => {
    await render();
    const senders = Array.from(host.querySelectorAll("[data-chat-sender]")).map(
      (el) => el.textContent,
    );
    expect(senders[0]).toContain("Kort 1");
    expect(senders[0]).toContain("agent");
    expect(senders[1]).toContain("Dig");
    expect(senders[2]).toContain("System");
  });

  it("viser hop-badgen", async () => {
    await render();
    expect(host.querySelector("[data-chat-hops]")?.textContent).toBe("4 / 20");
  });

  it("renderer systembeskeder adskilt fra agentbeskeder", async () => {
    await render();
    const kinds = Array.from(host.querySelectorAll("[data-message-kind]")).map((el) =>
      el.getAttribute("data-message-kind"),
    );
    expect(kinds).toEqual(["agent", "human", "system"]);
  });

  it("bruger en semantisk log frem for en div-suppe", async () => {
    await render();
    expect(host.querySelector("ol[role='log']")).toBeTruthy();
  });

  it("poster ejerens besked og rydder kladden ved succes", async () => {
    await render();
    const input = host.querySelector<HTMLTextAreaElement>("[data-chat-input]")!;
    const setter = Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )?.set;
    await act(async () => {
      setter?.call(input, "min replik");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      input.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
      await Promise.resolve();
    });
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("chat_thread_post", {
      thread: "t7",
      text: "min replik",
    });
    expect(input.value).toBe("");
  });

  it("bevarer kladden hvis backenden afviser", async () => {
    await render();
    vi.mocked(invoke).mockImplementation(async (cmd: string) => {
      if (cmd === "chat_thread_post") throw new Error("thread is closed");
      return openThread;
    });
    const input = host.querySelector<HTMLTextAreaElement>("[data-chat-input]")!;
    const setter = Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )?.set;
    await act(async () => {
      setter?.call(input, "vigtig replik");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      await Promise.resolve();
    });
    expect(host.querySelector<HTMLTextAreaElement>("[data-chat-input]")?.value).toBe(
      "vigtig replik",
    );
  });

  it("Shift+Enter og en igangvaerende IME-komposition submitter ikke", async () => {
    await render();
    const input = host.querySelector<HTMLTextAreaElement>("[data-chat-input]")!;
    const setter = Object.getOwnPropertyDescriptor(
      HTMLTextAreaElement.prototype,
      "value",
    )?.set;
    await act(async () => {
      setter?.call(input, "halv saetning");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      input.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true }),
      );
      const composing = new KeyboardEvent("keydown", { key: "Enter", bubbles: true });
      Object.defineProperty(composing, "isComposing", { value: true });
      input.dispatchEvent(composing);
      await Promise.resolve();
    });
    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith(
      "chat_thread_post",
      expect.anything(),
    );
  });

  it("en closed traad har intet inputfelt og ingen stop-knap", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === "chat_thread_read"
        ? { ...openThread, state: "closed", hops_used: 20, hops_left: 0 }
        : undefined,
    );
    await render();
    expect(host.querySelector("[data-chat-input]")).toBeNull();
    expect(host.querySelector("[data-chat-stop]")).toBeNull();
  });

  it("abonnerer paa chat-thread-updated og genlaeser traaden", async () => {
    await render();
    expect(vi.mocked(listen)).toHaveBeenCalledWith(
      "chat-thread-updated",
      expect.any(Function),
    );
    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { thread: string };
    }) => void;
    const before = vi.mocked(invoke).mock.calls.length;
    await act(async () => {
      handler({ payload: { thread: "t7" } });
      await Promise.resolve();
    });
    expect(vi.mocked(invoke).mock.calls.length).toBeGreaterThan(before);
    expect(vi.mocked(invoke)).toHaveBeenLastCalledWith("chat_thread_read", {
      thread: "t7",
      fromSeq: 3,
    });
  });

  it("appender nye beskeder fra et inkrementelt refresh", async () => {
    await render();
    vi.mocked(invoke).mockResolvedValueOnce({
      ...openThread,
      hops_used: 5,
      hops_left: 15,
      messages: [
        {
          seq: 4,
          from_card: "card-2",
          from_kind: "agent",
          intent: "answer",
          text: "Nyt svar",
          ts_ms: 4,
        },
      ],
    });
    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { thread: string };
    }) => void;
    await act(async () => {
      handler({ payload: { thread: "t7" } });
      await Promise.resolve();
    });

    expect(Array.from(host.querySelectorAll("[data-message-kind]"))).toHaveLength(4);
    expect(host.textContent).toContain("Jeg foreslaar");
    expect(host.textContent).toContain("Nyt svar");
  });

  it("deduper beskeder naar overlappende refresh-svar ankommer omvendt", async () => {
    await render();
    const sent = deferred<typeof openThread>();
    const newest = deferred<typeof openThread>();
    vi.mocked(invoke)
      .mockImplementationOnce(() => sent.promise)
      .mockImplementationOnce(() => newest.promise);
    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { thread: string };
    }) => void;
    act(() => {
      handler({ payload: { thread: "t7" } });
      handler({ payload: { thread: "t7" } });
    });

    await act(async () => {
      newest.resolve({
        ...openThread,
        messages: [
          {
            seq: 4,
            from_card: "card-2",
            from_kind: "agent",
            intent: "answer",
            text: "samme",
            ts_ms: 4,
          },
        ],
      });
      await newest.promise;
      sent.resolve({
        ...openThread,
        messages: [
          {
            seq: 4,
            from_card: "card-2",
            from_kind: "agent",
            intent: "answer",
            text: "samme",
            ts_ms: 4,
          },
        ],
      });
      await sent.promise;
    });

    expect(host.querySelectorAll("[data-message-kind]")).toHaveLength(4);
    expect((host.textContent ?? "").split("samme")).toHaveLength(2);
  });

  it("lader ikke et sent refresh rulle headeren tilbage", async () => {
    await render();
    const sent = deferred<typeof openThread>();
    const newest = deferred<typeof openThread>();
    vi.mocked(invoke)
      .mockImplementationOnce(() => sent.promise)
      .mockImplementationOnce(() => newest.promise);
    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { thread: string };
    }) => void;
    act(() => {
      handler({ payload: { thread: "t7" } });
      handler({ payload: { thread: "t7" } });
    });

    await act(async () => {
      newest.resolve({
        ...openThread,
        state: "closed",
        hops_used: 20,
        hops_left: 0,
        messages: [],
      });
      await newest.promise;
      sent.resolve({
        ...openThread,
        state: "open",
        hops_used: 4,
        hops_left: 16,
        messages: [],
      });
      await sent.promise;
    });

    expect(host.querySelector("[data-chat-card]")?.getAttribute("data-chat-state")).toBe(
      "closed",
    );
    expect(host.querySelector("[data-chat-hops]")?.textContent).toBe("20 / 20");
    expect(host.querySelector("[data-chat-input]")).toBeNull();
  });

  it("nulstiller ved traad-skift og kasserer et in-flight svar fra den gamle", async () => {
    const oldRead = deferred<typeof openThread>();
    const newRead = deferred<typeof openThread>();
    vi.mocked(invoke).mockImplementation((_cmd, args) => {
      const thread =
        args !== null && typeof args === "object" && "thread" in args
          ? args.thread
          : undefined;
      return thread === "t7" ? oldRead.promise : newRead.promise;
    });

    await act(async () => {
      root.render(<ChatCard card={baseCard} fullscreen={false} onToggleFullscreen={() => {}} />);
      await Promise.resolve();
    });
    const nextCard = { ...baseCard, thread_id: "t8", purpose: "ny traad" };
    await act(async () => {
      root.render(<ChatCard card={nextCard} fullscreen={false} onToggleFullscreen={() => {}} />);
      await Promise.resolve();
    });
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("chat_thread_read", {
      thread: "t8",
      fromSeq: 0,
    });

    await act(async () => {
      newRead.resolve({
        ...openThread,
        purpose: "ny traad",
        messages: [
          {
            seq: 1,
            from_card: "card-8",
            from_kind: "agent",
            intent: "sparring",
            text: "kun ny",
            ts_ms: 8,
          },
        ],
      });
      await newRead.promise;
      oldRead.resolve(openThread);
      await oldRead.promise;
    });

    expect(host.textContent).toContain("kun ny");
    expect(host.textContent).not.toContain("Jeg foreslaar");
  });

  it("giver de to agenter HVER sin accent, og lader ejeren staa uden for skalaen", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === "chat_thread_read"
        ? {
            ...openThread,
            messages: [
              ...openThread.messages.slice(0, 2),
              { seq: 4, from_card: "card-2", from_kind: "agent", intent: "answer", text: "Modspil", ts_ms: 4 },
            ],
          }
        : undefined,
    );
    await render();
    const agents = Array.from(
      host.querySelectorAll<HTMLElement>('[data-message-kind="agent"]'),
    );
    expect(agents).toHaveLength(2);
    const [first, second] = agents.map((li) => li.style.borderLeftColor);
    expect(first).not.toBe("");
    // To agenter maa ALDRIG faa samme farve — hele pointen med ryggen falder.
    expect(first).not.toBe(second);

    const owner = host.querySelector<HTMLElement>('[data-message-kind="human"]');
    expect(owner!.style.borderLeftStyle).toBe("dashed");
    expect(owner!.style.marginLeft).not.toBe("");
    expect(owner!.style.borderLeftColor).not.toBe(first);
    expect(owner!.style.borderLeftColor).not.toBe(second);
  });

  it("viser intent PAA agentbeskeder og ikke paa ejerens eller systemets", async () => {
    await render();
    const intentsOf = (kind: string) =>
      host.querySelector<HTMLElement>(`[data-message-kind="${kind}"]`)?.textContent ?? "";
    expect(intentsOf("agent")).toContain("sparring");
    // "Dig (menneske)" og "System" fortaeller allerede alt — wire-ordet
    // ville kun vaere stoej.
    expect(intentsOf("human")).not.toContain("sparring");
    expect(intentsOf("system")).not.toContain("status");
  });

  it("laeser hop-loftet fra traaden i stedet for at haardkode 20", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === "chat_thread_read"
        ? { ...openThread, hops_used: 7, hops_left: 5 }
        : undefined,
    );
    await render();
    expect(host.querySelector("[data-chat-hops]")?.textContent).toBe("7 / 12");
  });

  it("flytter IKKE loftet naar et answer passerer det", async () => {
    // Ved loftet taeller `post()` hop op ubetinget, ogsaa for det `answer` der
    // har lov at passere — saa hops_used bliver 21 mens hops_left er 0.
    // Udledningen used+left ville da vise "21 / 21", som om intet saerligt
    // skete. Det er praecis den ene aflaesning runbookens §2.5 haenger paa.
    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === "chat_thread_read"
        ? { ...openThread, hops_used: 19, hops_left: 1 }
        : undefined,
    );
    await render();
    expect(host.querySelector("[data-chat-hops]")?.textContent).toBe("19 / 20");

    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === "chat_thread_read"
        ? { ...openThread, hops_used: 21, hops_left: 0 }
        : undefined,
    );
    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { thread: string };
    }) => void;
    await act(async () => {
      handler({ payload: { thread: "t7" } });
      await Promise.resolve();
    });

    const hops = host.querySelector<HTMLElement>("[data-chat-hops]");
    expect(hops?.textContent).toBe("21 / 20");
    expect(hops?.dataset.chatHopTone).toBe("spent");
  });

  it("advarer paa hop-maaleren naar der er faa tilbage", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === "chat_thread_read"
        ? { ...openThread, hops_used: 18, hops_left: 2 }
        : undefined,
    );
    await render();
    expect(
      host.querySelector<HTMLElement>("[data-chat-hops]")?.dataset.chatHopTone,
    ).toBe("warn");
  });

  it("markerer awaiting i headeren", async () => {
    await render();
    expect(host.querySelector("[data-chat-awaiting]")).toBeNull();

    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === "chat_thread_read" ? { ...openThread, state: "awaiting" } : undefined,
    );
    await act(async () => root.unmount());
    root = createRoot(host);
    await render();
    expect(host.querySelector("[data-chat-awaiting]")).not.toBeNull();
  });

  it("lader den terminale aarsag staa ÉT sted i en lukket traad", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === "chat_thread_read"
        ? {
            ...openThread,
            state: "closed",
            messages: [
              ...openThread.messages,
              {
                seq: 4,
                from_card: "system",
                from_kind: "system",
                intent: "status",
                text: "Traaden naaede loftet paa 20 beskeder.",
                ts_ms: 4,
              },
            ],
          }
        : undefined,
    );
    await render();
    expect(host.querySelector("[data-chat-closed]")).not.toBeNull();
    expect(host.querySelector("[data-chat-input]")).toBeNull();
    // Aarsagen staar som traadens sidste systembesked, og loggen ruller til
    // bunds — en fodnote under loggen ville gentage den ordret.
    const systemRows = Array.from(
      host.querySelectorAll('[data-message-kind="system"]'),
    );
    expect(systemRows.at(-1)?.textContent).toContain(
      "Traaden naaede loftet paa 20 beskeder.",
    );
    const occurrences = (host.textContent ?? "").split(
      "Traaden naaede loftet paa 20 beskeder.",
    ).length - 1;
    expect(occurrences).toBe(1);
  });

  it("viser en kodeblok som pre og lader indholdet staa ordret", async () => {
    const code = 'fn main() {\n\tlet s = "quotes";\n\n}';
    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === "chat_thread_read"
        ? {
            ...openThread,
            messages: [
              {
                seq: 1,
                from_card: "card-1",
                from_kind: "agent",
                intent: "answer",
                text: `Her:\n\`\`\`rust\n${code}\n\`\`\``,
                ts_ms: 1,
              },
            ],
          }
        : undefined,
    );
    await render();
    const pre = host.querySelector<HTMLElement>("[data-chat-code]");
    expect(pre?.tagName).toBe("PRE");
    // Runbook §2.8: tabulator, citationstegn og den tomme linje skal overleve.
    expect(pre?.textContent).toBe(code);
  });

  it("inviterer i stedet for at vise et tomt hul", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd: string) =>
      cmd === "chat_thread_read" ? { ...openThread, messages: [] } : undefined,
    );
    await render();
    expect(host.querySelector("[data-chat-empty]")?.textContent).toContain(
      "første besked",
    );
  });

  it("ignorerer events for andre traade", async () => {
    await render();
    const handler = vi.mocked(listen).mock.calls[0][1] as (e: {
      payload: { thread: string };
    }) => void;
    const before = vi.mocked(invoke).mock.calls.length;
    await act(async () => {
      handler({ payload: { thread: "t99" } });
      await Promise.resolve();
    });
    expect(vi.mocked(invoke).mock.calls.length).toBe(before);
  });

  it("melder sig ud af canvas-rodens user-select: none", async () => {
    await act(async () => {
      root.render(
        <ChatCard card={baseCard} fullscreen={false} onToggleFullscreen={() => undefined} />,
      );
    });

    const style = host.querySelector("[data-chat-card] style");
    expect(style).not.toBeNull();
    const css = style!.textContent ?? "";
    expect(css).toContain("[data-chat-card] { user-select: text; }");
    expect(css).toContain("[data-chat-card] button { user-select: none; }");
  });
});
