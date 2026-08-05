/**
 * @vitest-environment happy-dom
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("./Settings", () => ({ default: () => null, Settings: () => null }));
vi.mock("./CanvasSurface", () => ({ CanvasSurface: () => null }));

import App from "./App";

let container: HTMLDivElement;
let root: Root;

function mockWorkspace(
  settings: Record<string, string>,
  settingsWarning: string | null = null,
) {
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
        settings_warning: settingsWarning,
        cards: [],
      };
    }
    if (command === "list_cards") return [];
    return undefined;
  });
}

beforeEach(() => {
  mocks.invoke.mockReset();
  mocks.listeners.clear();
  mocks.listen.mockImplementation(
    async (name: string, callback: (event: { payload: unknown }) => void) => {
      mocks.listeners.set(name, callback);
      return () => mocks.listeners.delete(name);
    },
  );
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
  act(() => root.render(<App />));
  await act(async () => {});
}

it("registrerer ikke DOM-lytteren for en muse-binding", async () => {
  mockWorkspace({ ptt_hotkey: "Mouse4" });
  await mount();
  expect(mocks.invoke).toHaveBeenCalledWith("configure_wake_hotkey", {
    accel: "Mouse4",
    // Wiren baerer nu ogsaa den valgfrie ANDEN binding. `null` = ingen.
    alt: null,
  });
  expect(container.textContent).not.toContain("Voice-hotkey fejlede");
});

it("viser settings_warning og lader den afvise", async () => {
  mockWorkspace({}, "Voice-hotkeyen i settings.json kunne ikke laeses");
  await mount();
  expect(container.textContent).toContain("kunne ikke laeses");
  const close = Array.from(container.querySelectorAll("button")).find((button) =>
    button.textContent?.includes("Luk advarsel"),
  );
  act(() => close?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
  expect(container.textContent).not.toContain("kunne ikke laeses");
});
