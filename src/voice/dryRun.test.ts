import { describe, expect, it, vi } from "vitest";
import { createDryRunDispatch } from "./dryRun";

const cards = [1, 2, 3, 5].map((number) => ({ number }));

describe("createDryRunDispatch", () => {
  const getProject = async () => ({
    root: "C:\\projekter\\demo",
    name: "demo",
  });

  it("kører resolveren men udfører ingen action for fokuseret send_prompt", async () => {
    const tauriAction = vi.fn();
    const dispatch = createDryRunDispatch({
      getCards: () => cards,
      getFocusedCard: () => 2,
      getProject,
      onAction: tauriAction,
    });

    const result = await dispatch(
      { kind: "send_prompt", card: null, text: "Kør testene" },
      "Kør testene",
    );

    expect(result).toMatchObject({
      ok: true,
      kind: "send_prompt",
      card: 2,
      dry_run: true,
      action_count: 0,
    });
    expect(tauriAction).not.toHaveBeenCalled();
  });

  it("resolver hele close-batchen og bevarer action_count=0", async () => {
    const dispatch = createDryRunDispatch({
      getCards: () => cards,
      getFocusedCard: () => null,
      getProject,
    });

    await expect(
      dispatch({ kind: "close_cards", cards: [3, 1] }, "Luk kort tre og kort et"),
    ).resolves.toMatchObject({
      ok: true,
      cards: [3, 1],
      dry_run: true,
      action_count: 0,
    });
  });

  it("capture_create_resolver_projekt_rod", async () => {
    const dispatch = createDryRunDispatch({
      getCards: () => cards,
      getFocusedCard: () => null,
      getProject,
    });
    await expect(
      dispatch({ kind: "new_card", count: 4 }, "Åbn 4 terminaler"),
    ).resolves.toMatchObject({
      ok: true,
      kind: "new_card",
      cwd: "C:\\projekter\\demo",
      dry_run: true,
      action_count: 0,
    });
  });

  it("logger resolver-afvisning uden fallback-action", async () => {
    const dispatch = createDryRunDispatch({
      getCards: () => cards,
      getFocusedCard: () => null,
      getProject,
    });

    await expect(
      dispatch({ kind: "restart_card", card: 9 }, "Genstart kort ni"),
    ).resolves.toMatchObject({
      ok: false,
      code: "no_such_card",
      dry_run: true,
      action_count: 0,
    });
  });
});
