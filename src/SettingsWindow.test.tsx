/**
 * @vitest-environment happy-dom
 */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const occlusionMocks = vi.hoisted(() => ({
  setOcclusionReason: vi.fn(),
}));

// Occlusion-gaten mockes saa vi kan assertere PAA kaldene (final-review
// Finding 1, spec §4a) uden at traekke Tauri's invoke() ind i testen — samme
// moenster som Hud.render.test.tsx.
vi.mock("./browser/occlusion", () => ({
  setOcclusionReason: occlusionMocks.setOcclusionReason,
}));

const settingsMocks = vi.hoisted(() => ({ mounts: 0, unmounts: 0 }));

// Settings-panelets indhold er irrelevant for vinduets soemme og laver egen
// async invoke-IO; stub det, saa testen forbliver hermetisk og stoejfri.
// Stubben taeller til gengaeld sine mounts, fordi lazy-loadingen gjorde
// montrings-LEVETIDEN til en egenskab der skal holdes fast, og den skriver
// den modtagne kategori i DOM'en, saa navigationen kan asserteres.
vi.mock("./Settings", async () => {
  const { useEffect } = await import("react");
  return {
    default: ({ category }: { category?: string }) => {
      useEffect(() => {
        settingsMocks.mounts += 1;
        return () => {
          settingsMocks.unmounts += 1;
        };
      }, []);
      return <div data-settings-mock data-category={category} />;
    },
    Settings: () => null,
  };
});

import { hentSettings, SettingsWindow } from "./SettingsWindow";

