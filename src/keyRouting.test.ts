// Task 9: normativ tast-arbitration (keyRouting.ts) — planens tabel testes
// udtoemmende, raekke for raekke, for TAST-raekkerne (wheel/drag/klik er
// pointer-arbitration i CanvasSurface og daekkes af Step 3-roegtesten):
//
// | Input             | canvas-mode              | type-mode                       |
// | `Shift+Esc`       | no-op ("canvas")         | "exit-type-mode" (IKKE terminal) |
// | `Esc` (alene)     | no-op ("canvas")         | "terminal" (ALTID — CC-interrupt)|
// | Alle oevrige      | "canvas" (ingen hotkeys) | "terminal"                       |

import { describe, expect, it } from "vitest";
import { routeKey, type Mode } from "./keyRouting";

function ev(
  key: string,
  mods: Partial<{ shiftKey: boolean; ctrlKey: boolean; altKey: boolean }> = {},
) {
  return {
    key,
    shiftKey: mods.shiftKey ?? false,
    ctrlKey: mods.ctrlKey ?? false,
    altKey: mods.altKey ?? false,
  };
}

describe("routeKey — canvas-mode", () => {
  const mode: Mode = "canvas";

  it("Shift+Esc er no-op (ingen exit at forlade)", () => {
    expect(routeKey(mode, ev("Escape", { shiftKey: true }))).toBe("canvas");
  });

  it("Esc alene er no-op", () => {
    expect(routeKey(mode, ev("Escape"))).toBe("canvas");
  });

  it("alle oevrige taster er canvas-hotkeys (ingen i v0) — ALDRIG terminal", () => {
    expect(routeKey(mode, ev("a"))).toBe("canvas");
    expect(routeKey(mode, ev("Enter"))).toBe("canvas");
    expect(routeKey(mode, ev("Tab"))).toBe("canvas");
    expect(routeKey(mode, ev("ArrowUp"))).toBe("canvas");
    expect(routeKey(mode, ev("c", { ctrlKey: true }))).toBe("canvas");
    expect(routeKey(mode, ev("F1"))).toBe("canvas");
    expect(routeKey(mode, ev("A", { shiftKey: true }))).toBe("canvas");
  });
});

describe("routeKey — type-mode", () => {
  const mode: Mode = "type";

  it("Shift+Esc forlader type-mode og sendes IKKE til terminalen", () => {
    expect(routeKey(mode, ev("Escape", { shiftKey: true }))).toBe("exit-type-mode");
  });

  it("Esc alene gaar ALTID til terminalen (CC-interrupt, laast beslutning)", () => {
    expect(routeKey(mode, ev("Escape"))).toBe("terminal");
  });

  it("Esc med andre modifiers end ren Shift er IKKE exit-kombinationen", () => {
    // Kun praecis Shift+Esc er exit; Ctrl/Alt-varianter leveres til
    // terminalen (konservativt: aldrig aede input CC kunne ville have).
    expect(routeKey(mode, ev("Escape", { ctrlKey: true }))).toBe("terminal");
    expect(routeKey(mode, ev("Escape", { altKey: true }))).toBe("terminal");
    expect(routeKey(mode, ev("Escape", { shiftKey: true, ctrlKey: true }))).toBe(
      "terminal",
    );
    expect(routeKey(mode, ev("Escape", { shiftKey: true, altKey: true }))).toBe(
      "terminal",
    );
  });

  it("alle oevrige taster gaar til terminalen", () => {
    expect(routeKey(mode, ev("a"))).toBe("terminal");
    expect(routeKey(mode, ev("A", { shiftKey: true }))).toBe("terminal");
    expect(routeKey(mode, ev("Enter"))).toBe("terminal");
    expect(routeKey(mode, ev("Tab"))).toBe("terminal");
    expect(routeKey(mode, ev("ArrowUp"))).toBe("terminal");
    expect(routeKey(mode, ev("ArrowDown"))).toBe("terminal");
    expect(routeKey(mode, ev("c", { ctrlKey: true }))).toBe("terminal");
    expect(routeKey(mode, ev("d", { ctrlKey: true }))).toBe("terminal");
    expect(routeKey(mode, ev("Backspace"))).toBe("terminal");
    expect(routeKey(mode, ev("F1"))).toBe("terminal");
  });
});
