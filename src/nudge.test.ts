// Task 8 — nudgeRepaint: 1:1-spejl af Card.tsx' reload-hydrerings-moenster
// (fit.fit() foerst, derefter TO resize_pty-invokes: cols-1 saa cols — en
// AEGTE pty-geometri-aendring tvinger CC's alt-screen-fulde repaint, FUND 5).
// Card.tsx' inline-kopi bestaar indtil Task 10 udskifter den og baerer
// regressionsvagten.

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Terminal } from "@xterm/xterm";
import type { FitAddon } from "@xterm/addon-fit";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { nudgeRepaint } from "./nudge";

function fakeTerm(cols: number, rows: number): Terminal {
  return { cols, rows } as unknown as Terminal;
}

function fakeFit(onFit?: () => void): FitAddon {
  return { fit: vi.fn(() => onFit?.()) } as unknown as FitAddon;
}

beforeEach(() => {
  invoke.mockReset();
  invoke.mockResolvedValue(undefined);
});

describe("nudgeRepaint (Card.tsx-reload-moensteret 1:1)", () => {
  it("kalder resize_pty med (name, cols-1) og DEREFTER (name, cols)", async () => {
    await nudgeRepaint("card-3", fakeTerm(120, 30), fakeFit());
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(invoke).toHaveBeenNthCalledWith(1, "resize_pty", {
      name: "card-3",
      cols: 119,
      rows: 30,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "resize_pty", {
      name: "card-3",
      cols: 120,
      rows: 30,
    });
  });

  it("synkroniserer xterm-side fit FOER pty-nudgen (cols/rows skal vaere sande)", async () => {
    const order: string[] = [];
    invoke.mockImplementation(async (_cmd: unknown, args: unknown) => {
      order.push(`invoke:${(args as { cols: number }).cols}`);
    });
    await nudgeRepaint("card-3", fakeTerm(120, 30), fakeFit(() => order.push("fit")));
    expect(order).toEqual(["fit", "invoke:119", "invoke:120"]);
  });

  it("gulver shrink-benet paa 2 cols (Card.tsx' Math.max-vaern)", async () => {
    await nudgeRepaint("card-7", fakeTerm(2, 10), fakeFit());
    expect(invoke).toHaveBeenNthCalledWith(1, "resize_pty", {
      name: "card-7",
      cols: 2,
      rows: 10,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "resize_pty", {
      name: "card-7",
      cols: 2,
      rows: 10,
    });
  });

  it("er best effort: en afvist invoke maa ikke kaste", async () => {
    invoke.mockRejectedValue(new Error("card not running: card-9"));
    await expect(nudgeRepaint("card-9", fakeTerm(80, 24), fakeFit())).resolves.toBeUndefined();
    // try/catch-moensteret fra Card.tsx: foerste fejl afbryder parret.
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("er idempotent: hvert kald er samme shrink/grow-par (synligheds-re-entry)", async () => {
    const term = fakeTerm(100, 40);
    const fit = fakeFit();
    await nudgeRepaint("card-5", term, fit);
    await nudgeRepaint("card-5", term, fit);
    expect(invoke).toHaveBeenCalledTimes(4);
    expect(invoke).toHaveBeenNthCalledWith(3, "resize_pty", {
      name: "card-5",
      cols: 99,
      rows: 40,
    });
    expect(invoke).toHaveBeenNthCalledWith(4, "resize_pty", {
      name: "card-5",
      cols: 100,
      rows: 40,
    });
  });
});
