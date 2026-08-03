/**
 * @vitest-environment happy-dom
 */

import { act, createRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CanvasSurface, type CanvasController } from "./CanvasSurface";
import type { CardInfo } from "./types";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async () => undefined),
}));

// ChatCard abonnerer paa chat-thread-updated; surface-testen har ingen
// Tauri-vaert, saa listen stubbes til en no-op unlisten.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));

vi.mock("./Card", () => ({
  PREPARE_FRESH_SPAWN_EVENT: "talminal:prepare-fresh-spawn",
  Card: ({ card }: { card: CardInfo }) => (
    <section data-card-mock={card.number}>{card.name}</section>
  ),
}));

class ResizeObserverMock {
  static instances: ResizeObserverMock[] = [];

  constructor(private readonly callback: ResizeObserverCallback) {
    ResizeObserverMock.instances.push(this);
  }

  observed: Element[] = [];

  observe(target?: Element): void {
    if (target) this.observed.push(target);
  }

  unobserve(): void {}

  disconnect(): void {}

  trigger(): void {
    this.callback([], this as unknown as ResizeObserver);
  }
}

function cards(count: number): CardInfo[] {
  return Array.from({ length: count }, (_, index) => {
    const number = index + 1;
    return {
      kind: "terminal",
      number,
      name: `card-${number}`,
      cwd: "C:\\project",
      profile: "claude",
      running: true,
      exited: null,
      restore_action: null,
      opened_by: null,
      url: null,
      title: null,
    };
  });
}

function browserCard(number = 1): CardInfo {
  return {
    kind: "browser",
    number,
    name: `card-${number}`,
    cwd: "",
    profile: "",
    running: true,
    exited: null,
    restore_action: null,
    opened_by: null,
    url: "https://example.com",
    title: "Example",
  };
}

function chatCard(number = 1): CardInfo {
  return {
    kind: "chat",
    number,
    name: `card-${number}`,
    cwd: "",
    profile: "",
    running: true,
    exited: null,
    restore_action: null,
    opened_by: null,
    url: null,
    title: null,
    thread_id: "t1",
    purpose: "spar om submit",
  };
}

