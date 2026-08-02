/**
 * @vitest-environment happy-dom
 */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BrowserCard, normalizeBrowserUrlInput } from "./BrowserCard";
import type { BrowserCardInfo } from "./types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => undefined),
}));

const baseCard: BrowserCardInfo = {
  kind: "browser",
  number: 5,
  name: "card-5",
  cwd: "",
  profile: "",
  running: true,
  exited: null,
  restore_action: null,
  opened_by: "card-3",
  url: "https://example.com",
  title: "Example",
};

function typeInto(input: HTMLInputElement, value: string): void {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )?.set;
  setter?.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

describe("BrowserCard", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.mocked(invoke).mockClear();
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  async function render(
    card: BrowserCardInfo,
    onToggleFullscreen: () => void = () => {},
    fullscreen = false,
    onClosed?: () => void,
  ): Promise<void> {
    await act(async () => {
      root.render(
        <BrowserCard
          card={card}
          fullscreen={fullscreen}
          onClosed={onClosed}
          onToggleFullscreen={onToggleFullscreen}
        />,
      );
      await Promise.resolve();
    });
  }

  it("viser nummer, opened_by-chip og titel i headeren", async () => {
    await render(baseCard);
    expect(
      host.querySelector<HTMLElement>("[data-browser-number-badge]")?.textContent,
    ).toBe("5");
    // Chippen viser kortet der aabnede browseren i LAESEFORM. Fixturens
    // opened_by er fortsat wire-navnet "card-3" (se baseCard) — at de to
    // afviger her ER pointen: oversaettelsen sker i visningen, ikke i data.
    expect(
      host.querySelector<HTMLElement>("[data-browser-opened-by]")?.textContent,
    ).toBe("Kort 3");
    expect(
      host.querySelector<HTMLElement>("[data-browser-title]")?.textContent,
    ).toBe("Example");
  });

  it("prefill'er URL-feltet og navigerer paa Enter med https for en bar host", async () => {
    await render(baseCard);
    const input = host.querySelector<HTMLInputElement>(
      "[data-browser-url-input]",
    );
    expect(input?.value).toBe("https://example.com");

    await act(async () => {
      typeInto(input!, "github.com");
      await Promise.resolve();
    });
    await act(async () => {
      input!.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
      await Promise.resolve();
    });

    expect(vi.mocked(invoke)).toHaveBeenCalledWith("navigate_browser_card", {
      name: "card-5",
      url: "https://github.com",
    });
    expect(input!.value).toBe("https://github.com");
  });

  it("bevarer eksplicitte schemes, så backend fortsat håndhæver allowlisten", () => {
    expect(normalizeBrowserUrlInput(" https://example.com/docs ")).toBe(
      "https://example.com/docs",
    );
    expect(normalizeBrowserUrlInput("file:///C:/secret.txt")).toBe(
      "file:///C:/secret.txt",
    );
    expect(normalizeBrowserUrlInput("   ")).toBeNull();
  });

  it("forveksler ikke localhost eller en host med port med et scheme", () => {
    expect(normalizeBrowserUrlInput("localhost:3000")).toBe(
      "https://localhost:3000",
    );
    expect(normalizeBrowserUrlInput("github.com:443/docs")).toBe(
      "https://github.com:443/docs",
    );
    expect(normalizeBrowserUrlInput("devbox:3000")).toBe(
      "https://devbox:3000",
    );
    // Punktum i "schemet" = host, ikke RFC-scheme — må ikke passere raat
    // (rå pass-through ville blive afvist stumt af backend-allowlisten).
    expect(normalizeBrowserUrlInput("example.com:8080abc")).toBe(
      "https://example.com:8080abc",
    );
  });

  it("viser navigationsfejl i kortet i stedet for at sluge dem", async () => {
    await render(baseCard);
    vi.mocked(invoke).mockRejectedValueOnce("unsupported url scheme: file");
    const input = host.querySelector<HTMLInputElement>(
      "[data-browser-url-input]",
    );
    await act(async () => {
      typeInto(input!, "file:///C:/secret.txt");
      await Promise.resolve();
    });
    await act(async () => {
      input!.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
      );
      await Promise.resolve();
    });
    const strip = host.querySelector<HTMLElement>("[data-browser-error]");
    expect(strip?.textContent).toContain("Navigation afvist");
    expect(strip?.textContent).toContain("unsupported url scheme");
  });

  it("viser luk-fejl i kortet i stedet for at sluge dem", async () => {
    await render(baseCard);
    vi.mocked(invoke).mockRejectedValueOnce(
      "native webview close failed: wedged",
    );
    const closeButton = host.querySelector<HTMLButtonElement>(
      "[data-browser-close-action]",
    );
    await act(async () => {
      closeButton!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });
    const strip = host.querySelector<HTMLElement>("[data-browser-error]");
    expect(strip?.textContent).toContain("Kunne ikke lukke kortet");
  });

  it("resynker ikke URL-feltet fra en poll-refresh midt i en indtastning", async () => {
    await render(baseCard);
    const input = host.querySelector<HTMLInputElement>(
      "[data-browser-url-input]",
    )!;
    await act(async () => {
      typeInto(input, "github.co");
      await Promise.resolve();
    });
    // 1s-pollen leverer en ny card.url mens brugeren stadig taster.
    await render({ ...baseCard, url: "https://example.com/poll-update" });
    expect(input.value).toBe("github.co");
    // Efter blur (redigering slut) slår næste reelle url-skifte igennem igen.
    await act(async () => {
      input.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
      await Promise.resolve();
    });
    await render({ ...baseCard, url: "https://example.com/after-blur" });
    expect(input.value).toBe("https://example.com/after-blur");
  });

  it("kalder onToggleFullscreen naar fuldskaerm-knappen klikkes", async () => {
    const onToggleFullscreen = vi.fn();
    await render(baseCard, onToggleFullscreen);
    const button = host.querySelector<HTMLButtonElement>(
      "[data-browser-fullscreen-action]",
    );
    await act(async () => {
      button!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });
    expect(onToggleFullscreen).toHaveBeenCalledTimes(1);
  });

  it("viser en eksplicit afslut-fuldskærm-kontrol i fuldskærm", async () => {
    await render(baseCard, () => {}, true);
    const button = host.querySelector<HTMLButtonElement>(
      "[data-browser-fullscreen-action]",
    );
    expect(button?.getAttribute("aria-label")).toBe("Afslut fuldskærm");
    expect(
      host.querySelector<HTMLElement>("[data-browser-fullscreen]")?.dataset
        .browserFullscreen,
    ).toBe("true");
  });

  it("viser doed-tilstand med luk-knap der invoker close_card (ingen genaabn)", async () => {
    await render({ ...baseCard, running: false });
    expect(host.textContent).toContain("Browserprocessen er død — luk kortet");
    // YAGNI: ingen genaabn-knap i v1.
    expect(host.textContent).not.toContain("Genåbn");
    const closeButton = host.querySelector<HTMLButtonElement>(
      "[data-browser-close-action]",
    );
    expect(closeButton).not.toBeNull();
    await act(async () => {
      closeButton!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("close_card", {
      name: "card-5",
    });
  });

  it("giver body-elementet data-browser-card-body i koerende tilstand", async () => {
    await render(baseCard);
    const body = host.querySelector<HTMLElement>("[data-browser-card-body]");
    expect(body).not.toBeNull();
  });

  it("melder en vellykket lukning tilbage, så workspace-listen refreshes", async () => {
    const onClosed = vi.fn();
    await render(baseCard, () => {}, false, onClosed);
    const closeButton = host.querySelector<HTMLButtonElement>(
      "[data-browser-close-action]",
    );
    await act(async () => {
      closeButton!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });
    expect(onClosed).toHaveBeenCalledTimes(1);
  });
});
