/**
 * @vitest-environment happy-dom
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  ResetProvidersButton,
  RoutingProviderSection,
  Settings,
} from "./Settings";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const WORKSPACE = {
  settings: {
    ptt_hotkey: "CmdOrCtrl+Shift+Space",
    exit_type_mode_hotkey: "Shift+Escape",
    voice_engine: "pipeline",
    wallpaper: "blue-folds",
    default_agent: "claude",
    stt_provider: "openai",
    routing_provider: "vercel",
  },
  voice_routes: {
    stt: {
      slug: "openai",
      label: "OpenAI",
      model: "gpt-transcribe",
      supports_partials: true,
      supports_domain_prompt: true,
      language_field: "plural",
      key_slot: "provider_key_openai",
    },
    routing: {
      slug: "vercel",
      label: "Vercel AI Gateway",
      model: "google/gemini-3.1-flash-lite",
      key_slot: "provider_key_vercel",
    },
  },
};

/**
 * Hed indtil 2026-08-05 `describe("VoiceProviderSection")`, og routing-testene
 * laa inde i den. Da STT-valget blev nedlagt, var de tre STT-tests ikke det
 * eneste der laa her — fixturen, `beforeEach` og `mount()` var faelles, saa en
 * ren sletning ville have taget routing-daekningen med sig.
 *
 * Tre tests er derfor REHOSTET frem for slettet: de maalte invarianter der
 * gaelder `ProviderSection` som saadan, ikke STT-valget i saerdeleshed.
 */
describe("Provider-sektionerne", () => {
  let host: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    (
      globalThis as typeof globalThis & {
        IS_REACT_ACT_ENVIRONMENT: boolean;
      }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
    vi.mocked(invoke).mockImplementation(async (command) =>
      command === "get_workspace" ? WORKSPACE : undefined,
    );
  });
  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    vi.mocked(invoke).mockReset();
  });
  async function mount() {
    await act(async () => {
      root.render(<RoutingProviderSection />);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }

  // Routing-sektionen ER et valg (STT er det ikke laengere), og OpenAI-ruten er
  // den eneste der ikke kraever en konto ud over den STT allerede bruger.
  // Testen laaser baade at valget findes, og at afvejnings-noten foelger med —
  // uden noten ser ruten ud som et gratis skift, og den er ~2x langsommere.
  it("tilbyder OpenAI som router-valg med afvejnings-noten", async () => {
    await mount();
    const slugs = Array.from(
      host.querySelectorAll<HTMLInputElement>('input[type="radio"]'),
    ).map((input) => input.value);
    expect(slugs).toEqual(["vercel", "google", "openrouter", "openai"]);
    expect(host.textContent).toContain("Kun din OpenAI-nøgle");
  });

  // STT-valget er nedlagt 2026-08-05. Testen er ikke en dubletkontrol af
  // `stt_routes_cover_the_supported_slugs` i Rust: den maaler at UI'ET ikke
  // laengere tegner et valg. Kommer sektionen tilbage ved et uheld — fx fordi
  // nogen genindfoerer `STT_CHOICES` — er dette det eneste sted det falder.
  it("tegner ikke noget STT-valg i voice-kategorien", async () => {
    await act(async () => {
      root.render(<Settings category="voice" />);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(
      host.querySelector('input[name="stt_provider"]'),
    ).toBeNull();
    expect(host.textContent).not.toContain("gpt-4o-transcribe");
    expect(host.textContent).not.toContain("gpt-4o-mini-transcribe");
  });

  // REHOSTET fra STT-sektionen. At MOUNTE maa aldrig skrive. Ud over det
  // aabenlyse (et panel der aabnes skal ikke aendre noget) laeser
  // Settings.hotkey.test.tsx det SIDSTE set_settings-kald, saa en
  // mount-skrivning herfra ville braekke tests i en anden fil paa en maade der
  // er svaer at spore.
  it("skriver ikke til settings ved mount", async () => {
    await mount();
    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith(
      "set_settings",
      expect.anything(),
    );
  });

  // REHOSTET fra STT-sektionen. Det gemte valg skal vinde over listens foerste
  // element, naar panelet aabnes igen. Uden denne test kunne `Settings.tsx`'
  // opslag erstattes af bare `choices[0].slug`, og ALT ville staa groent —
  // mens brugeren der valgte Google saa Vercel markeret ved hver genaabning.
  it("markerer det gemte valg og ikke listens foerste", async () => {
    vi.mocked(invoke).mockImplementation(async (command) =>
      command === "get_workspace"
        ? {
            ...WORKSPACE,
            settings: { ...WORKSPACE.settings, routing_provider: "google" },
            voice_routes: {
              ...WORKSPACE.voice_routes,
              routing: {
                ...WORKSPACE.voice_routes.routing,
                slug: "google",
                model: "gemini-3.1-flash-lite",
              },
            },
          }
        : undefined,
    );
    await mount();
    const valgt = host.querySelector<HTMLInputElement>(
      'input[name="routing_provider"]:checked',
    );
    expect(valgt?.value).toBe("google");
  });

  // REHOSTET fra STT-sektionen (hvor den gik gennem voice-kategorien).
  // Gennemfoeringen af onSaved fra panelet og ned i sektionen er load-bearing:
  // uden den naar valget aldrig `voiceRoutesRef` i App.tsx, og den KOERENDE
  // stemme-session taler videre til den gamle rute. At teste sektionen direkte
  // beviser det ikke — proppen kan tabes paa kaldestedet i `Settings` uden at
  // nogen test faelder.
  it("foerer onSaved gennem routing-kategorien", async () => {
    const onSaved = vi.fn();
    await act(async () => {
      root.render(<Settings category="routing" onSaved={onSaved} />);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    const google = host.querySelector<HTMLInputElement>('input[value="google"]');
    if (google === null) {
      throw new Error("google-valget mangler i routing-kategorien");
    }
    await act(async () => {
      google.click();
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_settings", {
      settings: expect.objectContaining({ routing_provider: "google" }),
    });
    expect(onSaved).toHaveBeenCalled();
  });

  // Patchen roerer stadig BEGGE roller, selv om STT kun har én rute: det er
  // billigt, og det bringer en gammel `openai-mini`-vaerdi tilbage i folden.
  // Hjaelpeteksten lover derimod kun routingen fra 2026-08-05 — se
  // `ResetProvidersButton`.
  it("nulstiller begge roller i ét kald", async () => {
    const onSaved = vi.fn();
    await act(async () => {
      root.render(<ResetProvidersButton onSaved={onSaved} />);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    await act(async () => {
      host.querySelector("button")!.click();
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_settings", {
      settings: expect.objectContaining({
        stt_provider: "openai",
        routing_provider: "vercel",
      }),
    });
    expect(onSaved).toHaveBeenCalled();
  });
});
