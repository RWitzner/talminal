/**
 * @vitest-environment happy-dom
 */

// Luk-bekraeftelsen som PRAESENTATION og som TILGAENGELIGHEDSKONTRAKT.
//
// Spec §5.2 kraever ordret at "bekraeftelsesdialogen tager og genskaber fokus
// korrekt" — det er et acceptancekrav, ikke pynt.
//
// Filen daekker KUN komponenten selv (fix-runde 1, fund 3). `WindowControls`'
// `closeDisabled` bor i `WindowControls.test.tsx`, og App'ens ledningsfoering i
// `App.close.test.tsx`; foer opdelingen laa alle tre her, og fokus-genskabelsen
// blev maalt paa en gren appen aldrig koerer.

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CloseWorkspaceDialog, closeDialogText } from "./CloseWorkspaceDialog";

function actEnvironment(): void {
  (
    globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
}

function key(
  target: EventTarget,
  k: string,
  modifiers: {
    shiftKey?: boolean;
    ctrlKey?: boolean;
    altKey?: boolean;
    metaKey?: boolean;
  } = {},
): void {
  target.dispatchEvent(
    new KeyboardEvent("keydown", { key: k, bubbles: true, ...modifiers }),
  );
}

describe("closeDialogText", () => {
  it("dette vindue og et navngivet workspace faar hver sin tekst", () => {
    const mine = closeDialogText({ workspaceName: null, running: 2 });
    expect(mine.title).toBe("Luk vinduet?");
    expect(mine.body).toContain("2 kort");
    expect(mine.body).toContain("dette vindue");

    const andet = closeDialogText({ workspaceName: "Alpha", running: 1 });
    expect(andet.title).toBe("Luk Alpha?");
    expect(andet.body).toContain("1 kort");
    expect(andet.body).toContain("Alpha");
  });

  it("global exit advarer om alle workspaces og hele Talminal", () => {
    const global = closeDialogText({
      kind: "application",
      workspaces: 3,
      running: 5,
    });
    expect(global.title).toBe("Afslut Talminal?");
    expect(global.body).toContain("Alle 3 åbne workspaces");
    expect(global.body).toContain("5 kørende kort");
    expect(global.body).toContain("Talminal afsluttes helt");
    expect(global.confirmLabel).toBe("Afslut Talminal");
  });
});

describe("CloseWorkspaceDialog", () => {
  let host: HTMLDivElement;
  let root: Root;
  let outside: HTMLButtonElement;

  beforeEach(() => {
    actEnvironment();
    host = document.createElement("div");
    document.body.append(host);
    // Et element uden for dialogen der kan HAVE fokus foer den aabner — ellers
    // kan "genskaber fokus" ikke skelnes fra "gjorde ingenting". Det er
    // samtidig stand-in for rail'ens ✕: det element native tabulering ville
    // lande paa hvis faelden svigtede.
    outside = document.createElement("button");
    outside.textContent = "udenfor";
    document.body.append(outside);
    root = createRoot(host);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    host.remove();
    outside.remove();
  });

  function render(
    request: { workspaceName: string | null; running: number } | null,
    handlers: {
      onConfirm?: () => void;
      onCancel?: () => void;
      restoreFocusTo?: HTMLElement | null;
    } = {},
  ): Promise<void> {
    return act(async () => {
      root.render(
        <CloseWorkspaceDialog
          request={request}
          onConfirm={handlers.onConfirm ?? (() => {})}
          onCancel={handlers.onCancel ?? (() => {})}
          restoreFocusTo={handlers.restoreFocusTo ?? null}
        />,
      );
    });
  }

  const dialog = () => host.querySelector<HTMLElement>("[data-close-dialog]");
  const confirm = () =>
    host.querySelector<HTMLButtonElement>("[data-close-confirm]")!;
  const cancel = () =>
    host.querySelector<HTMLButtonElement>("[data-close-cancel]")!;

  it("uden anmodning tegnes intet", async () => {
    await render(null);
    expect(dialog()).toBeNull();
  });

  it("dette vindue: viser antal koerende kort og to valg", async () => {
    await render({ workspaceName: null, running: 3 });
    const element = dialog()!;
    expect(element.getAttribute("role")).toBe("dialog");
    expect(element.getAttribute("aria-modal")).toBe("true");
    expect(element.textContent).toContain("3 kort");
    expect(confirm()).not.toBeNull();
    expect(cancel()).not.toBeNull();
  });

  it("andet workspace: navnet staar i dialogen", async () => {
    await render({ workspaceName: "Alpha", running: 2 });
    expect(dialog()!.textContent).toContain("Alpha");
    expect(dialog()!.textContent).toContain("2 kort");
  });

  it("tager fokus ved aabning — og laegger den paa Annullér, ikke paa Luk", async () => {
    outside.focus();
    expect(document.activeElement).toBe(outside);
    await render({ workspaceName: null, running: 1 });
    expect(document.activeElement).toBe(cancel());
  });

  // DEN GREN APPEN FAKTISK KOERER (fund 3): App.tsx fanger fokus paa
  // gerningsstedet og sender elementet ind, fordi titelbjaelkens ✕ bliver
  // `disabled` i samme commit som dialogen monteres. Proppen skal derfor VINDE
  // over det element der tilfaeldigvis har fokus naar mount-effekten koerer.
  it("restoreFocusTo vinder over det element der havde fokus ved monteringen", async () => {
    const kalderensValg = document.createElement("button");
    kalderensValg.textContent = "kalderens valg";
    document.body.append(kalderensValg);
    try {
      outside.focus();
      expect(document.activeElement).toBe(outside);
      await render(
        { workspaceName: null, running: 1 },
        { restoreFocusTo: kalderensValg },
      );
      await render(null, { restoreFocusTo: kalderensValg });
      expect(document.activeElement).toBe(kalderensValg);
      expect(document.activeElement).not.toBe(outside);
    } finally {
      kalderensValg.remove();
    }
  });

  it("restoreFocusTo=null falder tilbage paa elementet der havde fokus", async () => {
    outside.focus();
    await render({ workspaceName: null, running: 1 });
    expect(document.activeElement).toBe(cancel());
    await render(null);
    expect(document.activeElement).toBe(outside);
  });

  it("Escape annullerer — og bekraefter ikke", async () => {
    const onCancel = vi.fn();
    const onConfirm = vi.fn();
    await render({ workspaceName: null, running: 1 }, { onCancel, onConfirm });
    await act(async () => key(cancel(), "Escape"));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  // MINOR 6: Shift+Esc er husets LAASTE exit-type-mode-kombination
  // (keyRouting.ts). Dialogen maa ikke stjaele den — arbitrationen ejer den.
  it("Escape MED modifikator annullerer ikke (Shift+Esc er husets exit)", async () => {
    const onCancel = vi.fn();
    await render({ workspaceName: null, running: 1 }, { onCancel });
    await act(async () => key(cancel(), "Escape", { shiftKey: true }));
    await act(async () => key(cancel(), "Escape", { ctrlKey: true }));
    await act(async () => key(cancel(), "Escape", { altKey: true }));
    expect(onCancel).not.toHaveBeenCalled();

    await act(async () => key(cancel(), "Escape"));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("faelder fokus: Tab fra sidste knap gaar til foerste, Shift+Tab modsat", async () => {
    await render({ workspaceName: null, running: 1 });
    const buttons = Array.from(
      dialog()!.querySelectorAll<HTMLButtonElement>("button"),
    );
    expect(buttons.length).toBeGreaterThanOrEqual(2);
    const first = buttons[0];
    const last = buttons[buttons.length - 1];

    last.focus();
    await act(async () => key(last, "Tab"));
    expect(document.activeElement).toBe(first);

    await act(async () => key(first, "Tab", { shiftKey: true }));
    expect(document.activeElement).toBe(last);
  });

  // FUND 1, tilstanden "fokus er faldet UD af dialogen".
  //
  // Sådan opstår den i Chromium: brugeren klikker paa dialogens broedtekst,
  // fokus soeger naermeste fokuserbare forfader, og faldt foer helt ud paa
  // <body>. En React-`onKeyDown` paa panelet ser aldrig det naeste Tab (React
  // delegerer paa #root, som er BARN af body), saa native tabulering flyttede
  // til foerste tabbable i dokumentet — rail'en, der staar FOER <main>.
  // happy-dom har ingen native tabulering, saa testen maaler den PRAECISE
  // mekanisme: at faelden hoerer tasten selv om fokus ikke er i dialogen.
  it("faelden holder ogsaa naar fokus er faldet ud paa <body>", async () => {
    await render({ workspaceName: null, running: 1 });
    (document.activeElement as HTMLElement).blur();
    expect(document.activeElement).toBe(document.body);

    await act(async () => key(document.body, "Tab"));
    expect(dialog()!.contains(document.activeElement)).toBe(true);
    expect(document.activeElement).toBe(cancel());
  });

  it("panelet er selv fokuserbart, saa et klik i broedteksten ikke ryger paa <body>", async () => {
    // Chromium flytter fokus til naermeste fokuserbare forfader ved klik;
    // happy-dom goer det ikke, saa mekanismen pinnes strukturelt.
    // NB: `element.tabIndex` er -1 for ETHVERT ikke-fokuserbart element, saa
    // den property beviser intet. Attributten skal staa der.
    await render({ workspaceName: null, running: 1 });
    expect(dialog()!.getAttribute("tabindex")).toBe("-1");
  });

  it("fokus der lander uden for dialogen hales tilbage med det samme", async () => {
    await render({ workspaceName: null, running: 1 });
    // Stand-in for det rail-✕ native tabulering ville have ramt.
    outside.focus();
    expect(document.activeElement).not.toBe(outside);
    expect(dialog()!.contains(document.activeElement)).toBe(true);
  });

  it("klik paa baggrunden annullerer — klik i panelet goer ikke", async () => {
    const onCancel = vi.fn();
    await render({ workspaceName: null, running: 1 }, { onCancel });
    const backdrop = host.querySelector<HTMLElement>(
      "[data-close-dialog-backdrop]",
    )!;

    await act(async () => {
      dialog()!.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });
    expect(onCancel).not.toHaveBeenCalled();

    await act(async () => {
      backdrop.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("knapperne kalder hver sin handler", async () => {
    const onCancel = vi.fn();
    const onConfirm = vi.fn();
    await render({ workspaceName: null, running: 1 }, { onCancel, onConfirm });
    await act(async () => confirm().click());
    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(onCancel).not.toHaveBeenCalled();

    await act(async () => cancel().click());
    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});
