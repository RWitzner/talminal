/**
 * @vitest-environment happy-dom
 */

// Agent-sektionen (Task 7): "Agenter" med native <select> — samme moenster
// som wallpaper-sektionen (Settings.wallpaper.test.tsx): frisk-snapshot-
// helperen saveSettingsPatch henter get_workspace lige foer save, saa
// aendring af default_agent ikke overskriver de andre felter.

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Settings } from "./Settings";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

describe("Settings agent-vælger", () => {
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

  it("dropdown viser claude/codex og aendring gemmer default_agent uden at overskrive de andre felter", async () => {
    const onSaved = vi.fn(async () => {});
    await act(async () => {
      root.render(<Settings category="agents" onSaved={onSaved} />);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    const select = host.querySelector<HTMLSelectElement>(
      "[data-default-agent-select]",
    );
    expect(select).not.toBeNull();
    expect(
      Array.from(select!.options).map((option) => option.value),
    ).toEqual(["claude", "codex"]);
    expect(select!.value).toBe("claude");

    await act(async () => {
      select!.value = "codex";
      select!.dispatchEvent(new Event("change", { bubbles: true }));
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_settings", {
      settings: expect.objectContaining({
        ptt_hotkey: "Ctrl+Fresh",
        exit_type_mode_hotkey: "Alt+Fresh",
        voice_engine: "pipeline",
        wallpaper: "blue-folds",
        default_agent: "codex",
        stt_provider: "openai",
        routing_provider: "vercel",
      }),
    });
    expect(onSaved).toHaveBeenCalledTimes(1);
    expect(select!.value).toBe("codex");
  });
});
