/**
 * @vitest-environment happy-dom
 */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import WindowControls from "./WindowControls";

const windowMocks = vi.hoisted(() => ({
  minimize: vi.fn(async () => {}),
  toggleMaximize: vi.fn(async () => {}),
  close: vi.fn(async () => {}),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => windowMocks,
}));

describe("WindowControls", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (
      globalThis as typeof globalThis & {
        IS_REACT_ACT_ENVIRONMENT: boolean;
      }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    windowMocks.minimize.mockClear();
    windowMocks.toggleMaximize.mockClear();
    windowMocks.close.mockClear();
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  function render(): Promise<void> {
    return act(async () => {
      root.render(<WindowControls />);
    });
  }

  function button(label: string): HTMLButtonElement {
    const element = host.querySelector<HTMLButtonElement>(
      `button[aria-label="${label}"]`,
    );
    if (!element) throw new Error(`knap mangler: ${label}`);
    return element;
  }

  it("kalder vindues-API'et for minimér, maksimér og luk", async () => {
    await render();

    await act(async () => button("Minimér").click());
    expect(windowMocks.minimize).toHaveBeenCalledOnce();

    await act(async () => button("Maksimér eller gendan").click());
    expect(windowMocks.toggleMaximize).toHaveBeenCalledOnce();

    await act(async () => button("Afslut Talminal").click());
    expect(windowMocks.close).toHaveBeenCalledOnce();
  });

  // Task 11: luk-knappen skal vaere DOED mens en luk-bekraeftelse staar aaben,
  // ellers kan brugeren stable to lukninger oven paa hinanden. Testene laa
  // foerst i CloseWorkspaceDialog.test.tsx; de hoerer her, hos komponenten
  // (fix-runde 1, fund 3).
  it("uden closeDisabled er luk-knappen aktiv (uaendret adfaerd)", async () => {
    await render();
    expect(button("Afslut Talminal").disabled).toBe(false);
  });

  it("closeDisabled deaktiverer luk-knappen, saa to lukninger ikke kan stables", async () => {
    await act(async () => {
      root.render(<WindowControls closeDisabled />);
    });
    expect(button("Afslut Talminal").disabled).toBe(true);
    await act(async () => button("Afslut Talminal").click());
    expect(windowMocks.close).not.toHaveBeenCalled();
    // Minimér og maksimér maa IKKE rammes af gaten.
    expect(button("Minimér").disabled).toBe(false);
    expect(button("Maksimér eller gendan").disabled).toBe(false);
  });

  it("knapperne er type=button og er IKKE drag-region (skal kunne klikkes)", async () => {
    await render();
    const buttons = Array.from(host.querySelectorAll("button"));
    expect(buttons).toHaveLength(3);
    for (const element of buttons) {
      expect(element.getAttribute("type")).toBe("button");
      expect(element.hasAttribute("data-tauri-drag-region")).toBe(false);
    }
  });
});
