import { useEffect, useRef, useState, type CSSProperties } from "react";
import { ORB_DOCK_CLEARANCE } from "../canvas/responsiveLayout";
import type { HudSessionState } from "./Hud";
import {
  ORB_LEVEL_LERP,
  orbLevelTarget,
  orbModeForSession,
  orbVisualAttr,
  triggerErrorFlash,
  type OrbFlashState,
} from "./orbState";

/**
 * Voice-orben (spec 2026-07-20) — port af Redapting-companionens lytte-orb,
 * permanent synlig i canvas-bundbåndet. Visuel kontrakt som forlægget:
 * data-mode-attributten + --lvl-variablen driver al CSS; komponenten
 * modtager KUN mode/niveau — aldrig transcript, tool-kald eller indhold
 * (samme dataløshed som companion-rendereren).
 */
export function Orb(props: {
  session: HudSessionState;
  errorTick: number;
  sessionLabel: string;
  getMicLevel(): { level: number; at: number };
  getOutputLevel(): number;
}) {
  const shellRef = useRef<HTMLSpanElement | null>(null);
  const flashRef = useRef<OrbFlashState>({ flashUntil: null });
  const levelRef = useRef(0);
  /** Sidst skrevne `--lvl`, saa en uaendret frame ikke roerer CSSOM. */
  const writtenLevelRef = useRef<string | null>(null);
  const propsRef = useRef(props);
  propsRef.current = props;
  const [attr, setAttr] = useState<string>(() =>
    orbModeForSession(props.session),
  );

  // Fejl-blusset er edge-trigget på errorTick (App bumper pr. fejl-patch,
  // så identiske fejltekster blusser hver gang). Mount-værdien blusser ikke.
  const seenTickRef = useRef(props.errorTick);
  useEffect(() => {
    if (props.errorTick === seenTickRef.current) return;
    seenTickRef.current = props.errorTick;
    flashRef.current = triggerErrorFlash(flashRef.current, performance.now());
  }, [props.errorTick]);

  // Niveau-motoren: kører permanent (orben er permanent). Flash-udløb afgøres
  // pr. frame mod flashUntil — ingen setTimeout, så prioritetsreglen
  // (flash > session) bor alene i orbVisualAttr.
  useEffect(() => {
    let raf = 0;
    const tick = () => {
      const now = performance.now();
      const nextAttr = orbVisualAttr(
        propsRef.current.session,
        flashRef.current,
        now,
      );
      setAttr((current) => (current === nextAttr ? current : nextAttr));
      // Kilderne hentes KUN i den tilstand der laeser dem: `orbLevelTarget`
      // roerer `mic` i "listening" og `outputLevel` i "speaking" og smider
      // resten vaek. `getOutputLevel()` er en FFT-laesning plus en
      // 1024-samples RMS-loekke — den koerte 60 gange i sekundet resten af
      // sessionen efter det foerste afspillede klip, for et tal ingen laeste.
      const target = orbLevelTarget({
        attr: nextAttr,
        now,
        mic:
          nextAttr === "listening"
            ? propsRef.current.getMicLevel()
            : { level: 0, at: 0 },
        outputLevel:
          nextAttr === "speaking" ? propsRef.current.getOutputLevel() : 0,
      });
      levelRef.current += (target - levelRef.current) * ORB_LEVEL_LERP;
      // CSSOM-skrivning er ikke gratis; springes over naar den afrundede
      // vaerdi ikke har flyttet sig siden sidste frame.
      const next = levelRef.current.toFixed(3);
      if (next !== writtenLevelRef.current) {
        writtenLevelRef.current = next;
        shellRef.current?.style.setProperty("--lvl", next);
      }
      raf = window.requestAnimationFrame(tick);
    };
    raf = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(raf);
  }, []);

  return (
    <div
      role="status"
      aria-label={`Voice: ${props.sessionLabel}`}
      style={dockStyle}
    >
      <style>{orbCss}</style>
      {/* data-voice-orb: positions-opslag for CastLayer (spec 2026-07-22).
          Kun opslag — laget roerer aldrig orbens DOM. */}
      <span
        ref={shellRef}
        aria-hidden
        className="voice-orb"
        data-mode={attr}
        data-voice-orb
      >
        <span className="voice-orb__react">
          <span className="voice-orb__layer voice-orb__swirl" />
          <span className="voice-orb__layer voice-orb__swirl2" />
          <span className="voice-orb__layer voice-orb__core" />
          <span className="voice-orb__layer voice-orb__error" />
        </span>
      </span>
    </div>
  );
}

