// App'ens workspace-load, samlet ét sted så koblingen er testbar.
//
// Kerne-data (list_cards + get_workspace) afgør om canvas kan vises —
// fejler de, fejler hele loadet. get_project er kun topbar-identitet:
// dens fejl må ALDRIG blanke canvas, så den degraderer til null
// (App falder tilbage til "Workspace"-labelen).

import { invoke } from "@tauri-apps/api/core";
import type { CardInfo, WorkspaceResponse } from "./types";

export interface ProjectInfo {
  root: string;
  name: string;
}

export interface AppSnapshot {
  cards: CardInfo[];
  workspace: WorkspaceResponse;
  project: ProjectInfo | null;
}

export async function loadAppSnapshot(
  call: <T>(cmd: string) => Promise<T> = invoke,
): Promise<AppSnapshot> {
  // Alle tre IPC-kald er uafhængige. `get_project` er stadig best-effort, men
  // må ikke ligge som en ekstra sekventiel round-trip efter kerne-snapshottet
  // ved hvert workspace-reveal.
  const [cards, workspace, project] = await Promise.all([
    call<CardInfo[]>("list_cards"),
    call<WorkspaceResponse>("get_workspace"),
    call<ProjectInfo>("get_project").catch(() => null),
  ]);
  return { cards, workspace, project };
}
