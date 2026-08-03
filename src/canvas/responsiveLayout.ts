export interface CanvasSize {
  width: number;
  height: number;
}

export interface TilePlacement {
  column: number;
  row: number;
  columnSpan: number;
  rowSpan: number;
}

export interface ResponsiveTileLayout {
  columns: number;
  rows: number;
  tiles: TilePlacement[];
}

export interface ResponsiveGridSpacing {
  gap: number;
  padding: number;
}

const TARGET_TERMINAL_ASPECT = 1.45;
const DEFAULT_GRID_GAP = 18;
const DEFAULT_GRID_PADDING = 18;

/** Den globale topbars geometri (App.tsx tegner den ud fra disse). Gridden
 *  OG layout-matematikken skal begge kende clearance — ellers vælges
 *  topologi/spacing mod en flade, der er topbar-højden for stor. */
export const TOPBAR_TOP = 12;
export const TOPBAR_HEIGHT = 38;
export const TOPBAR_CLEARANCE = TOPBAR_TOP + TOPBAR_HEIGHT;

/** Den reserverede topzone. Lig med titlebar-clearance — zonen dækker
 *  trække-bjælken og intet andet.
 *
 *  Historik, fordi tallet har været 34px større i seks uger uden grund:
 *  `905c0e5` (2026-07-20) reserverede et ekstra 26px-bånd, fordi HUD-chippen
 *  dengang sad ØVERST på 58px og lagde sig oven på det øverste højre kort.
 *  Dagen efter flyttede `9609942` usage-baren ned i venstre hjørne
 *  (ORB_DOCK_CLEARANCE nedenfor), og båndet blev aldrig fjernet igen — det
 *  har siden reserveret plads til noget der ikke er der. Verificeret: Hud.tsx
 *  har ingen reference til topbar-konstanterne og positionerer intet i zonen.
 *
 *  Zonen er nu præcis bjælken og intet andet (ejer-retning 2026-08-03:
 *  "det må godt starte lige under" trække-bjælken). Luften mellem bjælken og
 *  det øverste kort kommer alene fra grid-paddingen — de samme 18px som mod
 *  venstre og højre kant, så afstanden er symmetrisk hele vejen rundt.
 *  Vokser tallet igen, skal det være fordi noget FAKTISK tegnes deroppe —
 *  ikke fordi det engang gjorde.
 *
 *  NB: `notice`-banneret (App.tsx) ligger på TOPBAR_CLEARANCE + TOPBAR_TOP og
 *  er ~42px højt, altså 62-104. Det overlapper kortene uanset denne konstant
 *  — det er et `position: absolute`-overlay med zIndex 10, ikke noget gridden
 *  gør plads til. Zonen her er ikke stedet at løse det. */
export const TOP_ZONE_CLEARANCE = TOPBAR_CLEARANCE;

/** Bundbåndet (spec 2026-07-20 §6). Samme disciplin som TOPBAR_CLEARANCE:
 *  både grid-padding og layout-matematikken skal kende båndet, ellers vælges
 *  terminal-topologien mod en for stor flade.
 *
 *  Tallet er båndets HØJESTE beboer, ikke et rundt tal. Målt 2026-08-03:
 *
 *    usage-HUD   `bottom: 24` + ~49px chip-højde  = 73   ← denne bestemmer
 *    voice-orb   56px, `placeItems: center`        = 56  (centreret, 8,5 i top)
 *
 *  Var 88 indtil da, altså 15px slack over den højeste — plus grid-paddingens
 *  18 gav det ~33px ned til HUD'en mod 18px til alle andre kanter. Nu er
 *  afstanden symmetrisk hele vejen rundt.
 *
 *  Vokser HUD-chippen (en tredje række, større font), skal tallet med op.
 *  Sker det ikke, rykker kortene bare tættere på — hverken orb eller HUD
 *  klippes, for begge er `position: absolute` og ligger uden for gridden. */
export const ORB_DOCK_CLEARANCE = 73;

function gcd(left: number, right: number): number {
  let a = left;
  let b = right;
  while (b !== 0) {
    const remainder = a % b;
    a = b;
    b = remainder;
  }
  return a;
}

function lcm(left: number, right: number): number {
  return (left / gcd(left, right)) * right;
}

function balancedRowCounts(count: number, rows: number): number[] {
  const shortRow = Math.floor(count / rows);
  const longerRows = count % rows;
  return Array.from(
    { length: rows },
    (_, row) => shortRow + (row < longerRows ? 1 : 0),
  );
}

function chooseRowCount(count: number, size: CanvasSize): number {
  const width = Math.max(1, size.width);
  const height = Math.max(1, size.height);
  let bestRows = 1;
  let bestScore = Number.POSITIVE_INFINITY;

  for (let rows = 1; rows <= count; rows += 1) {
    const rowCounts = balancedRowCounts(count, rows);
    const rowHeight = height / rows;
    const aspectScores = rowCounts.map((itemsInRow) =>
      Math.abs(
        Math.log((width / itemsInRow / rowHeight) / TARGET_TERMINAL_ASPECT),
      ),
    );
    // Den gennemsnitlige terminalform vælger topologien; max-leddet undgår
    // samtidig én ekstremt bred sidste række.
    const score =
      aspectScores.reduce((sum, value) => sum + value, 0) /
        aspectScores.length +
      Math.max(...aspectScores) * 0.2;

    if (score < bestScore) {
      bestRows = rows;
      bestScore = score;
    }
  }

  return bestRows;
}