// `absolute`, ikke `fixed`: orb-docken monteres inde i App'ens <main>, som er
// canvas-zonens position:relative-boks (App.tsx:981). Med fixed ville
// left:0/right:0 spænde over HELE vinduet, og placeItems:center ville centrere
// orben i forhold til vinduet — dvs. skævt (halvdelen af rail'ens bredde for
// langt til venstre), og docken ville desuden ligge hen over rail'en.
// CastLayer måler orbens position med getBoundingClientRect ([data-voice-orb],
// CastLayer.tsx:202+52) og tegner ind i sit EGET fixed;inset:0-lag. Begge sider
// er dermed vindues-relative på tegnetidspunktet, så strålen følger med orben
// af sig selv — CastLayer skal netop derfor IKKE flyttes.
const dockStyle: CSSProperties = {
  position: "absolute",
  left: 0,
  right: 0,
  bottom: 0,
  height: ORB_DOCK_CLEARANCE,
  display: "grid",
  placeItems: "center",
  pointerEvents: "none",
  zIndex: 40,
};

/* Porteret 1:1 fra companion-orben (Redapting styles.css, låst mockup-design)
   med tre afvigelser: --orb-size er ét justérbart tal, idle er et dæmpet
   udtryk i stedet for companionens skjul/vis-morf, og idle spinner
   langsommere end de aktive tilstande. */
const orbCss = `
  .voice-orb {
    --lvl: 0;
    --orb-size: 56px;
    position: relative;
    width: var(--orb-size);
    height: var(--orb-size);
    transition: opacity 0.4s ease, filter 0.4s ease, transform 0.4s ease;
  }
  .voice-orb[data-mode="idle"] {
    opacity: 0.5;
    filter: saturate(0.75);
    transform: scale(0.92);
  }
  /* Idle roterer ~3x langsommere (ejer-smoke 2026-07-20, skærpet fra 1,6x);
     blob-morfingen beholder sit tempo — det er kun spin der skal falde til
     ro. Første varighed = spin, anden = blob (animation-listens rækkefølge). */
  .voice-orb[data-mode="idle"] .voice-orb__swirl {
    animation-duration: 10s, 5.2s;
  }
  .voice-orb[data-mode="idle"] .voice-orb__swirl2 {
    animation-duration: 16s, 4.1s;
  }
  .voice-orb[data-mode="flash-error"] {
    animation: voice-orb-shake 0.45s ease;
  }
  .voice-orb__react {
    position: absolute;
    inset: 0;
    transform: scale(calc(0.84 + var(--lvl, 0) * 0.26));
  }
  .voice-orb__layer {
    position: absolute;
    inset: 0;
    border-radius: 50%;
  }
  .voice-orb__swirl {
    background: conic-gradient(
      from 0deg,
      #2e4c9a,
      #8fa7e0,
      #6b5a9e,
      #e9b98e,
      #2e4c9a
    );
    filter: blur(7px) saturate(1.3);
    animation:
      voice-orb-spin 3.4s linear infinite,
      voice-orb-blob 5.2s ease-in-out infinite;
    opacity: 0.9;
  }
  .voice-orb__swirl2 {
    background: conic-gradient(
      from 180deg,
      #8fa7e0,
      #2e4c9a,
      #a7c0f0,
      #6b5a9e,
      #8fa7e0
    );
    filter: blur(10px);
    animation:
      voice-orb-spin 5.6s linear infinite reverse,
      voice-orb-blob 4.1s ease-in-out infinite reverse;
    opacity: 0.7;
    inset: 6%;
  }
  .voice-orb__core {
    inset: 12%;
    background: radial-gradient(
      circle,
      rgb(255 255 255 / 0.38) 0%,
      rgb(199 214 244 / 0.25) 50%,
      transparent 80%
    );
    filter: blur(9px);
    transform: scale(calc(0.9 + var(--lvl, 0) * 0.3));
  }
  .voice-orb__error {
    background: radial-gradient(
      circle,
      rgb(190 55 45 / 0.65) 0%,
      rgb(190 55 45 / 0.35) 55%,
      transparent 80%
    );
    filter: blur(6px);
    opacity: 0;
    transition: opacity 0.25s ease;
  }
  .voice-orb[data-mode="flash-error"] .voice-orb__error {
    opacity: 1;
  }
  @keyframes voice-orb-spin {
    to {
      rotate: 360deg;
    }
  }
  @keyframes voice-orb-blob {
    0%,
    100% {
      border-radius: 50% 48% 52% 50% / 49% 51% 49% 51%;
    }
    33% {
      border-radius: 53% 47% 45% 55% / 52% 46% 54% 48%;
    }
    66% {
      border-radius: 46% 54% 55% 45% / 47% 55% 45% 53%;
    }
  }
  @keyframes voice-orb-shake {
    0%,
    100% {
      translate: 0 0;
    }
    25% {
      translate: -3px 0;
    }
    50% {
      translate: 3px 0;
    }
    75% {
      translate: -2px 0;
    }
  }
`;

export default Orb;
