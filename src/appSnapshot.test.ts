import { describe, expect, it } from "vitest";
import { loadAppSnapshot } from "./appSnapshot";

const workspace = { settings: {} };
const cardList = [{ number: 1, name: "card-1" }];

function callFor(impl: Record<string, () => Promise<unknown>>) {
  return <T,>(cmd: string): Promise<T> => {
    const fn = impl[cmd];
    if (!fn) return Promise.reject(new Error(`uventet kommando: ${cmd}`));
    return fn() as Promise<T>;
  };
}

describe("loadAppSnapshot", () => {
  it("starter project-opslaget parallelt med kerne-snapshottet", async () => {
    let releaseCards!: () => void;
    const cardsPending = new Promise<void>((resolve) => {
      releaseCards = resolve;
    });
    let projectStarted = false;

    const loading = loadAppSnapshot(
      callFor({
        list_cards: async () => {
          await cardsPending;
          return cardList;
        },
        get_workspace: async () => workspace,
        get_project: async () => {
          projectStarted = true;
          return { root: "C:\\p", name: "demo" };
        },
      }),
    );

    expect(projectStarted).toBe(true);
    releaseCards();
    await expect(loading).resolves.toMatchObject({
      project: { name: "demo" },
    });
  });

  it("leverer kort og workspace selv når get_project fejler", async () => {
    const snapshot = await loadAppSnapshot(
      callFor({
        list_cards: async () => cardList,
        get_workspace: async () => workspace,
        get_project: async () => {
          throw new Error("ingen project-info");
        },
      }),
    );
    expect(snapshot.cards).toBe(cardList);
    expect(snapshot.workspace).toBe(workspace);
    expect(snapshot.project).toBeNull();
  });

  it("fejler når kerne-loadet (list_cards) fejler", async () => {
    await expect(
      loadAppSnapshot(
        callFor({
          list_cards: async () => {
            throw new Error("boom");
          },
          get_workspace: async () => workspace,
          get_project: async () => ({ root: "C:\\p", name: "p" }),
        }),
      ),
    ).rejects.toThrow("boom");
  });

  it("leverer project-info når kaldet lykkes", async () => {
    const snapshot = await loadAppSnapshot(
      callFor({
        list_cards: async () => cardList,
        get_workspace: async () => workspace,
        get_project: async () => ({ root: "C:\\p", name: "demo" }),
      }),
    );
    expect(snapshot.project).toEqual({ root: "C:\\p", name: "demo" });
  });
});
