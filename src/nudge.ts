// Task 8: ekstraheret repaint-nudge — 1:1-spejl af Card.tsx' reload-
// hydrerings-sti (fit.fit() + to resize_pty-invokes). Efter reattach eller
// synligheds-re-entry er xterm tom (alt-screen; normal-buffer HAR
// scrollback, men en frisk Terminal er tom uanset hvad) og
// pty-geometrien uaendret — CC's fulde repaint-burst foelger kun en REEL
// geometri-aendring (spike-FUND 5). Derfor en aegte shrink/grow-cyklus:
// (cols-1, rows) og DEREFTER (cols, rows). fit.fit() koerer FOERST, saa
// term.cols/rows er sande for containeren foer nudgen (xterm-side sync).
// Idempotent: hvert kald er samme par; best effort (fejl sluges, og foerste
// fejl afbryder parret — praecis som Card.tsx' try/catch).
//
// NB (fil-lease): Card.tsx' inline-kopi BESTAAR indtil Task 10 udskifter
// den med dette modul og baerer regressionsvagten.

import { invoke } from "@tauri-apps/api/core";
import type { Terminal } from "@xterm/xterm";
import type { FitAddon } from "@xterm/addon-fit";

export async function nudgeRepaint(
  name: string,
  term: Terminal,
  fit: FitAddon,
): Promise<void> {
  fit.fit();
  try {
    await invoke("resize_pty", {
      name,
      cols: Math.max(term.cols - 1, 2),
      rows: term.rows,
    });
    await invoke("resize_pty", { name, cols: term.cols, rows: term.rows });
  } catch {
    /* repaint-nudge er best effort (som Card.tsx-reload-stien) */
  }
}
