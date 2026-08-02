/**
 * @vitest-environment happy-dom
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { writeClipboardText } from "./clipboard";

function setClipboard(writeText: ((text: string) => Promise<void>) | null): void {
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: writeText === null ? undefined : { writeText },
  });
}

describe("writeClipboardText", () => {
  let execCommand: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    execCommand = vi.fn(() => true);
    (document as unknown as { execCommand: unknown }).execCommand = execCommand;
  });

  afterEach(() => {
    setClipboard(null);
  });

  it("bruger Async Clipboard API naar den findes", async () => {
    const writeText = vi.fn(async () => undefined);
    setClipboard(writeText);

    await expect(writeClipboardText("hej")).resolves.toBe(true);

    expect(writeText).toHaveBeenCalledWith("hej");
    expect(execCommand).not.toHaveBeenCalled();
  });

  it("falder tilbage til execCommand naar Async Clipboard afviser", async () => {
    setClipboard(vi.fn(async () => Promise.reject(new Error("nej"))));

    await expect(writeClipboardText("hej")).resolves.toBe(true);

    expect(execCommand).toHaveBeenCalledWith("copy");
  });

  it("falder tilbage naar navigator.clipboard slet ikke findes", async () => {
    setClipboard(null);

    await expect(writeClipboardText("hej")).resolves.toBe(true);

    expect(execCommand).toHaveBeenCalledWith("copy");
  });

  it("returnerer false naar begge veje fejler", async () => {
    setClipboard(vi.fn(async () => Promise.reject(new Error("nej"))));
    execCommand.mockReturnValue(false);

    await expect(writeClipboardText("hej")).resolves.toBe(false);
  });

  it("efterlader ingen scratch-textarea i DOM'en", async () => {
    setClipboard(null);

    await writeClipboardText("hej");

    expect(document.querySelectorAll("textarea")).toHaveLength(0);
  });
});
