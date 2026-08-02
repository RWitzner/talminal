/**
 * @vitest-environment happy-dom
 */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Settings } from "./Settings";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

describe("Settings wallpaper-vælger", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);

    vi.mocked(invoke).mockImplementation(async (command) => {
      if (command === "load_secret") return false;
      if (command === "get_workspace") {
        return {
          settings: {
            ptt_hotkey: "Ctrl+Fresh",
            exit_type_mode_hotkey: "Alt+Fresh",
            voice_engine: "pipeline",
            wallpaper: "blue-folds",
            default_agent: "claude",
            stt_provider: "openai",
            routing_provider: "vercel",
          },
        };
      }
      return undefined;
    });
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    vi.mocked(invoke).mockReset();
  });

  it("bevarer friske settings-felter og kalder onSaved efter wallpaper-save", async () => {
    const onSaved = vi.fn(async () => {});
    await act(async () => {
      root.render(<Settings category="appearance" onSaved={onSaved} />);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    const choice = host.querySelector<HTMLButtonElement>(
      '[data-wallpaper-slug="liquid-only"]',
    );
    expect(choice).not.toBeNull();
    expect(
      choice!.querySelector("[data-wallpaper-liquid-preview]"),
    ).not.toBeNull();

    await act(async () => {
      choice!.click();
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_settings", {
      settings: expect.objectContaining({
        ptt_hotkey: "Ctrl+Fresh",
        exit_type_mode_hotkey: "Alt+Fresh",
        voice_engine: "pipeline",
        wallpaper: "liquid-only",
        default_agent: "claude",
        stt_provider: "openai",
        routing_provider: "vercel",
      }),
    });
    expect(onSaved).toHaveBeenCalledTimes(1);
    expect(choice!.getAttribute("aria-pressed")).toBe("true");
  });
});
