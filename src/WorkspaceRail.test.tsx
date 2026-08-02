/**
 * @vitest-environment happy-dom
 */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { WorkspaceRail, railCss } from "./WorkspaceRail";
import type { WorkspaceSummary } from "./workspaces";

const w = (over: Partial<WorkspaceSummary> = {}): WorkspaceSummary => ({
  slug: "a-1111",
  name: "alpha",
  path_hint: null,
  // Frossen DTO (integrator-kendelse afsnit B): `root` er den fulde sti.
  root: null,
  state: "running",
  cards: 2,
  running_cards: 1,
  attention: false,
  attention_kind: "none",
  is_active: false,
  hidden: false,
  defect: false,
  ...over,
});

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  // Husets teststil (UsageHud.render.test.tsx:125-130): uden flaget stoejer
  // React 19's act() med console.error i hver test.
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

function tegn(props: Partial<Parameters<typeof WorkspaceRail>[0]> = {}) {
  const alle = {
    workspaces: [w()],
    showHidden: false,
    onActivate: vi.fn(),
    onClose: vi.fn(),
    onToggleHidden: vi.fn(),
    onAdd: vi.fn(),
    ...props,
  };
  act(() => root.render(<WorkspaceRail {...alle} />));
  return alle;
}

/** Selektorerne der erklaerer en given app-region-vaerdi i rail'ens egen CSS.
 *  Testen laeser dem UD af komponenten i stedet for at gentage dem, saa den
 *  maaler kravet ("hvert interaktivt element er no-drag") og ikke en kopi af
 *  implementationen. `-webkit-app-region: drag` matcher ikke `no-drag`, fordi
 *  moenstret kraever kolon+mellemrum umiddelbart foer vaerdien. */
function selectorsFor(css: string, value: "drag" | "no-drag"): string[] {
  const blocks = /([^{}]+)\{([^{}]*)\}/g;
  const out: string[] = [];
  for (let m = blocks.exec(css); m !== null; m = blocks.exec(css)) {
    if (new RegExp(`-webkit-app-region:\\s*${value}\\s*;`).test(m[2])) {
      out.push(m[1].trim());
    }
  }
  return out;
}

const matchesAny = (el: Element, selectors: string[]) =>
  selectors.some((sel) => el.matches(sel));