describe("CanvasSurface responsive tile-layout", () => {
  let host: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.mocked(invoke).mockClear();
    ResizeObserverMock.instances = [];
    vi.stubGlobal("ResizeObserver", ResizeObserverMock);
    (
      globalThis as typeof globalThis & {
        IS_REACT_ACT_ENVIRONMENT: boolean;
      }
    ).IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div");
    document.body.append(host);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    vi.unstubAllGlobals();
  });

  it("renderer tre kort som venstre hero og højre stack", async () => {
    const cardList = cards(3);
    await act(async () => {
      root.render(
        <CanvasSurface cards={cardList} />,
      );
      await Promise.resolve();
    });

    const frames = Array.from(
      host.querySelectorAll<HTMLElement>("[data-card-frame]"),
    );
    expect(frames).toHaveLength(3);
    expect(frames.map((frame) => frame.dataset.cardRenderMode)).toEqual([
      "live",
      "live",
      "live",
    ]);
    expect(frames.every((frame) => frame.style.transform === "")).toBe(true);
    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith(
      "set_card_visible",
      expect.anything(),
    );
    expect(
      frames.map((frame) => ({
        column: frame.dataset.gridColumn,
        row: frame.dataset.gridRow,
        columnSpan: frame.dataset.gridColumnSpan,
        rowSpan: frame.dataset.gridRowSpan,
      })),
    ).toEqual([
      { column: "1", row: "1", columnSpan: "1", rowSpan: "2" },
      { column: "2", row: "1", columnSpan: "1", rowSpan: "1" },
      { column: "2", row: "2", columnSpan: "1", rowSpan: "1" },
    ]);
  });

  it("skifter to kort mellem kolonner og rækker uden at remounte terminalerne eller persistere geometri", async () => {
    const cardList = cards(2);
    await act(async () => {
      root.render(
        <CanvasSurface cards={cardList} />,
      );
      await Promise.resolve();
    });

    const canvas = host.querySelector<HTMLElement>("[data-canvas-root]");
    const grid = host.querySelector<HTMLElement>("[data-terminal-grid]");
    expect(canvas).not.toBeNull();
    expect(grid).not.toBeNull();

    let width = 1800;
    let height = 700;
    Object.defineProperties(canvas!, {
      clientWidth: { configurable: true, get: () => width },
      clientHeight: { configurable: true, get: () => height },
    });
    const observer = ResizeObserverMock.instances[0];

    await act(async () => observer.trigger());
    expect(grid!.dataset.gridColumns).toBe("2");
    expect(grid!.dataset.gridRows).toBe("1");
    expect(grid!.style.gridTemplateColumns).toBe(
      "repeat(2, minmax(0, 1fr))",
    );
    expect(grid!.style.gridTemplateRows).toBe("repeat(1, minmax(0, 1fr))");
    const landscapeFrames = Array.from(
      host.querySelectorAll<HTMLElement>("[data-card-frame]"),
    );
    expect(landscapeFrames.map((frame) => frame.style.gridColumn)).toEqual([
      "1 / span 1",
      "2 / span 1",
    ]);
    const originalCards = Array.from(
      host.querySelectorAll<HTMLElement>("[data-card-mock]"),
    );

    width = 600;
    height = 1000;
    await act(async () => observer.trigger());
    expect(grid!.dataset.gridColumns).toBe("1");
    expect(grid!.dataset.gridRows).toBe("2");
    expect(grid!.style.gridTemplateColumns).toBe(
      "repeat(1, minmax(0, 1fr))",
    );
    expect(grid!.style.gridTemplateRows).toBe("repeat(2, minmax(0, 1fr))");
    const portraitFrames = Array.from(
      host.querySelectorAll<HTMLElement>("[data-card-frame]"),
    );
    expect(portraitFrames.map((frame) => frame.style.gridRow)).toEqual([
      "1 / span 1",
      "2 / span 1",
    ]);
    const resizedCards = Array.from(
      host.querySelectorAll<HTMLElement>("[data-card-mock]"),
    );
    resizedCards.forEach((card, index) => expect(card).toBe(originalCards[index]));
    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith(
      "update_card_geometry",
      expect.anything(),
    );
    expect(vi.mocked(invoke)).not.toHaveBeenCalledWith(
      "set_viewport",
      expect.anything(),
    );
  });

  it("vælger topologi ud fra canvashøjden minus begge clearances", async () => {
    const cardList = cards(2);
    await act(async () => {
      root.render(<CanvasSurface cards={cardList} />);
      await Promise.resolve();
    });

    const canvas = host.querySelector<HTMLElement>("[data-canvas-root]");
    const grid = host.querySelector<HTMLElement>("[data-terminal-grid]");
    const width = 700;
    const height = 800;
    Object.defineProperties(canvas!, {
      clientWidth: { configurable: true, get: () => width },
      clientHeight: { configurable: true, get: () => height },
    });

    await act(async () => ResizeObserverMock.instances[0].trigger());
    // Højden minus topbaren alene (800 - 50 = 750 > 700) ville vælge
    // portræt; den reelle flade mellem topbar og orb-bånd
    // (800 - 50 - 88 = 662 < 700) skal vælge landskab.
    expect(grid!.dataset.gridColumns).toBe("2");
    expect(grid!.dataset.gridRows).toBe("1");
  });

  it("reserverer orb-båndet som paddingBottom i gridden", async () => {
    await act(async () => {
      root.render(<CanvasSurface cards={cards(2)} />);
      await Promise.resolve();
    });

    const grid = host.querySelector<HTMLElement>("[data-terminal-grid]");
    // Default-padding 18 + ORB_DOCK_CLEARANCE 88 — terminalerne må aldrig
    // tegne ind i orbens bundbånd.
    expect(grid!.style.paddingBottom).toBe("106px");
    // Default-padding 18 + TOP_ZONE_CLEARANCE 58 (titlebar 50 + 8px luft).
    // Tallet var 84 indtil 2026-08-03: de 26 ekstra reserverede plads til en
    // HUD-chip der flyttede til bundbåndet dagen efter reservationen blev
    // lavet. Se responsiveLayout.ts for datoerne.
    expect(grid!.style.paddingTop).toBe("76px");
  });

  it("tegner wallpaperet i glaslaget alene — roden har ingen egen baggrund", async () => {
    await act(async () => {
      root.render(<CanvasSurface cards={cards(1)} />);
      await Promise.resolve();
    });

    const canvasRoot = host.querySelector<HTMLElement>("[data-canvas-root]");
    const glassWallpaper = host.querySelector<HTMLElement>(
      "[data-canvas-liquid-wallpaper]",
    );
    expect(glassWallpaper?.style.backgroundImage).toContain(
      "canvas-wallpaper-blue-folds.webp",
    );
    // Glaslagets rod er opak og dækker canvas-roden fuldstændigt — en
    // baggrund på roden er dødt render af det 1,4 MB store wallpaper.
    expect(canvasRoot?.style.backgroundImage).toBe("");
  });

  it("renderer den valgte wallpaper-slug i glaslaget", async () => {
    await act(async () => {
      root.render(
        <CanvasSurface cards={cards(1)} wallpaper="ember-dunes" />,
      );
      await Promise.resolve();
    });

    const glassWallpaper = host.querySelector<HTMLElement>(
      "[data-canvas-liquid-wallpaper]",
    );
    expect(glassWallpaper?.style.backgroundImage).toContain(
      "canvas-wallpaper-ember-dunes.webp",
    );
  });

  it("renderer liquid-only uden wallpaper-DOM-laget", async () => {
    await act(async () => {
      root.render(
        <CanvasSurface cards={cards(1)} wallpaper="liquid-only" />,
      );
      await Promise.resolve();
    });

    const glass = host.querySelector<HTMLElement>("[data-canvas-liquid-glass]");
    expect(glass?.dataset.canvasLiquidMode).toBe("liquid-only");
    expect(host.querySelector("[data-canvas-liquid-wallpaper]")).toBeNull();
    expect(host.querySelector("[data-canvas-liquid-material]")).not.toBeNull();
  });

  it("falder tilbage til default-wallpaper ved en ukendt slug", async () => {
    await act(async () => {
      root.render(
        <CanvasSurface cards={cards(1)} wallpaper="ikke-bundlet" />,
      );
      await Promise.resolve();
    });

    const glassWallpaper = host.querySelector<HTMLElement>(
      "[data-canvas-liquid-wallpaper]",
    );
    expect(glassWallpaper?.style.backgroundImage).toContain(
      "canvas-wallpaper-blue-folds.webp",
    );
  });

  it("rydder type-mode, når det fokuserede kort lukkes", async () => {
    const controller = createRef<CanvasController>();
    const initialCards = cards(2);
    await act(async () => {
      root.render(
        <CanvasSurface
          ref={controller}
          cards={initialCards}
        />,
      );
      await Promise.resolve();
    });

    await act(async () => controller.current?.focusCard(2));
    const canvas = host.querySelector<HTMLElement>("[data-canvas-root]");
    expect(canvas?.dataset.canvasMode).toBe("type");
    expect(canvas?.dataset.focusedCard).toBe("2");
    const focusedFrame = host.querySelector<HTMLElement>(
      '[data-card-frame][data-card-number="2"]',
    );
    const unfocusedFrame = host.querySelector<HTMLElement>(
      '[data-card-frame][data-card-number="1"]',
    );
    expect(focusedFrame?.dataset.cardFocused).toBe("true");
    expect(unfocusedFrame?.dataset.cardFocused).toBe("false");
    expect(focusedFrame?.style.borderColor).not.toBe(
      unfocusedFrame?.style.borderColor,
    );

    const remainingCards = cards(1);
    await act(async () => {
      root.render(
        <CanvasSurface
          ref={controller}
          cards={remainingCards}
        />,
      );
      await Promise.resolve();
    });
    expect(canvas?.dataset.canvasMode).toBe("canvas");
    expect(canvas?.dataset.focusedCard).toBe("");
    expect(controller.current?.getFocusedCard()).toBeNull();
  });

  it("sender frisk-spawn-forberedelse synkront til terminalkortet", async () => {
    const controller = createRef<CanvasController>();
    const received: string[] = [];
    const listener = (event: Event) => {
      received.push((event as CustomEvent<{ name: string }>).detail.name);
    };
    window.addEventListener("talminal:prepare-fresh-spawn", listener);
    try {
      await act(async () => {
        root.render(<CanvasSurface ref={controller} cards={cards(1)} />);
        await Promise.resolve();
      });
      controller.current?.prepareFreshSpawn(1);
      expect(received).toEqual(["card-1"]);
    } finally {
      window.removeEventListener("talminal:prepare-fresh-spawn", listener);
    }
  });

  it("holder fuldskærms-headeren uden for native webview-bounds", async () => {
    await act(async () => {
      root.render(<CanvasSurface cards={[browserCard()]} />);
      await Promise.resolve();
    });

    const frame = host.querySelector<HTMLElement>("[data-card-frame]");
    const body = host.querySelector<HTMLElement>("[data-browser-card-body]");
    const fullscreenButton = host.querySelector<HTMLButtonElement>(
      "[data-browser-fullscreen-action]",
    );
    expect(frame).not.toBeNull();
    expect(body).not.toBeNull();
    expect(fullscreenButton).not.toBeNull();

    body!.getBoundingClientRect = () =>
      ({
        x: 0,
        y: 119,
        left: 0,
        top: 119,
        right: 900,
        bottom: 700,
        width: 900,
        height: 581,
        toJSON: () => ({}),
      }) as DOMRect;
    vi.mocked(invoke).mockClear();

    await act(async () => {
      fullscreenButton!.dispatchEvent(
        new MouseEvent("click", { bubbles: true }),
      );
      await new Promise<void>((resolve) =>
        requestAnimationFrame(() => resolve()),
      );
    });

    expect(frame!.dataset.browserFullscreenFrame).toBe("true");
    expect(frame!.style.position).toBe("absolute");
    // TOP_ZONE_CLEARANCE 58: titlebar 50 + 8px luft. Var 84 indtil 2026-08-03,
    // hvor et efterladt HUD-chip-baand blev fjernet (se responsiveLayout.ts).
    expect(frame!.style.top).toBe("58px");
    expect(frame!.style.bottom).toBe("0px");
    expect(fullscreenButton!.getAttribute("aria-label")).toBe(
      "Afslut fuldskærm",
    );
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_browser_bounds", {
      name: "card-1",
      x: 0,
      y: 119,
      w: 900,
      h: 581,
    });
  });

  it("løfter OGSÅ chat-kortet ud af gridden i fuldskærm", async () => {
    // Fuldskaerms-framen var gated paa isBrowserCard: chat-kortets knap satte
    // state (ikonet skiftede), men framen blev liggende i sin grid-celle, saa
    // knappen saa doed ud. Fuldskaerm hoerer til framen, ikke til korttypen.
    await act(async () => {
      root.render(<CanvasSurface cards={[chatCard()]} />);
      await Promise.resolve();
    });

    const frame = host.querySelector<HTMLElement>("[data-card-frame]");
    const button = host.querySelector<HTMLButtonElement>(
      "[data-chat-fullscreen-action]",
    );
    expect(frame).not.toBeNull();
    expect(button).not.toBeNull();
    expect(frame!.style.position).toBe("relative");

    vi.mocked(invoke).mockClear();
    await act(async () => {
      button!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });

    expect(frame!.dataset.cardFullscreen).toBe("true");
    expect(frame!.style.position).toBe("absolute");
    // TOP_ZONE_CLEARANCE 58: titlebar 50 + 8px luft. Var 84 indtil 2026-08-03,
    // hvor et efterladt HUD-chip-baand blev fjernet (se responsiveLayout.ts).
    expect(frame!.style.top).toBe("58px");
    expect(frame!.style.bottom).toBe("0px");
    expect(button!.getAttribute("aria-label")).toBe("Afslut fuldskærm");
    // Navnet er IKKE et browser-kort, og det er meningen: fuldskaerms-gaten
    // Rust-side skjuler da ALLE browser-webviews, saa ingen native child
    // tegner over det fuldskaerms-chat-kort.
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("set_browser_fullscreen", {
      name: "card-1",
    });

    await act(async () => {
      button!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });
    expect(frame!.dataset.cardFullscreen).toBe("false");
    expect(frame!.style.position).toBe("relative");
  });

  it("markerer stadig browser-fuldskærm med sin egen frame-attribut", async () => {
    await act(async () => {
      root.render(<CanvasSurface cards={[browserCard()]} />);
      await Promise.resolve();
    });
    const frame = host.querySelector<HTMLElement>("[data-card-frame]");
    await act(async () => {
      host
        .querySelector<HTMLButtonElement>("[data-browser-fullscreen-action]")!
        .dispatchEvent(new MouseEvent("click", { bubbles: true }));
      await Promise.resolve();
    });
    // Chat-kortet maa ikke arve browser-attributten, og browser-kortet maa
    // ikke miste den: bounds-rapporten og §8a-testene hænger paa den.
    expect(frame!.dataset.browserFullscreenFrame).toBe("true");
    expect(frame!.dataset.cardFullscreen).toBe("true");
  });

  it("observerer browser-kortets body-element for indholds-skift (fejl-strip)", async () => {
    await act(async () => {
      root.render(<CanvasSurface cards={[browserCard()]} />);
      await Promise.resolve();
    });
    const body = host.querySelector<HTMLElement>("[data-browser-card-body]");
    expect(body).not.toBeNull();
    // Indhold i kortet (fx BrowserCards fejl-strip) ændrer body-rekten uden
    // at rodens størrelse ændres — uden en observer på selve body-elementet
    // genrapporteres bounds aldrig, og det native webview-barn dækker den
    // nye DOM ved sin gamle rekt.
    const boundsObserver = ResizeObserverMock.instances.find((instance) =>
      instance.observed.includes(body!),
    );
    expect(boundsObserver).toBeDefined();
    // Låst antagelse andetsteds i denne fil: instances[0] er rootSize-
    // observeren og må ikke begynde at dække body-elementer.
    expect(ResizeObserverMock.instances[0].observed).not.toContain(body!);
  });
});
