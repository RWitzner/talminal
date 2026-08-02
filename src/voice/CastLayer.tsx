import { useEffect, useRef, type CSSProperties } from "react";
import {
  CAST_BURST_INITIAL,
  nextCastDelay,
  subscribeCast,
  type CastBurstState,
  type CastLanding,
} from "./cast";
import {
  castApproxLength,
  castControlPoint,
  castPathD,
  type CastPoint,
} from "./castGeometry";

/**
 * Cast-straalen (spec 2026-07-22): overlay der tegner en lysstraale fra orben
 * til maalkortet ved send_prompt/new_card/open_browser. Ren pynt — laget
 * abonnerer paa cast-bussen og maa aldrig kaste eller forsinke noget.
 * Orbens DOM roeres ALDRIG: flare-gloeden er lagets eget element over
 * orb-positionen ([data-voice-orb] bruges kun til positions-opslag).
 */

const MOUNT_WAIT_MS = 500; // spawn: vent paa at kortet dukker op i DOM'en
const DRAW_MS = 200; // straalens dash-sweep
const FADE_DELAY_MS = 200; // fade starter naar sweepet er faerdigt
const FADE_MS = 240; // udbraending — straalen er vaek ved ~450 ms
const GLOW_MS = 650; // nedslags-gloed paa kortrammen
const FLARE_MS = 340; // affyrings-flare over orben

const SVG_NS = "http://www.w3.org/2000/svg";

