import type { CSSProperties, ReactNode } from "react";

/** Rail-bredden. Eksponeres også som CSS-variablen --rail-width, så overlays
 *  der bliver ved med at være fixed kan regne med den. */
export const RAIL_WIDTH = 208;

/*
 * Hvorfor en shell og ikke bare en sibling til CanvasSurface:
 * App'ens rod var `position: fixed; inset: 0`, og topbar, dock, UsageHud og
 * Orb var fixed mod HELE viewporten. En sibling indsnævrer ingenting — de lag
 * ville flyde hen over rail'en. Shellen giver rail og canvas hver sin flex-zone,
 * og viewport-zonen er `position: relative`, så alt inde i den kan skifte fra
 * `fixed` til `absolute` og dermed blive indelukket i sin egen zone.
 *
 * Ét lag bliver bevidst FIXED: `voice/CastLayer.tsx`. Det tegner viewport-
 * relative getBoundingClientRect-punkter (CastLayer.tsx:52, :66, :80-82) ind i
 * en SVG hvis origo ER laget selv (:271-277). Flyttes lagets origo 208 px, mens
 * punkterne stadig måles mod vinduet, lander stemme-strålen 208 px forkert.
 * Laget er aria-hidden + pointerEvents:none og dækker hele vinduet, så det
 * generer hverken rail'en eller musen.
 */
const styles: Record<string, CSSProperties> = {
  shell: {
    position: "fixed",
    inset: 0,
    display: "flex",
    overflow: "hidden",
  },
  // Longhand-flex, ikke shorthand: husets stil (WindowControls.tsx:63) og
  // det eneste der er stabilt at asserte på i happy-dom, som normaliserer
  // `none` → "0 0 auto" og `1` → "1 1 0%".
  rail: {
    width: `${RAIL_WIDTH}px`,
    flex: "0 0 auto",
    position: "relative",
    overflow: "hidden",
  },
  viewport: { flex: "1 1 0%", position: "relative", overflow: "hidden" },
};

export function AppShell({ rail, children }: { rail: ReactNode; children: ReactNode }) {
  return (
    <div
      data-app-shell
      style={{
        ...styles.shell,
        // Betinget, ikke fast: uden rail findes zonen ikke, og et lag der
        // regner `calc(100vw - var(--rail-width))` ville blive 208 px for
        // smalt i en flade der fylder hele vinduet (Task 9-review, punkt b).
        ["--rail-width" as string]: rail ? `${RAIL_WIDTH}px` : "0px",
      }}
    >
      {rail ? <div style={styles.rail}>{rail}</div> : null}
      <div style={styles.viewport}>{children}</div>
    </div>
  );
}

export default AppShell;