describe("SettingsWindow", () => {
  let host: HTMLDivElement;
  let root: Root;
  let onClose: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    (
      globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    occlusionMocks.setOcclusionReason.mockClear();
    settingsMocks.mounts = 0;
    settingsMocks.unmounts = 0;
    onClose = vi.fn();
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
  });

  function render(open: boolean, showDebug = true): Promise<void> {
    return act(async () => {
      root.render(
        <SettingsWindow
          open={open}
          onClose={onClose}
          dryRun={false}
          onDryRunChange={() => {}}
          dryRunForced={false}
          showDebug={showDebug}
          capturePath={null}
          onSaved={() => {}}
        />,
      );
    });
  }

  const find = <T extends Element>(selector: string): T | null =>
    host.querySelector<T>(selector);

  const mustFind = <T extends Element>(selector: string): T => {
    const el = find<T>(selector);
    if (el === null) throw new Error(`${selector} ikke fundet`);
    return el;
  };

  // -------------------------------------------------------------------------
  // Occlusion — WebView2-boernene maler OVER DOM'et, saa uden gaten ville et
  // browser-kort daekke hele vinduet.
  // -------------------------------------------------------------------------

  it("aabning registrerer ('settings-panel', true)", async () => {
    await render(false);
    expect(occlusionMocks.setOcclusionReason).not.toHaveBeenCalledWith(
      "settings-panel",
      true,
    );

    await render(true);
    expect(occlusionMocks.setOcclusionReason).toHaveBeenLastCalledWith(
      "settings-panel",
      true,
    );
  });

  it("lukning rydder ('settings-panel', false)", async () => {
    await render(true);
    await render(false);
    expect(occlusionMocks.setOcclusionReason).toHaveBeenLastCalledWith(
      "settings-panel",
      false,
    );
  });

  it("unmount mens vinduet er aabent rydder occlusion-reason", async () => {
    await render(true);
    expect(occlusionMocks.setOcclusionReason).toHaveBeenLastCalledWith(
      "settings-panel",
      true,
    );
    await act(async () => root.unmount());
    expect(occlusionMocks.setOcclusionReason).toHaveBeenLastCalledWith(
      "settings-panel",
      false,
    );
  });

  // -------------------------------------------------------------------------
  // Levetid — den kontrakt der beskytter ugemte kladder i noeglefelterne.
  // -------------------------------------------------------------------------

  it("henter foerst Settings ved foerste aabning", async () => {
    await render(false);
    expect(find("[data-settings-mock]")).toBeNull();
    expect(settingsMocks.mounts).toBe(0);

    await render(true);
    await act(async () => {});
    expect(find("[data-settings-mock]")).not.toBeNull();
    expect(settingsMocks.mounts).toBe(1);
  });

  it("lukning river IKKE Settings ned igen", async () => {
    await render(true);
    await act(async () => {});
    expect(settingsMocks.mounts).toBe(1);

    await render(false);
    await act(async () => {});
    expect(
      find("[data-settings-mock]"),
      "et lukket vindue SKJULER sit indhold — det maa ikke unmountes, ellers " +
        "forsvinder en halvt indtastet API-noegle",
    ).not.toBeNull();
    expect(settingsMocks.unmounts).toBe(0);

    await render(true);
    await act(async () => {});
    expect(settingsMocks.mounts, "gen-aabning maa ikke gen-montere").toBe(1);
  });

  it("lukket vindue er display:none — ikke fjernet", async () => {
    await render(true);
    await render(false);
    expect(
      mustFind<HTMLElement>("[data-settings-window-root]").style.display,
    ).toBe("none");
  });

  it("tegner intet foer foerste aabning", async () => {
    await render(false);
    expect(find("[data-settings-window-root]")).toBeNull();
  });

  // -------------------------------------------------------------------------
  // Opvarmning — gevinsten er at chunken ikke ligger foer foerste frame, ikke
  // at den hentes sent.
  // -------------------------------------------------------------------------

  it("varmer Settings-chunken op i idle, uden at montere panelet", async () => {
    const idle = vi.fn();
    vi.stubGlobal("requestIdleCallback", idle);
    vi.stubGlobal("cancelIdleCallback", vi.fn());
    try {
      await render(false);
      expect(idle).toHaveBeenCalledTimes(1);
      expect(idle.mock.calls[0][1]).toEqual({ timeout: 3_000 });

      await act(async () => {
        (idle.mock.calls[0][0] as () => void)();
      });
      expect(
        settingsMocks.mounts,
        "opvarmning henter modulet — den maa ikke montere det",
      ).toBe(0);
      expect(find("[data-settings-mock]")).toBeNull();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("lazy() og opvarmningen deler praecis samme modul-loefte", () => {
    expect(hentSettings()).toBe(hentSettings());
  });

  // -------------------------------------------------------------------------
  // Afvisning. Vinduet er IKKE modalt (rail'en skal forblive aktiv), saa
  // Esc og daempningen er de to veje ud ud over ✕.
  // -------------------------------------------------------------------------

  it("Escape lukker", async () => {
    await render(true);
    await act(async () => {
      document.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  // Shift+Esc er husets LAASTE exit-type-mode-kombination (keyRouting.ts).
  // Lukkede vinduet ogsaa paa den, ville én tast betyde to ting.
  it("Shift+Escape lukker IKKE", async () => {
    await render(true);
    await act(async () => {
      document.dispatchEvent(
        new KeyboardEvent("keydown", {
          key: "Escape",
          shiftKey: true,
          bubbles: true,
        }),
      );
    });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("Escape lytter kun mens vinduet er aabent", async () => {
    await render(true);
    await render(false);
    await act(async () => {
      document.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
      );
    });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("klik paa daempningen lukker", async () => {
    await render(true);
    await act(async () => {
      mustFind<HTMLElement>("[data-settings-backdrop]").click();
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("✕ lukker", async () => {
    await render(true);
    await act(async () => {
      mustFind<HTMLElement>("[data-settings-close]").click();
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  // -------------------------------------------------------------------------
  // Navigationen
  // -------------------------------------------------------------------------

  it("starter paa Stemme og sender kategorien videre til Settings", async () => {
    await render(true);
    await act(async () => {});
    expect(mustFind("[data-settings-mock]").getAttribute("data-category")).toBe(
      "voice",
    );
    expect(
      mustFind('[data-settings-category-button="voice"]').getAttribute(
        "aria-current",
      ),
    ).toBe("true");
  });

  it("klik i navigationen skifter kategori", async () => {
    await render(true);
    await act(async () => {});
    await act(async () => {
      mustFind<HTMLElement>(
        '[data-settings-category-button="appearance"]',
      ).click();
    });
    expect(mustFind("[data-settings-mock]").getAttribute("data-category")).toBe(
      "appearance",
    );
  });

  // Session-kontakterne persisteres ikke og hoerer kun til paa Stemme-siden.
  it("fejlfindings-kontakterne staar kun paa Stemme-siden", async () => {
    await render(true);
    await act(async () => {});
    expect(find("[data-settings-debug]")).not.toBeNull();

    await act(async () => {
      mustFind<HTMLElement>('[data-settings-category-button="keys"]').click();
    });
    expect(find("[data-settings-debug]")).toBeNull();
  });

  // Blokken er skrevet til udvikling. I den udsendte binaer er `DEV` falsk, og
  // App sender `showDebug={false}` — saa findes den slet ikke.
  it("fejlfindings-blokken er helt væk uden showDebug", async () => {
    await render(true, false);
    await act(async () => {});
    expect(
      mustFind("[data-settings-mock]").getAttribute("data-category"),
      "vi staar stadig paa Stemme-siden",
    ).toBe("voice");
    expect(find("[data-settings-debug]")).toBeNull();
  });
});