function rowLayout(count: number, size: CanvasSize): ResponsiveTileLayout {
  const rows = chooseRowCount(count, size);
  const rowCounts = balancedRowCounts(count, rows);
  const columns = rowCounts.reduce(lcm, 1);
  const tiles: TilePlacement[] = [];

  rowCounts.forEach((itemsInRow, rowIndex) => {
    const columnSpan = columns / itemsInRow;
    for (let item = 0; item < itemsInRow; item += 1) {
      tiles.push({
        column: item * columnSpan + 1,
        row: rowIndex + 1,
        columnSpan,
        rowSpan: 1,
      });
    }
  });

  return { columns, rows, tiles };
}

/**
 * Afledt tile-layout for den synlige canvasflade.
 *
 * Placeringerne er CSS-grid-koordinater, ikke persisteret workspace-geometri.
 * Derfor kan browseren ændre de reelle frame-mål kontinuerligt, når canvaset
 * resizes, uden en update_card_geometry-storm.
 */
export function computeResponsiveTileLayout(
  count: number,
  size: CanvasSize,
): ResponsiveTileLayout {
  if (!Number.isInteger(count) || count < 0) {
    throw new Error("Card count must be a non-negative integer");
  }

  if (count === 0) return { columns: 0, rows: 0, tiles: [] };
  if (count === 1) {
    return {
      columns: 1,
      rows: 1,
      tiles: [{ column: 1, row: 1, columnSpan: 1, rowSpan: 1 }],
    };
  }

  if (count === 2) {
    const horizontal = size.width >= size.height;
    return horizontal
      ? {
          columns: 2,
          rows: 1,
          tiles: [
            { column: 1, row: 1, columnSpan: 1, rowSpan: 1 },
            { column: 2, row: 1, columnSpan: 1, rowSpan: 1 },
          ],
        }
      : {
          columns: 1,
          rows: 2,
          tiles: [
            { column: 1, row: 1, columnSpan: 1, rowSpan: 1 },
            { column: 1, row: 2, columnSpan: 1, rowSpan: 1 },
          ],
        };
  }

  // Produktkravet: ét højt kort til venstre, to stablet til højre.
  if (count === 3) {
    return {
      columns: 2,
      rows: 2,
      tiles: [
        { column: 1, row: 1, columnSpan: 1, rowSpan: 2 },
        { column: 2, row: 1, columnSpan: 1, rowSpan: 1 },
        { column: 2, row: 2, columnSpan: 1, rowSpan: 1 },
      ],
    };
  }

  // Fire kort skal være et stabilt 2x2-grid uanset små aspect-ratio-skift.
  if (count === 4) {
    return {
      columns: 2,
      rows: 2,
      tiles: [
        { column: 1, row: 1, columnSpan: 1, rowSpan: 1 },
        { column: 2, row: 1, columnSpan: 1, rowSpan: 1 },
        { column: 1, row: 2, columnSpan: 1, rowSpan: 1 },
        { column: 2, row: 2, columnSpan: 1, rowSpan: 1 },
      ],
    };
  }

  return rowLayout(count, size);
}

/**
 * Holder spacing inden for den faktiske canvasstørrelse. Det er især vigtigt
 * for balancerede rækker, hvor CSS-gridet kan have flere virtuelle kolonner
 * end synlige kort.
 */
export function computeResponsiveGridSpacing(
  layout: Pick<ResponsiveTileLayout, "columns" | "rows">,
  size: CanvasSize,
  preferred: ResponsiveGridSpacing = {
    gap: DEFAULT_GRID_GAP,
    padding: DEFAULT_GRID_PADDING,
  },
): ResponsiveGridSpacing {
  const width = Math.max(0, size.width);
  const height = Math.max(0, size.height);
  if (layout.columns === 0 || layout.rows === 0) {
    return { gap: 0, padding: Math.max(0, preferred.padding) };
  }

  const padding = Math.max(
    0,
    Math.min(
      preferred.padding,
      (width - Math.min(width, layout.columns)) / 2,
      (height - Math.min(height, layout.rows)) / 2,
    ),
  );
  const usableWidth = Math.max(0, width - 2 * padding);
  const usableHeight = Math.max(0, height - 2 * padding);
  const horizontalLimit =
    layout.columns > 1
      ? Math.max(0, (usableWidth - layout.columns) / (layout.columns - 1))
      : preferred.gap;
  const verticalLimit =
    layout.rows > 1
      ? Math.max(0, (usableHeight - layout.rows) / (layout.rows - 1))
      : preferred.gap;

  return {
    gap: Math.max(0, Math.min(preferred.gap, horizontalLimit, verticalLimit)),
    padding,
  };
}
