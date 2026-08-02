import { describe, expect, it, vi } from "vitest";
import {
  confirmationDemand,
  loadWorkspaces,
  rowLabel,
  visibleWorkspaces,
  workspaceListFrom,
  type WorkspaceSummary,
} from "./workspaces";

const base: WorkspaceSummary = {
  slug: "a-1111",
  name: "alpha",
  path_hint: null,
  // `root` staar i Rust-DTO'en og er det rail'en lover i `title` paa trunkerede
  // navne (integrator-kendelse afsnit B — planens TS-interface manglede feltet).
  root: null,
  state: "stopped",
  cards: 0,
  running_cards: 0,
  attention: false,
  attention_kind: "none",
  is_active: false,
  hidden: false,
  defect: false,
};

describe("visibleWorkspaces", () => {
  it("skjuler hidden-poster som standard", () => {
    const alle = [base, { ...base, slug: "b-2222", name: "beta", hidden: true }];
    expect(visibleWorkspaces(alle, false).map((w) => w.slug)).toEqual(["a-1111"]);
    expect(visibleWorkspaces(alle, true).map((w) => w.slug)).toEqual(["a-1111", "b-2222"]);
  });

  it("bevarer backendens raekkefoelge", () => {
    const alle = [{ ...base, slug: "z-9999" }, base];
    expect(visibleWorkspaces(alle, false).map((w) => w.slug)).toEqual(["z-9999", "a-1111"]);
  });

  // SLUTREVIEW B4. `hidden` og `is_active` er to UAFHAENGIGE felter i DTO'en, og
  // default-workspacet skjules af en UDLEDT regel (listing.rs'
  // `skjul_default_naar_rigtige_findes`) saa snart ét rigtigt projekt findes —
  // ogsaa naar det er default ejeren faktisk staar i. Fem veje fører dertil:
  // last_project peger paa en mappe der er flyttet/fjernet/korrupt, en rollback
  // efter en fejlet launch, eller en overdragelse til den default-proces der
  // stadig lever skjult. Filtreredes posten vaek, stod ejeren med sine kort paa
  // skaermen og "default" i titelbjaelken, mens rail'en viste hans OEVRIGE
  // projekter uden at en eneste raekke var markeret — og `aria-current`
  // (WorkspaceRail.tsx) sad inde i netop den raekke der var fjernet, saa
  // skaermlaeser-brugeren fik ingenting. Spec gør `aria-current` til et
  // ACCEPTANCEKRAV, ikke pynt.
  it("en skjult men AKTIV post filtreres ikke vaek", () => {
    const alle: WorkspaceSummary[] = [
      base,
      {
        ...base,
        slug: "default",
        name: "default",
        state: "running",
        hidden: true,
        is_active: true,
      },
    ];
    expect(visibleWorkspaces(alle, false).map((w) => w.slug)).toEqual([
      "a-1111",
      "default",
    ]);
    // Posten baerer stadig `hidden` uaendret igennem. Det er hele grunden til at
    // rettelsen hoerer HER og ikke i `summaries` som `hidden = e.hidden &&
    // !is_active`: dér ville raekkens knap skifte fra "Hent frem" til "Fjern",
    // og brugeren kunne ikke pinne posten tilbage i listen.
    expect(visibleWorkspaces(alle, false)[1].hidden).toBe(true);
  });

  // Modstykket: en skjult post der IKKE er aktiv filtreres stadig vaek — ellers
  // ville undtagelsen ovenfor bare vaere "vis alt".
  it("en skjult INAKTIV post filtreres stadig vaek", () => {
    const alle = [base, { ...base, slug: "b-2222", hidden: true, is_active: false }];
    expect(visibleWorkspaces(alle, false).map((w) => w.slug)).toEqual(["a-1111"]);
  });
});

describe("rowLabel", () => {
  it("viser stisuffiks som undertitel ved navnekollision", () => {
    expect(rowLabel({ ...base, path_hint: "alpha\\repos\\bridgemind" }).subtitle)
      .toBe("alpha\\repos\\bridgemind");
  });
  it("uden kollision er der ingen undertitel", () => {
    expect(rowLabel(base).subtitle).toBeNull();
  });
  it("defekt post navngives med sin slug og markeres", () => {
    const d = rowLabel({ ...base, defect: true, name: "a-1111" });
    expect(d.title).toBe("a-1111");
    expect(d.subtitle).toBe("kan ikke laeses");
  });
});

// Task 8 er ikke landet naar rail'en tages i brug: `list_workspaces` findes
// ikke, og Tauri afviser ukendte commands. Rail'en skal degradere til en tom
// liste, ikke faelde appen (integrator-kendelse C.T10). Naar T8 lander er
// praecis den samme vej ogsaa vaernet mod en midlertidig backend-fejl.
describe("loadWorkspaces", () => {
  it("degraderer til tom liste naar commanden ikke findes", async () => {
    const kald = vi.fn().mockRejectedValue(
      new Error("Command list_workspaces not found"),
    );
    await expect(loadWorkspaces(kald)).resolves.toEqual([]);
  });

  it("leverer backendens liste uaendret igennem", async () => {
    const liste = [base, { ...base, slug: "b-2222" }];
    await expect(loadWorkspaces(async () => liste)).resolves.toEqual(liste);
  });

  it("et svar der ikke er en liste behandles som tomt", async () => {
    await expect(loadWorkspaces(async () => ({ slug: "a" }))).resolves.toEqual([]);
  });
});

describe("workspaceListFrom", () => {
  it("tager event-payloaden naar den er en liste", () => {
    expect(workspaceListFrom([base])).toEqual([base]);
  });
  it("giver null for alt andet, saa kalderen kan hente selv", () => {
    expect(workspaceListFrom(undefined)).toBeNull();
    expect(workspaceListFrom({ workspaces: [base] })).toBeNull();
  });
});

// "Fjern fra listen" er en DOED KNAP hvis appen ikke laeser det her praefiks:
// `set_workspace_hidden` fejler lukket paa et koerende workspace, og uden
// parsingen faar brugeren en intetsigende notits og der sker intet.
describe("confirmationDemand", () => {
  it("laeser antallet af koerende kort ud af backendens afvisning", () => {
    expect(confirmationDemand("kraever_bekraeftelse:3")).toBe(3);
    // Tauri afviser med den raa streng; en Error er den anden form vejen kan
    // tage (test-doubles, en fremtidig indpakning).
    expect(confirmationDemand(new Error("kraever_bekraeftelse:12"))).toBe(12);
  });

  it("giver null for enhver anden fejl, saa notitsen tager over", () => {
    expect(confirmationDemand("ukendt workspace: a-1")).toBeNull();
    expect(confirmationDemand(new Error("unknown command"))).toBeNull();
    expect(confirmationDemand(undefined)).toBeNull();
    expect(confirmationDemand(null)).toBeNull();
  });

  // Vi gaetter ALDRIG paa et tal. Aendrer fejlformatet sig, skal vejen falde
  // tilbage til notitsen — ikke aabne en dialog der lyver om hvad der stoppes.
  it("giver null naar formatet ikke kan laeses", () => {
    expect(confirmationDemand("kraever_bekraeftelse")).toBeNull();
    expect(confirmationDemand("kraever_bekraeftelse:")).toBeNull();
    expect(confirmationDemand("kraever_bekraeftelse:mange")).toBeNull();
    expect(confirmationDemand("kraever_bekraeftelse:-2")).toBeNull();
    // 0 er ikke et spoergsmaal: backenden afviser kun naar der ER noget at stoppe.
    expect(confirmationDemand("kraever_bekraeftelse:0")).toBeNull();
  });
});
