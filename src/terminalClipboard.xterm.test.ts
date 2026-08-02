/**
 * @vitest-environment happy-dom
 */

import { describe, expect, it } from "vitest";
import { Terminal } from "@xterm/xterm";
import { attachTerminalClipboard } from "./terminalClipboard";

function makeTerm(): { term: Terminal; received: string[] } {
  const host = document.createElement("div");
  document.body.append(host);
  const term = new Terminal({ scrollback: 2000 });
  term.open(host);
  const received: string[] = [];
  term.onData((data) => received.push(data));
  return { term, received };
}

function press(
  term: Terminal,
  key: string,
  keyCode: number,
  init: KeyboardEventInit = {},
): void {
  const textarea = (term as unknown as { textarea: HTMLTextAreaElement }).textarea;
  const ev = new KeyboardEvent("keydown", {
    bubbles: true,
    cancelable: true,
    key,
    ...init,
  });
  Object.defineProperty(ev, "keyCode", { get: () => keyCode });
  Object.defineProperty(ev, "which", { get: () => keyCode });
  textarea.dispatchEvent(ev);
}

function deadThenCopyThenEnter(term: Terminal): void {
  press(term, "Dead", 221);
  press(term, "Control", 17, { ctrlKey: true });
  press(term, "c", 67, { ctrlKey: true });
  press(term, "Enter", 13);
}

describe("doedtast-vaernet mod aegte xterm", () => {
  it("KONTROLCASE: uden handler naar Enter frem", () => {
    const { term, received } = makeTerm();
    deadThenCopyThenEnter(term);
    expect(received).toEqual(["\r"]);
  });

  it("med vaernet naar Enter stadig frem", () => {
    const { term, received } = makeTerm();
    attachTerminalClipboard(term, async () => true);
    deadThenCopyThenEnter(term);
    expect(received).toEqual(["\r"]);
  });

  it("uden forudgaaende doedtast sluges Ctrl+C som den skal", async () => {
    const { term, received } = makeTerm();
    let copied = "";
    attachTerminalClipboard(term, async (text) => {
      copied = text;
      return true;
    });
    await new Promise<void>((resolve) => term.write("noget output", resolve));
    term.selectAll();

    press(term, "Control", 17, { ctrlKey: true });
    press(term, "c", 67, { ctrlKey: true });

    expect(received).toEqual([]);
    expect(copied).not.toBe("");
  });

  it("Enter alene naar frem baade med og uden vaernet", () => {
    const bare = makeTerm();
    press(bare.term, "Enter", 13);
    expect(bare.received).toEqual(["\r"]);

    const guarded = makeTerm();
    attachTerminalClipboard(guarded.term, async () => true);
    press(guarded.term, "Enter", 13);
    expect(guarded.received).toEqual(["\r"]);
  });
});
