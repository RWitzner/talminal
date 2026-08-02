// Central occlusion-gate for browser-webviews (spec §4).
//
// WebView2-børnene lever OVER canvas-DOM'et; når et modalt lag (spawn-dialog,
// hud-panel, …) skal ses, må webviewsene skjules. Flere lag kan kræve
// occlusion samtidig, så et modul-globalt Set tæller aktive årsager: Rust
// invokes KUN på tom↔ikke-tom-transitioner, ikke pr. årsag. Det holder
// set_browser_occlusion-trafikken minimal og idempotent.

import { invoke } from "@tauri-apps/api/core";

let apply: (occluded: boolean) => void = (occluded) => {
  void invoke("set_browser_occlusion", { occluded }).catch(() => undefined);
};
const reasons = new Set<string>();

export function setOcclusionReason(reason: string, active: boolean): void {
  const wasOccluded = reasons.size > 0;
  if (active) reasons.add(reason);
  else reasons.delete(reason);
  const isOccluded = reasons.size > 0;
  if (wasOccluded !== isOccluded) apply(isOccluded);
}

export function _resetForTest(fn?: (occluded: boolean) => void): void {
  reasons.clear();
  if (fn) apply = fn;
}