describe("WorkspaceRail", () => {
  it("markerer aktiv post for skaermlaesere", () => {
    tegn({ workspaces: [w({ is_active: true })] });
    const post = container.querySelector<HTMLElement>("[data-workspace-row='a-1111']")!;
    expect(post.getAttribute("aria-current")).toBe("true");
  });

  it("klik paa den allerede aktive post faar ingen lokal pending-state", () => {
    const { onActivate } = tegn({ workspaces: [w({ is_active: true })] });
    const post = container.querySelector<HTMLElement>("[data-workspace-row='a-1111']")!;
    act(() => post.click());
    // Backenden ejer no-op-gaten: en stale rail må ikke blokere et hurtigt
    // A→B→A-skift. Frontenden undgår kun den lange lokale busy-markering.
    expect(onActivate).toHaveBeenCalledWith("a-1111");
    expect(post.getAttribute("aria-busy")).toBeNull();
  });

  it("done_unread beholder dagens ravgule prik og tekst", () => {
    tegn({ workspaces: [w({ attention: true, attention_kind: "done_unread" })] });
    expect(container.textContent).toMatch(/venter p/i);
    const dot = container.querySelector<HTMLElement>("[data-attention-kind='done_unread']")!;
    expect(dot.style.background).toBe("#e8b046");
  });

  it("needs_you eskalerer prikken og tekstens hast", () => {
    tegn({ workspaces: [w({ attention: true, attention_kind: "needs_you" })] });
    expect(container.textContent).toMatch(/kræver din handling/i);
    const dot = container.querySelector<HTMLElement>("[data-attention-kind='needs_you']")!;
    expect(dot.style.background).toBe("#ff665c");
    expect(dot.style.boxShadow).toContain("14px");
  });

  it("legacy attention true uden kind behandles som needs_you", () => {
    tegn({ workspaces: [w({ attention: true, attention_kind: "none" })] });
    expect(container.textContent).toMatch(/kræver din handling/i);
    expect(container.querySelector("[data-attention-kind='needs_you']")).not.toBeNull();
  });

  it("en summary hvor kind-feltet MANGLER helt eskalerer ogsaa", () => {
    // En aeldre backend sender ikke feltet. Uden `?? \"none\"` i
    // effectiveAttentionKind matcher hverken needs_you- eller done_unread-
    // grenen, og kortet ville falde igennem til state-grenene og staa som
    // "koerer" med normal prik — daarligere end i dag.
    const legacy = w({ attention: true });
    delete (legacy as Partial<WorkspaceSummary>).attention_kind;
    tegn({ workspaces: [legacy] });
    expect(container.textContent).toMatch(/kræver din handling/i);
    expect(container.querySelector("[data-attention-kind='needs_you']")).not.toBeNull();
  });

  it("luk-knappen er en rigtig knap i tab-raekkefoelgen, ikke hover-only", () => {
    const { onClose } = tegn();
    const luk = container.querySelector<HTMLButtonElement>("[data-workspace-close='a-1111']")!;
    // Hover-only ville betyde display:none og tabIndex -1 — så var den utilgængelig.
    expect(luk.tabIndex).not.toBe(-1);
    act(() => luk.click());
    expect(onClose).toHaveBeenCalledWith("a-1111");
  });

  it("klik aktiverer, men en post der allerede starter er en no-op", () => {
    const { onActivate } = tegn({ workspaces: [w({ state: "starting" })] });
    const post = container.querySelector<HTMLElement>("[data-workspace-row='a-1111']")!;
    act(() => post.click());
    expect(onActivate).not.toHaveBeenCalled();
    expect(post.getAttribute("aria-busy")).toBe("true");
  });

  it("klik paa en koerende post aktiverer den", () => {
    const { onActivate } = tegn();
    const post = container.querySelector<HTMLElement>("[data-workspace-row='a-1111']")!;
    act(() => post.click());
    expect(onActivate).toHaveBeenCalledWith("a-1111");
  });

  it("tom liste viser foerste-gangs-opfordringen", () => {
    tegn({ workspaces: [] });
    // Afvigelse fra planens regex (/tilfoej dit foerste projekt/i): appens
    // synlige tekst er rigtigt dansk ("Tilføj dit første projekt"), som i
    // resten af fladen (App.tsx "Indstillinger", WindowControls "Minimér").
    // Moenstret accepterer begge stavemaader, saa kravet er uaendret.
    expect(container.textContent).toMatch(/tilf(ø|oe)j dit f(ø|oe)rste projekt/i);
    const tom = container.querySelector<HTMLElement>("[data-workspace-empty]")!;
    expect(tom.dataset.workspaceEmpty).toBe("first-run");
  });

  it("alt-skjult er ikke foerste gang — beskeden peger paa Vis skjulte", () => {
    // Tre projekter, alle "Fjern"-et. Vaelges tom-tilstanden paa rows.length
    // EFTER filtreringen, staar der "Tilføj dit første projekt" selvom de tre
    // ligger ét klik vaek bag "Vis skjulte".
    tegn({
      workspaces: [
        w({ hidden: true }),
        w({ slug: "b-2222", name: "beta", hidden: true }),
        w({ slug: "c-3333", name: "gamma", hidden: true }),
      ],
    });
    expect(container.querySelector("[data-workspace-row]")).toBeNull();
    const tom = container.querySelector<HTMLElement>("[data-workspace-empty]")!;
    expect(tom.dataset.workspaceEmpty).toBe("all-hidden");
    expect(tom.textContent).not.toMatch(/f(ø|oe)rste projekt/i);
    expect(tom.textContent).toMatch(/vis skjulte/i);
  });

  it("skjulte poster vises kun naar showHidden er sat", () => {
    tegn({ workspaces: [w(), w({ slug: "b-2222", name: "beta", hidden: true })] });
    expect(container.querySelector("[data-workspace-row='b-2222']")).toBeNull();

    tegn({ workspaces: [w(), w({ slug: "b-2222", name: "beta", hidden: true })], showHidden: true });
    expect(container.querySelector("[data-workspace-row='b-2222']")).not.toBeNull();
  });

  // SLUTREVIEW B4. Default-workspacet skjules af en UDLEDT regel saa snart ét
  // rigtigt projekt findes — ogsaa naar opstarten er faldet tilbage til default
  // (last_project peger paa en flyttet mappe, en rollback efter fejlet launch,
  // eller en overdragelse til den default-proces der lever videre skjult). Blev
  // raekken filtreret vaek, sad `aria-current` inde i den, og
  // skaermlaeser-brugeren fik INGEN markering af den post han faktisk staar i —
  // mens spec §5.2 gør `aria-current` til et acceptancekrav.
  it("den AKTIVE post vises og markeres, ogsaa naar den er skjult", () => {
    tegn({
      workspaces: [
        w({ slug: "b-2222", name: "beta" }),
        w({ slug: "default", name: "default", hidden: true, is_active: true }),
      ],
    });
    const post = container.querySelector<HTMLElement>("[data-workspace-row='default']");
    expect(post).not.toBeNull();
    expect(post!.getAttribute("aria-current")).toBe("true");
    // Knappen hedder stadig "Hent frem": posten ER skjult, den er bare synlig
    // fordi den er aktiv. Ellers kunne brugeren ikke pinne den tilbage i listen.
    expect(
      container.querySelector<HTMLButtonElement>("[data-workspace-hide='default']")!.textContent,
    ).toMatch(/hent frem/i);
  });

  // ——— tilgaengelighed og skalering (spec §5.2, acceptancekrav) ———

  it("✕ aktiverer ikke posten den sidder i", () => {
    const { onActivate } = tegn();
    const luk = container.querySelector<HTMLButtonElement>("[data-workspace-close='a-1111']")!;
    act(() => luk.click());
    expect(onActivate).not.toHaveBeenCalled();
  });

  it("Fjern er en knap i tab-raekkefoelgen og skjuler posten", () => {
    const { onToggleHidden } = tegn();
    const fjern = container.querySelector<HTMLButtonElement>("[data-workspace-hide='a-1111']")!;
    expect(fjern.tabIndex).not.toBe(-1);
    expect(fjern.textContent).toMatch(/fjern/i);
    act(() => fjern.click());
    expect(onToggleHidden).toHaveBeenCalledWith("a-1111", true);
  });

  it("en skjult post tilbyder at komme tilbage i stedet for at blive fjernet igen", () => {
    const { onToggleHidden } = tegn({
      workspaces: [w({ hidden: true })],
      showHidden: true,
    });
    const knap = container.querySelector<HTMLButtonElement>("[data-workspace-hide='a-1111']")!;
    act(() => knap.click());
    expect(onToggleHidden).toHaveBeenCalledWith("a-1111", false);
  });

  it("laenge navne trunkeres, men fuld sti staar i title", () => {
    tegn({ workspaces: [w({ name: "et-meget-langt-projektnavn", root: "C:\\r\\alpha" })] });
    const aabn = container.querySelector<HTMLElement>("[data-workspace-open='a-1111']")!;
    expect(aabn.title).toBe("C:\\r\\alpha");
    const navn = container.querySelector<HTMLElement>("[data-workspace-name='a-1111']")!;
    expect(navn.style.overflow).toBe("hidden");
    expect(navn.style.textOverflow).toBe("ellipsis");
    expect(navn.style.whiteSpace).toBe("nowrap");
  });

  it("+ Tilfoej projekt og Vis skjulte er knapper med hver sin handling", () => {
    const { onAdd } = tegn();
    const tilfoej = container.querySelector<HTMLButtonElement>("[data-workspace-add]")!;
    act(() => tilfoej.click());
    expect(onAdd).toHaveBeenCalledTimes(1);

    const vis = container.querySelector<HTMLButtonElement>("[data-workspace-show-hidden]")!;
    expect(vis.getAttribute("aria-pressed")).toBe("false");
    const skift = vi.fn();
    act(() =>
      root.render(
        <WorkspaceRail
          workspaces={[w()]}
          showHidden
          onActivate={vi.fn()}
          onClose={vi.fn()}
          onToggleHidden={vi.fn()}
          onAdd={vi.fn()}
          onShowHiddenChange={skift}
        />,
      ),
    );
    const vis2 = container.querySelector<HTMLButtonElement>("[data-workspace-show-hidden]")!;
    expect(vis2.getAttribute("aria-pressed")).toBe("true");
    act(() => vis2.click());
    expect(skift).toHaveBeenCalledWith(false);
  });

  // Tandhjulet flyttede hertil fra bund-docken 2026-07-29. Rail'en tegner KUN
  // knappen — vinduet bor i canvas-zonen, fordi rail-zonen er 208 px med
  // overflow:hidden og ville klippe enhver flade der foldede ud herfra.
  it("tandhjulet kalder onOpenSettings", () => {
    const aabn = vi.fn();
    tegn({ onOpenSettings: aabn });
    const knap = container.querySelector<HTMLButtonElement>(
      "[data-workspace-settings]",
    )!;
    expect(knap.getAttribute("aria-label")).toBe("Indstillinger");
    expect(knap.getAttribute("aria-expanded")).toBe("false");
    act(() => knap.click());
    expect(aabn).toHaveBeenCalledTimes(1);
  });

  it("tandhjulet melder aaben tilstand videre til skaermlaeseren", () => {
    tegn({ onOpenSettings: vi.fn(), settingsOpen: true });
    const knap = container.querySelector<HTMLButtonElement>(
      "[data-workspace-settings]",
    )!;
    expect(knap.getAttribute("aria-expanded")).toBe("true");
    expect(knap.getAttribute("aria-haspopup")).toBe("dialog");
  });

  // Den frosne prop-kontrakt: rail'en skal kunne monteres uden de valgfri
  // handlere. Uden gaten ville et klik kaste i stedet for at vaere inaktivt.
  it("tandhjulet er inaktivt uden handler", () => {
    tegn();
    const knap = container.querySelector<HTMLButtonElement>(
      "[data-workspace-settings]",
    )!;
    expect(() => act(() => knap.click())).not.toThrow();
  });

  it("50 projekter: alle poster staar, og listen scroller i stedet for at vokse", () => {
    const mange = Array.from({ length: 50 }, (_, i) =>
      w({ slug: `p-${i}`, name: `projekt-${i}` }),
    );
    tegn({ workspaces: mange });
    expect(container.querySelectorAll("[data-workspace-row]").length).toBe(50);
    const liste = container.querySelector<HTMLElement>("[data-workspace-list]")!;
    expect(liste.style.overflowY).toBe("auto");
    // Uden minHeight:0 i en flex-kolonne vokser en scroll-boks i stedet for at
    // scrolle — saa ville post 50 ligge under "+ Tilføj projekt".
    // parseFloat, ikke "0px": happy-dom serialiserer nul-laengder uden enhed
    // ("0"), og en tom streng (= egenskaben mangler) bliver NaN og fejler.
    expect(parseFloat(liste.style.minHeight)).toBe(0);
  });

  it("vinduet kan traekkes i rail'en, og knapperne er no-drag", () => {
    tegn({ workspaces: [w(), w({ slug: "b-2222", hidden: true })], showHidden: true });
    const drag = selectorsFor(railCss, "drag");
    const noDrag = selectorsFor(railCss, "no-drag");
    expect(drag.length).toBeGreaterThan(0);

    // Rail'en SKAL kunne traekkes — det var hele grunden til at drag'en kom ind.
    const baand = container.querySelector<HTMLElement>("[data-workspace-drag-region]")!;
    expect(baand).not.toBeNull();
    expect(matchesAny(baand, drag)).toBe(true);
    // Tauri-attributten som fallback, praecis som den globale topbar.
    expect(baand.getAttribute("data-tauri-drag-region")).toBe("true");

    // En drag-flade aeder klik paa alt den daekker. Knapperne melder sig ud,
    // saa et fremtidigt interaktivt element inde i baandet stadig kan klikkes.
    const knapper = [...container.querySelectorAll("[data-workspace-rail] button")];
    expect(knapper.length).toBeGreaterThan(4);
    for (const el of knapper) {
      expect({
        el: el.outerHTML.slice(0, 80),
        noDrag: matchesAny(el, noDrag),
      }).toEqual({ el: el.outerHTML.slice(0, 80), noDrag: true });
    }
  });

  it("drag-fladen holder sig fri af vinduets resize-kanter", () => {
    tegn();
    const drag = selectorsFor(railCss, "drag");

    // Rod-nav'en er `position:absolute; inset:0` i AppShells rail-zone, som
    // ligger i en `position:fixed; inset:0`-shell med nulstillet body-margin.
    // Var DEN drag-flade, begyndte drag'en paa x=0, y=0 og loeb til vinduets
    // bund: vinduet er `decorations:false, resizable:true`, WebView2 melder
    // HTCAPTION for drag-regioner, og et traek i venstre kant eller i et af de
    // to venstre hjoerner ville FLYTTE vinduet i stedet for at resize det.
    const rail = container.querySelector<HTMLElement>("[data-workspace-rail]")!;
    expect(matchesAny(rail, drag)).toBe(false);

    const baand = container.querySelector<HTMLElement>("[data-workspace-drag-region]")!;
    // Samme maal som den globale topbar (App.tsx styles.topbar: top TOPBAR_TOP,
    // left 18) — husets to drag-flader foelger én regel.
    expect(baand.style.left).toBe("18px");
    expect(baand.style.top).toBe("12px");
    // Fast hoejde, ikke `bottom`: baandet maa ikke naa vinduets nederste kant.
    expect(baand.style.height).toBe("38px");
    expect(baand.style.bottom).toBe("");

    // Og der er ikke SMUTTET en anden drag-flade ind ved siden af.
    const alleDrag = [...container.querySelectorAll("[data-workspace-rail] *")].filter(
      (el) => matchesAny(el, drag),
    );
    expect(alleDrag).toEqual([baand]);
  });

  it("status-teksten er skjult for oejet, ikke bare tilfoejet", () => {
    tegn({ workspaces: [w({ attention: true, attention_kind: "done_unread" })] });
    const sr = container.querySelector<HTMLElement>(
      "[data-workspace-row='a-1111'] .workspace-rail-sr",
    )!;
    expect(sr).not.toBeNull();
    expect(sr.textContent).toMatch(/venter p/i);

    // Klassen alene beviser intet: taber CSS'en reglen, staar "kører, 2 kort,
    // vises nu" som synlig broedtekst under hvert projektnavn i en 208 px
    // kolonne, og textContent-asserterne ovenfor bliver ved med at vaere
    // groenne. Reglen skal staa i rail'ens egen CSS.
    const regel = /\.workspace-rail-sr\s*\{([^}]*)\}/.exec(railCss);
    expect(regel).not.toBeNull();
    const krop = regel![1];
    expect(krop).toMatch(/position\s*:\s*absolute\s*;/);
    expect(krop).toMatch(/width\s*:\s*1px\s*;/);
    expect(krop).toMatch(/height\s*:\s*1px\s*;/);
    expect(krop).toMatch(/overflow\s*:\s*hidden\s*;/);
    expect(krop).toMatch(/clip-path\s*:\s*inset\(50%\)\s*;/);
  });

  it("defekt post markeres, og en stoppet post er daempet", () => {
    tegn({
      workspaces: [
        w({ defect: true, name: "a-1111" }),
        w({ slug: "b-2222", name: "beta", state: "stopped" }),
      ],
    });
    expect(container.textContent).toMatch(/kan ikke laeses/);
    const stoppet = container.querySelector<HTMLElement>("[data-workspace-row='b-2222']")!;
    expect(stoppet.dataset.state).toBe("stopped");

    // `data-state` ekkoer bare fixturen. Daempningen (spec §5.2: "ikke-koerende
    // daempet") skal maales for sig — ellers kan `rowDim` slettes uden at
    // nogen test bliver roed.
    const daempning = parseFloat(stoppet.style.opacity);
    expect(daempning).toBeGreaterThan(0);
    expect(daempning).toBeLessThan(1);

    // Kontrol: en koerende post er IKKE daempet, saa testen ogsaa faelder en
    // daempning der er lagt paa alle rader.
    const koerende = container.querySelector<HTMLElement>("[data-workspace-row='a-1111']")!;
    expect(koerende.dataset.state).toBe("running");
    expect(koerende.style.opacity).toBe("");
  });

  it("en skjult post er ogsaa daempet", () => {
    tegn({ workspaces: [w({ hidden: true })], showHidden: true });
    const skjult = container.querySelector<HTMLElement>("[data-workspace-row='a-1111']")!;
    expect(parseFloat(skjult.style.opacity)).toBeLessThan(1);
  });

  // ——— single-flight (fund 5) ———

  /** Render med de samme handlere paa tvaers af flere kald — `tegn()` laver
   *  friske vi.fn()'er hver gang og kan ikke taelle over to renderinger. */
  function tegnMed(onActivate: ReturnType<typeof vi.fn>) {
    return (workspaces: WorkspaceSummary[]) =>
      act(() =>
        root.render(
          <WorkspaceRail
            workspaces={workspaces}
            showHidden={false}
            onActivate={onActivate}
            onClose={vi.fn()}
            onToggleHidden={vi.fn()}
            onAdd={vi.fn()}
          />,
        ),
      );
  }

  const post = () =>
    container.querySelector<HTMLElement>("[data-workspace-row='a-1111']")!;

  it("dobbeltklik foer backendens ekko sender kun ét activate", () => {
    const onActivate = vi.fn();
    const tegnListe = tegnMed(onActivate);
    // Samme liste-objekt begge renderinger: backenden har ikke svaret endnu.
    const liste = [w({ state: "stopped" })];
    tegnListe(liste);

    act(() => post().click());
    // Posten staar STADIG som "stopped" — `state === "starting"` kommer foerst
    // med `workspaces-changed`, saa den kan ikke sluge det andet klik.
    expect(post().dataset.state).toBe("stopped");
    act(() => post().click());

    expect(onActivate).toHaveBeenCalledTimes(1);
    expect(onActivate).toHaveBeenCalledWith("a-1111");
    // Den lokale laas melder sig ogsaa til skaermlaeseren.
    expect(post().getAttribute("aria-busy")).toBe("true");
    // Men aabne-knappen disables IKKE af det lokale klik: det ville rive
    // fokus vaek fra en tastaturbruger midt i hans egen aktivering.
    const aabn = container.querySelector<HTMLButtonElement>("[data-workspace-open='a-1111']")!;
    expect(aabn.disabled).toBe(false);
  });

  it("laasen aabnes igen naar backenden svarer — ogsaa hvis kaldet fejlede", () => {
    const onActivate = vi.fn();
    const tegnListe = tegnMed(onActivate);
    tegnListe([w({ state: "stopped" })]);

    act(() => post().click());
    expect(onActivate).toHaveBeenCalledTimes(1);

    // Ny liste fra backenden, stadig "stopped": spawn'et blev afvist. Ryddede
    // laasen kun paa "starting"/"running", var posten doed resten af sessionen.
    tegnListe([w({ state: "stopped" })]);
    expect(post().getAttribute("aria-busy")).toBeNull();

    act(() => post().click());
    expect(onActivate).toHaveBeenCalledTimes(2);
  });

  it("naar backenden melder starting, overtager dens egen single-flight", () => {
    const onActivate = vi.fn();
    const tegnListe = tegnMed(onActivate);
    tegnListe([w({ state: "stopped" })]);

    act(() => post().click());
    tegnListe([w({ state: "starting" })]);

    expect(post().getAttribute("aria-busy")).toBe("true");
    act(() => post().click());
    expect(onActivate).toHaveBeenCalledTimes(1);
  });

  it("laasen slipper ogsaa naar der ALDRIG kommer en ny liste", () => {
    // Begge de to rydnings-signaler forudsaetter at backenden svarer med en ny
    // liste. Afvises `activate_workspace` — ukendt slug, spawn-fejl — aendres
    // ingenting i backenden, og badge-tick'et emitter kun VED DIFF. Uden
    // sidste udvej stod raden `aria-busy` og uklikkbar resten af sessionen.
    vi.useFakeTimers();
    try {
      const onActivate = vi.fn();
      const tegnListe = tegnMed(onActivate);
      const liste = [w({ state: "stopped" })];
      tegnListe(liste);

      act(() => post().click());
      expect(post().getAttribute("aria-busy")).toBe("true");

      // Samme liste-objekt, ingen nye events — kun tiden gaar.
      act(() => {
        vi.advanceTimersByTime(8_000);
      });

      expect(post().getAttribute("aria-busy")).toBeNull();
      act(() => post().click());
      expect(onActivate).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });
});
