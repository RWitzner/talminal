// Liquid glass for hele canvas-baggrunden.
//
// Wallpaperet tegnes igen i et separat fuldskaermslag og forvrides med et
// lavfrekvent SVG displacement-map. Ovenpaa ligger smoke, lys og en svag
// indvendig vignette. Terminal-gridet renderes over dette lag og forbliver
// derfor skarpt; glasset er canvasets atmosfaere, ikke terminalernes materiale.

import { type CSSProperties, type ReactElement } from "react";

const CANVAS_FILTER_ID = "canvas-liquid-refraction";

/** Faelles frost-materiale til HUD, notice og settings. */
export const FROSTED_BACKDROP =
  "blur(24px) saturate(160%) brightness(1.08)";

export interface CanvasLiquidGlassProps {
  wallpaperUrl?: string | null;
}

export function CanvasLiquidGlass({
  wallpaperUrl,
}: CanvasLiquidGlassProps): ReactElement {
  const hasWallpaper = typeof wallpaperUrl === "string" && wallpaperUrl.length > 0;

  return (
    <div
      data-canvas-liquid-glass
      data-canvas-liquid-mode={hasWallpaper ? "wallpaper" : "liquid-only"}
      style={{
        ...styles.root,
        ...(hasWallpaper ? {} : styles.liquidOnlyRoot),
      }}
      aria-hidden="true"
    >
      <svg
        width={0}
        height={0}
        style={styles.defs}
        focusable="false"
        aria-hidden="true"
      >
        <defs>
          <filter
            id={CANVAS_FILTER_ID}
            x="-8%"
            y="-8%"
            width="116%"
            height="116%"
            colorInterpolationFilters="sRGB"
          >
            <feTurbulence
              type="fractalNoise"
              baseFrequency="0.004 0.009"
              numOctaves={2}
              seed={17}
              stitchTiles="stitch"
              result="liquidNoise"
            />
            <feGaussianBlur
              in="liquidNoise"
              stdDeviation={3.2}
              result="softLiquidNoise"
            />
            <feDisplacementMap
              in="SourceGraphic"
              in2="softLiquidNoise"
              scale={32}
              xChannelSelector="R"
              yChannelSelector="G"
            />
          </filter>
        </defs>
      </svg>

      <div data-canvas-liquid-refraction style={styles.refraction}>
        {hasWallpaper ? (
          <div
            data-canvas-liquid-wallpaper
            style={{
              ...styles.wallpaper,
              backgroundImage: `url("${wallpaperUrl}")`,
            }}
          />
        ) : (
          <div data-canvas-liquid-material style={styles.liquidOnlyMaterial} />
        )}
      </div>
      <div data-canvas-liquid-smoke style={styles.smoke} />
      <div data-canvas-liquid-highlight style={styles.highlight} />
    </div>
  );
}

const styles: Record<string, CSSProperties> = {
  root: {
    position: "absolute",
    inset: 0,
    zIndex: 0,
    overflow: "hidden",
    pointerEvents: "none",
    background: "#020816",
    boxShadow:
      "inset 0 0 180px rgba(0, 3, 12, 0.58), inset 0 0 36px rgba(132, 203, 255, 0.06)",
  },
  liquidOnlyRoot: {
    background: "transparent",
    boxShadow:
      "inset 0 0 180px rgba(0, 3, 12, 0.3), inset 0 0 36px rgba(132, 203, 255, 0.05)",
  },
  defs: {
    position: "absolute",
    pointerEvents: "none",
  },
  refraction: {
    position: "absolute",
    inset: -28,
    filter: `url("#${CANVAS_FILTER_ID}")`,
    transform: "scale(1.035)",
    transformOrigin: "center",
  },
  wallpaper: {
    position: "absolute",
    inset: 0,
    backgroundPosition: "center",
    backgroundRepeat: "no-repeat",
    backgroundSize: "cover",
    filter: "blur(7px) saturate(138%) brightness(0.88)",
    opacity: 0.96,
  },
  liquidOnlyMaterial: {
    position: "absolute",
    inset: 0,
    background: [
      "radial-gradient(ellipse 92% 70% at 22% 8%, rgba(116, 195, 255, 0.13), transparent 64%)",
      "radial-gradient(ellipse 78% 82% at 92% 76%, rgba(75, 106, 190, 0.1), transparent 68%)",
      "linear-gradient(132deg, rgba(5, 14, 30, 0.2), rgba(2, 8, 20, 0.08))",
    ].join(", "),
  },
  smoke: {
    position: "absolute",
    inset: 0,
    background: [
      "radial-gradient(ellipse 115% 72% at 42% -16%, rgba(184, 224, 255, 0.15), transparent 58%)",
      "radial-gradient(ellipse 72% 90% at 108% 48%, rgba(42, 112, 188, 0.15), transparent 66%)",
      "linear-gradient(124deg, rgba(1, 6, 17, 0.3), rgba(2, 10, 24, 0.56))",
    ].join(", "),
    backdropFilter: "blur(10px) saturate(136%)",
    WebkitBackdropFilter: "blur(10px) saturate(136%)",
  },
  highlight: {
    position: "absolute",
    inset: "-8% -5%",
    background: [
      "radial-gradient(ellipse 58% 13% at 28% 12%, rgba(225, 244, 255, 0.16), transparent 72%)",
      "radial-gradient(ellipse 46% 11% at 76% 72%, rgba(69, 173, 255, 0.13), transparent 76%)",
      "linear-gradient(112deg, transparent 12%, rgba(194, 229, 255, 0.045) 34%, transparent 52%)",
    ].join(", "),
    filter: "blur(18px)",
    mixBlendMode: "screen",
    opacity: 0.72,
    transform: "rotate(-1.5deg)",
  },
};
