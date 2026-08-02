/**
 * @vitest-environment happy-dom
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MicrophoneSection } from "./Settings";
import { startBrowserCapture } from "./voice/ptt";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("./voice/ptt", () => ({ startBrowserCapture: vi.fn() }));

/** DOMException-agtig fejl; happy-dom har ikke alle konstruktoerer. */
function domError(name: string): Error {
  const error = new Error(`${name}: fejlede`);
  error.name = name;
  return error;
}

describe("MicrophoneSection", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
    // Tauris invoke returnerer ALTID et promise. Uden en default her giver
    // mocken undefined, og produktionskodens .catch() falder over noget der
    // ikke kan ske i drift — en fejl der ville staa i outputtet og skjule
    // aegte fejl.
    vi.mocked(invoke).mockResolvedValue(undefined);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    vi.mocked(invoke).mockReset();
    vi.mocked(startBrowserCapture).mockReset();
  });

  async function mountAndTest() {
    await act(async () => {
      root.render(<MicrophoneSection />);
    });
    const button = host.querySelector<HTMLButtonElement>("button");
    await act(async () => {
      button?.click();
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }

  it("melder klar og slipper mikrofonen igen naar adgangen virker", async () => {
    const stop = vi.fn().mockResolvedValue(undefined);
    vi.mocked(startBrowserCapture).mockResolvedValue({ stop });

    await mountAndTest();

    expect(host.textContent).toContain("Mikrofonen svarer");
    // Testen maa ikke holde enheden aabnet — saa ville den blokere den
    // rigtige optagelse bagefter.
    expect(stop).toHaveBeenCalledTimes(1);
  });

  it("viser den oversatte vejvisning og knappen ved blokeret adgang", async () => {
    vi.mocked(startBrowserCapture).mockRejectedValue(
      domError("NotAllowedError"),
    );

    await mountAndTest();

    expect(host.textContent).toContain("Privatliv");
    expect(host.textContent).not.toContain("NotAllowedError");
    const settingsButton = host.querySelector<HTMLButtonElement>(
      "[data-open-mic-settings]",
    );
    expect(settingsButton).not.toBeNull();

    await act(async () => {
      settingsButton?.click();
    });
    expect(invoke).toHaveBeenCalledWith("open_microphone_settings");
  });

  it("tilbyder IKKE indstillings-knappen naar der bare ingen mikrofon er", async () => {
    vi.mocked(startBrowserCapture).mockRejectedValue(domError("NotFoundError"));

    await mountAndTest();

    expect(host.textContent).toContain("Ingen mikrofon");
    // Privatlivsindstillingen loeser ikke en manglende enhed; en knap derhen
    // ville sende brugeren det forkerte sted hen.
    expect(host.querySelector("[data-open-mic-settings]")).toBeNull();
  });

  it("vaelter ikke hvis skallen ikke kan aabne indstillingerne", async () => {
    vi.mocked(startBrowserCapture).mockRejectedValue(
      domError("NotAllowedError"),
    );
    vi.mocked(invoke).mockRejectedValue(new Error("explorer mangler"));

    await mountAndTest();

    await act(async () => {
      host
        .querySelector<HTMLButtonElement>("[data-open-mic-settings]")
        ?.click();
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    // Vejvisningen staar stadig — genvejen er en bekvemmelighed, ikke vejen.
    expect(host.textContent).toContain("Privatliv");
  });
});
