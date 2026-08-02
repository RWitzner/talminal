/**
 * @vitest-environment happy-dom
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

import Settings from "./Settings";

let container: HTMLDivElement;
let root: Root;

function mockWorkspace(settings: Record<string, string>) {
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "get_workspace") {
      return {
        schema_version: 1,
        next_card_number: 1,
        viewport: { x: 0, y: 0, zoom: 1 },
        settings: {
          ptt_hotkey: "CmdOrCtrl+Shift+Space",
          exit_type_mode_hotkey: "Shift+Escape",
          voice_engine: "pipeline",
          wallpaper: "blue-folds",
          default_agent: "claude",
          ...settings,
        },
        settings_warning: null,
        cards: [],
      };
    }
    if (command === "load_secret") return false;
    return undefined;
  });
}

beforeEach(() => {
  mocks.invoke.mockReset();
  mockWorkspace({});
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount() {
  act(() => root.render(<Settings category="voice" />));
  await act(async () => {});
}

it("exit-type-mode-feltet er fjernet", async () => {
  await mount();
  expect(container.textContent).toContain("Stemme-aktivering");
  expect(container.textContent).not.toContain("Exit type-mode");
});

it("nulstil skriver standard-hotkeyen", async () => {
  mockWorkspace({ ptt_hotkey: "Alt+KeyQ" });
  await mount();
  // Nulstil bor efter redesignet 2026-07-29 inde i optageren som en stille
  // tekst-handling, ikke som selvstaendig kant-knap under den.
  const reset = container.querySelector<HTMLButtonElement>(
    "[data-hotkey-reset]",
  );
  if (reset === null) throw new Error("nulstil-knappen mangler");
  act(() => reset.dispatchEvent(new MouseEvent("click", { bubbles: true })));
  await act(async () => {});
  expect(mocks.invoke).toHaveBeenCalledWith(
    "set_settings",
    expect.objectContaining({
      settings: expect.objectContaining({
        ptt_hotkey: "CmdOrCtrl+Shift+Space",
      }),
    }),
  );
});

it("viser Mouse4-dobbeltvirkningen uden at blokere", async () => {
  mockWorkspace({ ptt_hotkey: "Mouse4" });
  await mount();
  expect(container.textContent).toContain("navigerer også frem/tilbage");
  const change = container.querySelector<HTMLButtonElement>(
    "[data-hotkey-change]",
  );
  expect(change?.disabled).toBe(false);
});
