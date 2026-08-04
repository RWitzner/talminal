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
  VoiceProviderSection,
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
      model: "gpt-4o-transcribe",
      supports_partials: true,
      supports_domain_prompt: true,
    },
    routing: {
      slug: "vercel",
      label: "Vercel AI Gateway",
      model: "google/gemini-3.1-flash-lite",
    },
  },
};

describe("VoiceProviderSection", () => {
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
      root.render(<VoiceProviderSection />);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }

  // Sektionen er et valg igen (2026-08-04), men et MODEL-valg inden for samme
  // udbyder — ikke det udbyder-valg der blev slettet 2026-07-29. OpenRouter-STT
  // er stadig vaek, og dét skal listen blive ved med at fange.
  it("tilbyder begge transskriptionsmodeller", async () => {
    await mount();
    const slugs = Array.from(
      host.querySelectorAll<HTMLInputElement>('input[type="radio"]'),
    ).map((input) => input.value);
    expect(slugs).toEqual(["openai", "openai-mini"]);
    expect(host.textContent).toContain("gpt-4o-transcribe");
    expect(host.textContent).toContain("gpt-4o-mini-transcribe");
    expect(host.textContent).not.toContain("OpenRouter");
  });

  // Der asserteres KUN paa skrivningen og paa onSaved — ikke paa modeltagget.
  // `choose` henter `get_workspace` igen, og mocken svarer med den samme
  // statiske fixture, saa tagget ville staa paa gpt-4o-transcribe uanset hvad.
  // En assert paa tagget her ville altsaa maale mocken, ikke koden.
  it("gemmer modelvalget og melder det videre", async () => {
    const onSaved = vi.fn();
    await act(async () => {
      root.render(<VoiceProviderSection onSaved={onSaved} />);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    const mini = host.querySelector<HTMLInputElement>(
      'input[value="openai-mini"]',
    );
    if (mini === null) throw new Error("mini-valget mangler");
    await act(async () => {
      mini.click();
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_settings", {
      settings: expect.objectContaining({ stt_provider: "openai-mini" }),
    });
    // onSaved er leddet der naar `voiceRoutesRef` i App.tsx. Uden det taler den
    // KOERENDE stemme-session videre til den gamle model.
    expect(onSaved).toHaveBeenCalled();
  });

  // Routing-sektionen ER et valg (modsat STT), og OpenAI-ruten er den eneste
  // der ikke kraever en konto ud over den STT allerede bruger. Testen laaser
  // baade at valget findes, og at afvejnings-noten foelger med — uden noten
  // ser ruten ud som et gratis skift, og den er ~2x langsommere.
  it("tilbyder OpenAI som router-valg med afvejnings-noten", async () => {
    await act(async () => {
      root.render(<RoutingProviderSection />);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    const slugs = Array.from(
      host.querySelectorAll<HTMLInputElement>('input[type="radio"]'),
    ).map((input) => input.value);
    expect(slugs).toEqual(["vercel", "google", "openrouter", "openai"]);
    expect(host.textContent).toContain("Kun din OpenAI-nøgle");
  });

  // At MOUNTE maa aldrig skrive. Ud over det aabenlyse (et panel der aabnes
  // skal ikke aendre noget) laeser Settings.hotkey.test.tsx det SIDSTE
  // set_settings-kald, saa en mount-skrivning herfra ville braekke tests i en
  // anden fil paa en maade der er svaer at spore.
  it("skriver ikke til settings ved mount", async () => {
    await mount();
    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith(
      "set_settings",
      expect.anything(),
    );
  });

  // Det gemte valg skal vinde over listens foerste element, naar panelet
  // aabnes igen. Uden denne test kunne `Settings.tsx`' opslag erstattes af bare
  // `choices[0].slug`, og ALT ville staa groent — mens brugeren der valgte mini
  // saa gpt-4o-transcribe markeret ved hver genaabning.
  it("markerer det gemte valg og ikke listens foerste", async () => {
    vi.mocked(invoke).mockImplementation(async (command) =>
      command === "get_workspace"
        ? {
            ...WORKSPACE,
            settings: { ...WORKSPACE.settings, stt_provider: "openai-mini" },
            voice_routes: {
              ...WORKSPACE.voice_routes,
              stt: {
                ...WORKSPACE.voice_routes.stt,
                slug: "openai-mini",
                model: "gpt-4o-mini-transcribe",
              },
            },
          }
        : undefined,
    );
    await mount();
    const valgt = host.querySelector<HTMLInputElement>(
      'input[name="stt_provider"]:checked',
    );
    expect(valgt?.value).toBe("openai-mini");
  });

  // Gennemfoeringen af onSaved fra panelet og ned i sektionen er load-bearing:
  // uden den naar modelvalget aldrig `voiceRoutesRef` i App.tsx, og den KOERENDE
  // stemme-session taler videre til den gamle model. At teste
  // VoiceProviderSection direkte beviser det ikke — proppen kan tabes paa
  // kaldestedet i `Settings` uden at nogen test faelder.
  it("foerer onSaved gennem voice-kategorien", async () => {
    const onSaved = vi.fn();
    await act(async () => {
      root.render(<Settings category="voice" onSaved={onSaved} />);
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    const mini = host.querySelector<HTMLInputElement>(
      'input[value="openai-mini"]',
    );
    if (mini === null) throw new Error("mini-valget mangler i voice-kategorien");
    await act(async () => {
      mini.click();
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(onSaved).toHaveBeenCalled();
  });

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
