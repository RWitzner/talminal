/**
 * Frontendens fallback-hotkeys.
 *
 * Disse er KOPIER af `workspace.rs`' `DEFAULT_PTT_HOTKEY`,
 * `DEFAULT_EXIT_TYPE_MODE_HOTKEY` og `DEFAULT_DICTATION_HOTKEY` — den kopi er
 * uundgåelig, fordi de to sider
 * ikke deler et sprog, og Rust ejer den rigtige værdi. Men der behøver kun
 * være ÉN kopi på TS-siden: `App.tsx` og `Settings.tsx` havde hver sin literal
 * af samme streng.
 *
 * Bruges kun som fallback hvis `get_workspace` fejler (fx workspace endnu ikke
 * loaded) — den levende værdi kommer altid over wiren.
 *
 * Modulet er bevidst uden imports: `Settings.tsx` er sin egen bundle-chunk, så
 * en konstant lånt DERFRA ville trække hele indstillingspanelet ind i
 * hovedbundtet.
 */
export const DEFAULT_PTT_HOTKEY = "CmdOrCtrl+Shift+Space";
export const DEFAULT_EXIT_TYPE_MODE_HOTKEY = "Shift+Escape";
export const DEFAULT_DICTATION_HOTKEY = "CmdOrCtrl+Shift+KeyD";
