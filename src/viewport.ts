// Task 9: ren world/screen-matematik — INGEN React, ingen DOM.
//
// Konvention (spejler Rust-sidens workspace::Viewport-default {0,0,1}):
// (v.x, v.y) er WORLD-punktet der ligger i skaerm-origo (canvas-fladens
// oeverste venstre hjoerne); zoom er skaerm-px pr. world-enhed.
//
//   screen = (world - v.xy) * zoom        world = screen / zoom + v.xy
//
// CSS-koblingen (CanvasSurface, laast transform-invariant): containeren faar
// `translate(-v.x*zoom, -v.y*zoom) scale(zoom)` med transform-origin 0 0 —
// kort ligger paa faste world-koordinater og roeres aldrig enkeltvis.

export interface Viewport {
  x: number;
  y: number;
  zoom: number;
}

export function worldToScreen(
  v: Viewport,
  p: { x: number; y: number },
): { x: number; y: number } {
  return { x: (p.x - v.x) * v.zoom, y: (p.y - v.y) * v.zoom };
}

export function screenToWorld(
  v: Viewport,
  p: { x: number; y: number },
): { x: number; y: number } {
  return { x: p.x / v.zoom + v.x, y: p.y / v.zoom + v.y };
}

/** World-punktet i midten af det synlige view: (v.x, v.y) er hjoernet, saa
 *  centret er hjoernet plus et halvt vindue omregnet til world-enheder. */
export function viewCenterWorld(
  v: Viewport,
  sizePx: { w: number; h: number },
): { x: number; y: number } {
  return { x: v.x + sizePx.w / (2 * v.zoom), y: v.y + sizePx.h / (2 * v.zoom) };
}

/** Zoom med skaermpunktet som fixpunkt: world-punktet under cursoren bliver
 *  staaende paa praecis samme skaermposition. Ingen clamping her — det er en
 *  ren funktion; kalderen clamper sin EFFEKTIVE faktor foer kaldet. */
export function zoomAround(
  v: Viewport,
  screenPoint: { x: number; y: number },
  factor: number,
): Viewport {
  const anchor = screenToWorld(v, screenPoint);
  const zoom = v.zoom * factor;
  return {
    x: anchor.x - screenPoint.x / zoom,
    y: anchor.y - screenPoint.y / zoom,
    zoom,
  };
}
