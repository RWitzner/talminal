// Task 9: ren tast-arbitration — planens normative tabel (TAST-raekkerne).
//
// | Input         | canvas-mode | type-mode        |
// | Shift+Esc     | "canvas"    | "exit-type-mode" | (sendes IKKE til terminalen)
// | Esc (alene)   | "canvas"    | "terminal"       | (ALTID — CC-interrupt, laast)
// | alle oevrige  | "canvas"    | "terminal"       |
//
// Exit-kombinationen er PRAECIS Shift+Esc: Ctrl-/Alt-varianter gaar til
// terminalen (konservativt — aed aldrig input CC kunne ville have). Planens
// spec-afvigelse: default-exit er Shift+Esc (ikke spec'ens dobbelt-Esc, som
// kolliderer med CC's Esc-Esc-binding); konfigurerbarhed kommer med settings
// (Task 14) — denne funktion er arbitrationens sandhed.

export type Mode = "canvas" | "type";

export interface KeyLike {
  key: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
}

export function routeKey(
  mode: Mode,
  ev: KeyLike,
): "terminal" | "canvas" | "exit-type-mode" {
  if (mode === "canvas") {
    // Canvas-hotkeys (ingen i v0) — aldrig terminal.
    return "canvas";
  }
  if (ev.key === "Escape" && ev.shiftKey && !ev.ctrlKey && !ev.altKey) {
    return "exit-type-mode";
  }
  return "terminal";
}
