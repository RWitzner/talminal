/**
 * @vitest-environment happy-dom
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ContextBadge } from "./ContextBadge";

describe("ContextBadge", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    // Repo-præcedens i DOM-render-tests: uden flaget støjer React 19's act().
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  async function render(percent: number | null) {
    await act(async () => {
      root.render(<ContextBadge percent={percent} />);
    });
  }

  it("null → renderer ingenting", async () => {
    await render(null);
    expect(host.querySelector("[data-context-badge]")).toBeNull();
  });

  it("viser CTX-label og afrundet procent med titel", async () => {
    await render(17.6);
    const badge = host.querySelector("[data-context-badge]") as HTMLElement;
    expect(badge.textContent).toBe("CTX 18%");
    expect(badge.title).toContain("18%");
  });

  it("farve følger 70/90-tærsklerne (delt barColor)", async () => {
    await render(18);
    const green = (host.querySelector("[data-context-badge]") as HTMLElement).style.color;
    await render(75);
    const yellow = (host.querySelector("[data-context-badge]") as HTMLElement).style.color;
    await render(95);
    const red = (host.querySelector("[data-context-badge]") as HTMLElement).style.color;
    expect(new Set([green, yellow, red]).size).toBe(3);
  });
});
