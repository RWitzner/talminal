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

function mockWorkspace(settings: Record<string, string | boolean>) {
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

// --- Diktering -------------------------------------------------------------

function savedSettings(): Record<string, unknown> {
  const calls = mocks.invoke.mock.calls.filter(
    (call: unknown[]) => call[0] === "set_settings",
  );
  const last = calls[calls.length - 1];
  if (!last) throw new Error("set_settings blev aldrig kaldt");
  return (last[1] as { settings: Record<string, unknown> }).settings;
}

it("dikteringen har sin egen genvej og sin egen kontakt", async () => {
  await mount();
  expect(container.textContent).toContain("Diktering");
  expect(container.querySelectorAll("[data-hotkey-change]").length).toBe(2);
  expect(container.querySelector("[data-dictation-submit]")).not.toBeNull();
});

it("auto-send er slaaet fra som udgangspunkt og siger hvad den goer", async () => {
  await mount();
  const toggle = container.querySelector<HTMLInputElement>(
    "[data-dictation-submit]",
  );
  expect(toggle?.checked).toBe(false);
  // Konsekvensen skal staa der — ikke kun loeftet om fart.
  expect(container.textContent).toContain("også hvis den blev hørt forkert");
});

it("kontakten gemmer auto-send", async () => {
  await mount();
  const toggle = container.querySelector<HTMLInputElement>(
    "[data-dictation-submit]",
  )!;
  // click() og ikke et sat .checked: React binder checkboxes' onChange til
  // click-eventet, saa en manuelt sat property fyrer ingen handler.
  act(() => toggle.click());
  await act(async () => {});
  expect(savedSettings().dictation_submit).toBe(true);
});

it("en gemt kontakt paa true kan slaas FRA igen", async () => {
  // `??` og ikke `||` i saveSettingsPatch: med `||` ville `false` falde
  // igennem til den gemte `true`, og kontakten kunne aldrig slukkes.
  mockWorkspace({ dictation_submit: true });
  await mount();
  const toggle = container.querySelector<HTMLInputElement>(
    "[data-dictation-submit]",
  )!;
  expect(toggle.checked).toBe(true);
  act(() => toggle.click());
  await act(async () => {});
  expect(savedSettings().dictation_submit).toBe(false);
});

it("ENHVER gem-vej sender ogsaa dikterings-felterne og keyword-listen", async () => {
  // Regressionsvaern for `deny_unknown_fields` paa Rust-sidens SettingsInput:
  // udelades ét felt, fejler ALLE settings-gem — ikke kun dikteringens.
  //
  // `stt_keywords` kom til 2026-08-05 og har praecis samme sagsforhold: den
  // gemmes fra en HELT anden sektion end hotkey-optageren, saa uden denne
  // assertion ville et glemt felt i `saveSettingsPatch` foerst vise sig naar
  // en bruger aendrede noget tredje.
  await mount();
  const reset = container.querySelector<HTMLButtonElement>(
    "[data-hotkey-reset]",
  )!;
  act(() => reset.dispatchEvent(new MouseEvent("click", { bubbles: true })));
  await act(async () => {});
  const settings = savedSettings();
  expect(settings.dictation_hotkey).toBe("CmdOrCtrl+Shift+KeyD");
  expect(settings.dictation_submit).toBe(false);
  expect(settings.stt_keywords).toBeDefined();
});

it("nulstil paa diktér-optageren skriver dens egen standard", async () => {
  mockWorkspace({ dictation_hotkey: "Alt+KeyM" });
  await mount();
  // Anden optager i "voice"-kategorien er dikteringens.
  const resets = container.querySelectorAll<HTMLButtonElement>(
    "[data-hotkey-reset]",
  );
  act(() =>
    resets[1].dispatchEvent(new MouseEvent("click", { bubbles: true })),
  );
  await act(async () => {});
  expect(savedSettings().dictation_hotkey).toBe("CmdOrCtrl+Shift+KeyD");
});
