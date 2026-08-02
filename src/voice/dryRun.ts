import {
  resolveTarget,
  resolveTargets,
  type VoiceIntent,
} from "./intents";

export type DryRunResult =
  | {
      ok: true;
      kind: VoiceIntent["kind"];
      message: string;
      card?: number;
      cards?: number[];
      cwd?: string;
      dry_run: true;
      action_count: 0;
    }
  | {
      ok: false;
      code: string;
      message: string;
      dry_run: true;
      action_count: 0;
    };

export function createDryRunDispatch(dependencies: {
  getCards(): Array<{ number: number }>;
  getFocusedCard(): number | null;
  getProject(): Promise<{ root: string; name: string }>;
  onAction?(): void;
}): (intent: VoiceIntent, rawTranscript?: string) => Promise<DryRunResult> {
  const success = (
    intent: VoiceIntent,
    extra: Partial<Extract<DryRunResult, { ok: true }>> = {},
  ): DryRunResult => ({
    ok: true,
    kind: intent.kind,
    message: `Dry-run: ${intent.kind} resolveret; ingen handling udført`,
    dry_run: true,
    action_count: 0,
    ...extra,
  });
  const failure = (code: string): DryRunResult => ({
    ok: false,
    code,
    message: `Dry-run resolver blokerede: ${code}`,
    dry_run: true,
    action_count: 0,
  });

  return async (intent, rawTranscript) => {
    void rawTranscript;
    const cards = dependencies.getCards();
    if (intent.kind === "close_cards") {
      const resolved = resolveTargets(intent, cards);
      return resolved.ok
        ? success(intent, { cards: resolved.cards })
        : failure(resolved.reason);
    }

    if (intent.kind === "new_card") {
      const project = await dependencies.getProject();
      return success(intent, {
        cwd: project.root,
        message: `Dry-run: new_card i ${project.root}; ingen handling udført`,
      });
    }

    if (intent.kind === "send_prompt" || intent.kind === "restart_card") {
      const resolved = resolveTarget(
        intent,
        dependencies.getFocusedCard(),
        cards,
      );
      if (!resolved.ok) return failure(resolved.reason);
      return success(intent, { card: resolved.card });
    }

    return success(intent); // open_browser
  };
}
