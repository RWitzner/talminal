/**
 * @vitest-environment happy-dom
 *
 * Noeglesiden. Testen findes, fordi hullet den daekker var tavst: STT kunne
 * vaelges til OpenRouter og routeren til Google direkte eller OpenRouter, men
 * UI'et havde kun felter til OpenAI og Vercel. Eneste symptom var, at ruten
 * fejlede naar man brugte den.
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

import Settings from "./Settings";

let container: HTMLDivElement;
let root: Root;

/** De fire slots i Rusts providers.rs (KEY_SLOT_*). */
const ALLE_SLOTS = [
  "provider_key_openai",
  "provider_key_vercel",
  "provider_key_google",
  "provider_key_openrouter",
];

function mockBackend({
  sttSlot = "provider_key_openai",
  routingSlot = "provider_key_vercel",
  gemte = [] as string[],
} = {}) {
  mocks.invoke.mockImplementation(
    async (command: string, args?: { key?: string }) => {
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
          },
          voice_routes: {
            stt: { slug: "x", label: "x", key_slot: sttSlot },
            routing: { slug: "y", label: "y", key_slot: routingSlot },
          },
          settings_warning: null,
          cards: [],
        };
      }
      if (command === "load_secret") {
        return gemte.includes(args?.key ?? "");
      }
      return undefined;
    },
  );
}

beforeEach(() => {
  // Husets teststil (UsageHud.render.test.tsx): uden flaget stoejer React 19's
  // act() med console.error i hver test.
  (
    globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  mocks.invoke.mockReset();
  mockBackend();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount() {
  act(() => root.render(<Settings category="keys" />));
  await act(async () => {});
}

const row = (slot: string): HTMLElement => {
  const el = container.querySelector<HTMLElement>(`[data-secret-slot="${slot}"]`);
  if (el === null) throw new Error(`ingen raekke for ${slot}`);
  return el;
};

const buttonIn = (parent: HTMLElement, label: string): HTMLButtonElement => {
  const found = Array.from(parent.querySelectorAll("button")).find((b) =>
    b.textContent?.includes(label),
  );
  if (!found) throw new Error(`ingen ${label}-knap`);
  return found;
};

/** React lytter paa den native setter — en ren `.value =` ses ikke. */
function type(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    window.HTMLInputElement.prototype,
    "value",
  )!.set!;
  setter.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

describe("noeglesiden", () => {
  it("har et felt til hvert af de fire slots", async () => {
    await mount();
    const set = Array.from(
      container.querySelectorAll("[data-secret-slot]"),
    ).map((el) => el.getAttribute("data-secret-slot"));
    expect(set).toEqual(ALLE_SLOTS);
  });

  it("markerer de slots det aktuelle udbyder-valg bruger", async () => {
    mockBackend({
      sttSlot: "provider_key_openrouter",
      routingSlot: "provider_key_google",
    });
    await mount();

    expect(
      row("provider_key_openrouter").querySelector("[data-secret-required]"),
    ).not.toBeNull();
    expect(
      row("provider_key_google").querySelector("[data-secret-required]"),
    ).not.toBeNull();
    expect(
      row("provider_key_vercel").querySelector("[data-secret-required]"),
    ).toBeNull();
  });

  // Maerkatet maa ikke spaerre: man skal kunne gemme noeglen FOER man skifter
  // udbyder, ellers er der ingen vej ind i den nye udbyder.
  it("en umarkeret noegle kan gemmes alligevel", async () => {
    await mount();
    const raekke = row("provider_key_google");
    type(raekke.querySelector("input")!, "AIzaTest");
    await act(async () => {});
    await act(async () => buttonIn(raekke, "Gem").click());

    expect(mocks.invoke).toHaveBeenCalledWith("store_secret", {
      key: "provider_key_google",
      value: "AIzaTest",
    });
  });

  it("gemmer OpenRouter-noeglen under sit eget slot", async () => {
    await mount();
    const raekke = row("provider_key_openrouter");
    type(raekke.querySelector("input")!, "sk-or-test");
    await act(async () => {});
    await act(async () => buttonIn(raekke, "Gem").click());

    expect(mocks.invoke).toHaveBeenCalledWith("store_secret", {
      key: "provider_key_openrouter",
      value: "sk-or-test",
    });
  });

  it("Fjern er kun aktiv naar noeglen faktisk er gemt", async () => {
    mockBackend({ gemte: ["provider_key_openai"] });
    await mount();
    expect(buttonIn(row("provider_key_openai"), "Fjern").disabled).toBe(false);
    expect(buttonIn(row("provider_key_google"), "Fjern").disabled).toBe(true);
  });

  it("Fjern sletter det rigtige slot", async () => {
    mockBackend({ gemte: ["provider_key_openrouter"] });
    await mount();
    await act(async () =>
      buttonIn(row("provider_key_openrouter"), "Fjern").click(),
    );
    expect(mocks.invoke).toHaveBeenCalledWith("delete_secret", {
      key: "provider_key_openrouter",
    });
  });

  // Markeringen er en hjaelp, ikke en funktion.
  it("felterne virker selv om rute-opslaget fejler", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_workspace") throw new Error("ingen workspace");
      if (command === "load_secret") return false;
      return undefined;
    });
    await mount();
    expect(container.querySelectorAll("[data-secret-slot]")).toHaveLength(4);
    expect(container.querySelector("[data-secret-required]")).toBeNull();
  });
});