export function CastLayer() {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const svgRef = useRef<SVGSVGElement | null>(null);
  const burstRef = useRef<CastBurstState>(CAST_BURST_INITIAL);
  const castSeqRef = useRef(0);

  useEffect(() => {
    const timeouts = new Set<number>();
    const rafs = new Set<number>();

    const later = (fn: () => void, ms: number) => {
      const id = window.setTimeout(() => {
        timeouts.delete(id);
        fn();
      }, ms);
      timeouts.add(id);
    };

    function centerOf(el: Element): CastPoint {
      const r = el.getBoundingClientRect();
      return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
    }

    /**
     * Nedslagspunktet (ejer-beslutning 2026-07-22 aften): send_prompt lander
     * i terminalens TEKSTFELT, ikke midt paa vinduet. xterms helper-textarea
     * foelger cursoren (= CC's promptboks); ligger den uden for kortkroppen
     * (redraw-mellemtilstand), falder vi tilbage til bundstriben, hvor CC
     * altid tegner promptboksen.
     */
    function beamTarget(card: Element, landing: CastLanding): CastPoint {
      if (landing !== "prompt") return centerOf(card);
      const body = card.querySelector("[data-card-body]") ?? card;
      const bodyRect = body.getBoundingClientRect();
      const field = card.querySelector(".xterm-helper-textarea");
      if (field) {
        const p = centerOf(field);
        if (
          p.x >= bodyRect.left &&
          p.x <= bodyRect.right &&
          p.y >= bodyRect.top &&
          p.y <= bodyRect.bottom
        ) {
          return p;
        }
      }
      return {
        x: bodyRect.left + bodyRect.width / 2,
        y: bodyRect.bottom - Math.min(48, bodyRect.height * 0.15),
      };
    }

    function cardGlow(target: Element) {
      target.animate(
        [
          {
            boxShadow: "0 0 0 0 rgba(143,167,224,0)",
            borderColor: "rgba(151,184,218,0.2)",
          },
          {
            boxShadow: "0 0 24px 3px rgba(143,167,224,0.5)",
            borderColor: "rgba(167,192,240,0.85)",
            offset: 0.22,
          },
          {
            boxShadow: "0 0 0 0 rgba(143,167,224,0)",
            borderColor: "rgba(151,184,218,0.2)",
          },
        ],
        { duration: GLOW_MS, easing: "ease-out" },
      );
    }

    function flare(host: HTMLDivElement, p: CastPoint) {
      const el = document.createElement("span");
      el.style.cssText =
        `position:fixed;left:${p.x}px;top:${p.y}px;width:72px;height:72px;` +
        "margin:-36px 0 0 -36px;border-radius:50%;pointer-events:none;" +
        "background:radial-gradient(circle, rgba(233,185,142,0.5) 0%, " +
        "rgba(143,167,224,0.35) 45%, transparent 75%);filter:blur(6px);";
      host.append(el);
      el.animate(
        [
          { opacity: 0, transform: "scale(0.5)" },
          { opacity: 1, transform: "scale(1.15)", offset: 0.3 },
          { opacity: 0, transform: "scale(1.3)" },
        ],
        { duration: FLARE_MS, easing: "ease-out" },
      );
      // Oprydning via timer frem for onfinish — deterministisk ogsaa hvor
      // WAAPI er stubbet (happy-dom).
      later(() => el.remove(), FLARE_MS + 100);
    }

    function beam(
      svg: SVGSVGElement,
      p0: CastPoint,
      cp: CastPoint,
      p2: CastPoint,
    ) {
      castSeqRef.current += 1;
      const gradId = `cast-grad-${castSeqRef.current}`;
      const defs = document.createElementNS(SVG_NS, "defs");
      const grad = document.createElementNS(SVG_NS, "linearGradient");
      grad.setAttribute("id", gradId);
      grad.setAttribute("gradientUnits", "userSpaceOnUse");
      grad.setAttribute("x1", String(p0.x));
      grad.setAttribute("y1", String(p0.y));
      grad.setAttribute("x2", String(p2.x));
      grad.setAttribute("y2", String(p2.y));
      const stops: Array<[string, string]> = [
        ["0", "#e9b98e"],
        ["0.55", "#8fa7e0"],
        ["1", "#e6efff"],
      ];
      for (const [offset, color] of stops) {
        const stop = document.createElementNS(SVG_NS, "stop");
        stop.setAttribute("offset", offset);
        stop.setAttribute("stop-color", color);
        grad.append(stop);
      }
      defs.append(grad);

      const d = castPathD(p0, cp, p2);
      const glow = document.createElementNS(SVG_NS, "path");
      glow.setAttribute("d", d);
      glow.setAttribute("stroke", `url(#${gradId})`);
      glow.setAttribute("stroke-width", "6");
      glow.setAttribute("fill", "none");
      glow.setAttribute("stroke-linecap", "round");
      glow.style.filter = "blur(3px)";
      const core = document.createElementNS(SVG_NS, "path");
      core.setAttribute("d", d);
      core.setAttribute("stroke", "#dfe9ff");
      core.setAttribute("stroke-width", "2");
      core.setAttribute("fill", "none");
      core.setAttribute("stroke-linecap", "round");
      svg.append(defs, glow, core);

      const length = castApproxLength(p0, cp, p2);
      for (const path of [glow, core]) {
        path.style.strokeDasharray = String(length);
        path.style.strokeDashoffset = String(length);
        path.animate(
          [{ strokeDashoffset: length }, { strokeDashoffset: 0 }],
          { duration: DRAW_MS, easing: "ease-out", fill: "forwards" },
        );
        path.animate([{ opacity: 1 }, { opacity: 0 }], {
          duration: FADE_MS,
          delay: FADE_DELAY_MS,
          easing: "ease-out",
          fill: "forwards",
        });
      }
      later(() => {
        defs.remove();
        glow.remove();
        core.remove();
      }, FADE_DELAY_MS + FADE_MS + 60);
    }

    function launch(target: Element, landing: CastLanding) {
      const host = hostRef.current;
      const svg = svgRef.current;
      if (!host || !svg) return;
      if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
        cardGlow(target);
        return;
      }
      const orb = document.querySelector("[data-voice-orb]");
      if (!orb) return;
      const p0 = centerOf(orb);
      const p2 = beamTarget(target, landing);
      const cp = castControlPoint(p0, p2);
      flare(host, p0);
      beam(svg, p0, cp, p2);
      later(() => cardGlow(target), DRAW_MS);
    }

    function fire(card: number, landing: CastLanding) {
      // Spawn-vejen: kortet er skabt i Rust men maaske ikke mountet endnu.
      // Vent pr. frame, max MOUNT_WAIT_MS — derefter tavst drop (pynt).
      const deadline = performance.now() + MOUNT_WAIT_MS;
      const attempt = () => {
        const target = document.querySelector(
          `[data-card-number="${card}"]`,
        );
        if (target) {
          launch(target, landing);
          return;
        }
        if (performance.now() >= deadline) return;
        const id = window.requestAnimationFrame(() => {
          rafs.delete(id);
          attempt();
        });
        rafs.add(id);
      };
      attempt();
    }

    const unsubscribe = subscribeCast(({ card, landing }) => {
      const next = nextCastDelay(burstRef.current, performance.now());
      burstRef.current = next.state;
      if (next.delay === 0) fire(card, landing);
      else later(() => fire(card, landing), next.delay);
    });

    return () => {
      unsubscribe();
      for (const id of timeouts) window.clearTimeout(id);
      for (const id of rafs) window.cancelAnimationFrame(id);
      svgRef.current?.replaceChildren();
      const host = hostRef.current;
      if (host) {
        for (const child of [...host.children]) {
          if (child !== svgRef.current) child.remove();
        }
      }
    };
  }, []);

  return (
    <div ref={hostRef} aria-hidden style={layerStyle}>
      <svg ref={svgRef} style={svgStyle} />
    </div>
  );
}

const layerStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  pointerEvents: "none",
  // Over kortene (CanvasSurface topper ved 20), under orb-dokken (40) saa
  // straalen udspringer BAG orben.
  zIndex: 39,
};

const svgStyle: CSSProperties = {
  position: "absolute",
  inset: 0,
  width: "100%",
  height: "100%",
  overflow: "visible",
};

export default CastLayer;
