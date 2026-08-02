/**
 * @vitest-environment happy-dom
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { AppShell, RAIL_WIDTH } from "./AppShell";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  // Repo-præcedens i ALLE DOM-render-tests (UsageHud.render.test.tsx:125-130):
  // uden flaget støjer React 19's act() med console.error i hver test.
  (
    globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("AppShell", () => {
  it("giver rail og viewport hver sin zone", () => {
    act(() => {
      root.render(
        <AppShell rail={<div data-rail>rail</div>}>
          <div data-viewport-barn>canvas</div>
        </AppShell>,
      );
    });

    const shell = container.querySelector<HTMLElement>("[data-app-shell]")!;
    expect(shell.style.display).toBe("flex");
    // Uden fixed+inset:0 kollapser shellen til hoejde 0 (App'ens rod har ingen
    // egen hoejde), canvas bliver usynligt — og alle oevrige asserts her ville
    // stadig vaere groenne. Derfor staar de to eksplicit.
    expect(shell.style.position).toBe("fixed");
    // parseFloat, ikke "0px": happy-dom ^20 serialiserer nul-laengder uden
    // enhed ("0"). En manglende egenskab giver "" → NaN og fejler stadig.
    expect(parseFloat(shell.style.inset)).toBe(0);

    const rail = container.querySelector<HTMLElement>("[data-rail]")!.parentElement!;
    expect(rail.style.width).toBe(`${RAIL_WIDTH}px`);
    // Longhand, ikke shorthand: happy-dom ^20 normaliserer `flex: none` til
    // "0 0 auto" ved tilbagelæsning, så en shorthand-assert ville være rød mod
    // en korrekt implementation. Longhand er også husets stil
    // (WindowControls.tsx:63).
    expect(rail.style.flex).toBe("0 0 auto");

    // Viewporten er position:relative, så børn med inset:0 begrænses til DEN
    // og ikke til hele vinduet. Det er hele pointen med tasken.
    const viewport = container.querySelector<HTMLElement>("[data-viewport-barn]")!.parentElement!;
    expect(viewport.style.position).toBe("relative");
    expect(viewport.style.flex).toBe("1 1 0%");
  });

  it("saetter --rail-width saa overlays kan regne med bredden", () => {
    act(() => {
      root.render(
        <AppShell rail={<div data-rail>r</div>}>
          <div>c</div>
        </AppShell>,
      );
    });
    const shell = container.querySelector<HTMLElement>("[data-app-shell]")!;
    expect(shell.style.getPropertyValue("--rail-width")).toBe(`${RAIL_WIDTH}px`);
  });

  it("uden rail fylder viewporten hele bredden", () => {
    act(() => {
      root.render(
        <AppShell rail={null}>
          <div data-viewport-barn>c</div>
        </AppShell>,
      );
    });
    expect(container.querySelector("[data-rail]")).toBeNull();

    const shell = container.querySelector<HTMLElement>("[data-app-shell]")!;
    // Rail-zonen findes ikke, saa bredden er 0 — ellers ville et lag der
    // regner med variablen trække 208 px fra i en flade uden rail.
    expect(shell.style.getPropertyValue("--rail-width")).toBe("0px");
    expect(shell.children.length).toBe(1);
    const viewport = container.querySelector<HTMLElement>("[data-viewport-barn]")!.parentElement!;
    expect(viewport).toBe(shell.children[0]);
    expect(viewport.style.flex).toBe("1 1 0%");
  });
});
