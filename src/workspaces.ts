/**
 * Ren logik for workspace-rail'en — sortering, synlighed, etiketter og den
 * degraderende indlaesning. Ingen DOM, ingen Tauri-import: modulet er testbart
 * uden begge dele, praecis som `contextHud.ts` og `nudge.ts` (spec §5.2).
 */

/**
 * Spejler Rust-DTO'en fra Task 8 (`workspaces::WorkspaceSummary`) felt for felt.
 * Kontrakten er frosset i integrator-kendelsernes afsnit B — planens
 * TS-interface manglede `root`, som Rust-siden har og som rail'en lover i
 * `title` paa trunkerede navne.
 */
export interface WorkspaceSummary {
  slug: string;
  name: string;
  path_hint: string | null;
  /** Fuld rod-sti. Vises i `title` paa trunkerede navne. */
  root: string | null;
  state: "stopped" | "starting" | "running" | "failed";
  cards: number;
  running_cards: number;
  attention: boolean;
  attention_kind: "none" | "done_unread" | "needs_you";
  is_active: boolean;
  hidden: boolean;
  defect: boolean;
}

/** Backenden skjuler ingen information — filtreringen er frontendens valg,
 *  så "Vis skjulte" ikke kræver en ny round-trip. Rækkefølgen er backendens
 *  (total sortering, Task 5) og må ikke omsorteres her.
 *
 *  **Den AKTIVE post er undtaget fra filteret.** `hidden` og `is_active` er to
 *  uafhængige felter, og default-workspacet skjules af en UDLEDT regel
 *  (`listing.rs::skjul_default_naar_rigtige_findes`) så snart ét rigtigt
 *  projekt findes — også når det er default ejeren faktisk står i. Uden
 *  undtagelsen kunne det workspace han kigger på være helt væk fra rail'en:
 *  ingen række markeret, intet `aria-current` for skærmlæseren, og den eneste
 *  vej tilbage til hans egne kort gik gennem "Vis skjulte" + "Hent frem" uden
 *  at noget antydede det.
 *
 *  Undtagelsen hører HER og ikke et lag dybere: `listing::list` kender hverken
 *  eller skal kende aktivitet, og `summaries` kunne kun løse det som
 *  `hidden = e.hidden && !is_active` — hvorefter rækkens knap ville skifte fra
 *  "Hent frem" til "Fjern", så brugeren ikke længere kunne pinne posten. */
export function visibleWorkspaces(
  all: WorkspaceSummary[],
  showHidden: boolean,
): WorkspaceSummary[] {
  return showHidden ? all : all.filter((w) => !w.hidden || w.is_active);
}

export function rowLabel(w: WorkspaceSummary): { title: string; subtitle: string | null } {
  if (w.defect) {
    // Et defekt project.json har intet laesbart navn; backenden sender sluggen
    // som `name`, og undertitlen fortaeller hvorfor posten ser sadan ud.
    return { title: w.name, subtitle: "kan ikke laeses" };
  }
  return { title: w.name, subtitle: w.path_hint };
}

/** Event-payloaden fra `workspaces-changed` baerer listen selv. Er den ikke en
 *  liste (aeldre/uventet form), returneres null, saa kalderen kan hente den
 *  frem for at rydde rail'en paa et gaet. */
export function workspaceListFrom(payload: unknown): WorkspaceSummary[] | null {
  return Array.isArray(payload) ? (payload as WorkspaceSummary[]) : null;
}

/** Praefikset backenden afviser med, naar en lukning rammer et levende
 *  workspace med koerende kort (`workspaces/commands.rs`). */
export const KRAEVER_BEKRAEFTELSE = "kraever_bekraeftelse";

/**
 * Laeser antallet af koerende kort ud af backendens afvisning.
 *
 * BEGGE destruktive veje — rail'ens ✕ (`request_close_workspace`) og "Fjern fra
 * listen" (`set_workspace_hidden`) — fejler LUKKET: de returnerer
 * `Err("kraever_bekraeftelse:<n>")` FOER de goer noget som helst.
 *
 * **Hvorfor gaten ligger i backenden og ikke her:** rail'ens liste kommer fra
 * `workspaces-changed`, som udsendes paa badge-kadencen (1 s). Startede et kort
 * i det sekund, ville frontenden se 0 koerende kort og lukke et workspace med en
 * levende agent-session uden at spoerge. Tallet skal laeses paa
 * beslutningstidspunktet, i den proces der ejer sandheden — og praefikset baerer
 * det med tilbage, saa dialogen viser det RIGTIGE antal uden en ekstra
 * round-trip.
 *
 * `null` betyder "det her er ikke en bekraeftelses-afvisning" — enhver anden
 * fejl, og ogsaa et fremtidigt fejlformat vi ikke kan laese. Kalderen falder da
 * tilbage til den almindelige notits. Vi gaetter ALDRIG paa et tal: en dialog
 * der lyver om hvor meget der bliver stoppet, er vaerre end en notits.
 */
export function confirmationDemand(err: unknown): number | null {
  const text = (
    typeof err === "string"
      ? err
      : err instanceof Error
        ? err.message
        : String(err ?? "")
  ).trim();
  const match = new RegExp(`^${KRAEVER_BEKRAEFTELSE}:(\\d+)$`).exec(text);
  if (match === null) return null;
  const running = Number(match[1]);
  // 0 er ikke en gyldig anmodning: backenden afviser kun naar der ER kort at
  // stoppe, og "Der kører 0 kort" er ikke et spoergsmaal.
  return Number.isSafeInteger(running) && running > 0 ? running : null;
}

/**
 * Henter listen og degraderer til tom ved enhver fejl.
 *
 * Rail'en monteres FOER Task 8 lander, saa `invoke("list_workspaces")` afvises
 * af Tauri med "unknown command". Det maa koste en tom rail, ikke en faeldet
 * app (integrator-kendelse C.T10). Naar T8 er landet daekker samme vej et
 * transient backend-udfald.
 */
export async function loadWorkspaces(
  call: () => Promise<unknown>,
): Promise<WorkspaceSummary[]> {
  try {
    return workspaceListFrom(await call()) ?? [];
  } catch {
    return [];
  }
}
