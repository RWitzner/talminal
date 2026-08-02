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

  // OpenRouter-STT blev slettet 2026-07-29. Sektionen er ikke laengere et
  // valg — den fortaeller hvad der koeres paa, og tilbyder testknappen.
  it("viser ruten uden at tilbyde et valg", async () => {
    await mount();
    expect(host.textContent).toContain("OpenAI");
    expect(host.textContent).toContain("gpt-4o-transcribe");
    expect(
      host.querySelector('input[type="radio"]'),
      "der er kun én rute — ingen radioknapper",
    ).toBeNull();
    expect(host.textContent).not.toContain("OpenRouter");
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

  it("skriver ikke til settings", async () => {
    await mount();
    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith(
      "set_settings",
      expect.anything(),
    );
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
