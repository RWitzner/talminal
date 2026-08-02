// Indstillings-vinduet: centreret kasse i CANVAS-ZONEN med to-panes-navigation.
//
// Afloeser den gamle `SettingsPanel` — et `<details>` i bund-docken, der foldede
// opad fra sit eget hjoerne. Ejer-retning 2026-07-29: baade tandhjulet OG det
// udfoldede panel laa i hoejre side, saa indstillinger aad praecis det hjoerne af
// kortarealet man kiggede paa. Tandhjulet bor nu i rail'ens footer (app-kram
// hoerer til app-navigationen), og fladen er en centreret kasse.
//
// TRE TING DER SER UD SOM DETALJER, MEN ER KRAV:
//
//  1. KOMPONENTEN ER ALTID MONTERET — ogsaa naar vinduet er lukket. To ting
//     haenger paa det: idle-opvarmningen af lazy-chunken (som ellers aldrig
//     ville koere), og `everOpened`, der holder Settings monteret efter foerste
//     aabning, saa en halvt indtastet API-noegle ikke forsvinder naar man lukker.
//     Lukket tilstand er `display: none`, praecis som et sammenklappet
//     `<details>` — IKKE en unmount.
//
//  2. VINDUET ER IKKE MODALT. Ejer-valget var, at rail'en forbliver aktiv, saa
//     man kan skifte projekt med indstillinger aabne. Derfor: `role="dialog"`
//     UDEN `aria-modal`, og ingen fokus-faelde. `CloseWorkspaceDialog` har en
//     haerdet faelde (document-capture, `focusin`-vagt) — den er rigtig for en
//     bekraeftelse og ville her spaerre praecis den rail, brugeren skal kunne naa.
//
//  3. ESC LYTTES PAA `document`, IKKE `window`. `HotkeyRecorder` lytter paa
//     `window` i capture-fasen og kalder `stopImmediatePropagation()`, mens den
//     optager. Capture gaar window -> document, saa optageren ser Esc FOERST og
//     vinduet ser den slet ikke: Esc afbryder optagelsen i stedet for at lukke
//     vinduet under haenderne paa en bruger midt i at vaelge genvej. Byttes de to
//     lyttere om, forsvinder den egenskab tavst.

import {
  Suspense,
  lazy,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { setOcclusionReason } from "./browser/occlusion";
import { FROSTED_BACKDROP } from "./canvas/liquidGlass";
import {
  DEFAULT_SETTINGS_CATEGORY,
  SETTINGS_CATEGORIES,
  type SettingsCategory,
} from "./settingsCategories";

// Lazy-chunken. Samme moenster (og samme begrundelse) som App.tsx' §3: modulet
// maa ikke parses paa den opstarts-kritiske vej. `hentSettings` eksporteres,
// fordi opvarmningen og `lazy()` SKAL dele praecis samme modul-loefte — to kald
// til `import()` ville give to hentninger.
const importerSettings = () => import("./Settings");
let settingsModul: ReturnType<typeof importerSettings> | null = null;
export const hentSettings = () => (settingsModul ??= importerSettings());
const Settings = lazy(hentSettings);

export interface SettingsWindowProps {
  open: boolean;
  onClose(): void;
  dryRun: boolean;
  onDryRunChange(value: boolean): void;
  /** `VITE_VOICE_DRY_RUN` er sat: afkrydsningen er laast og forklaringen skifter. */
  dryRunForced: boolean;
  /**
   * Tegn fejlfindings-blokken. Kalderen afgoer det, saa beslutningen om hvad
   * der SENDES MED bor ét sted (App'ens build-flag) i stedet for at vaere
   * gemt i en `import.meta.env`-opslag her — og saa begge grene kan testes.
   */
  showDebug: boolean;
  capturePath: string | null;
  onSaved(): void | Promise<void>;
}

export function SettingsWindow({
  open,
  onClose,
  dryRun,
  onDryRunChange,
  dryRunForced,
  showDebug,
  capturePath,
  onSaved,
}: SettingsWindowProps) {
  const [everOpened, setEverOpened] = useState(false);
  const [category, setCategory] = useState<SettingsCategory>(
    DEFAULT_SETTINGS_CATEGORY,
  );
  const boxRef = useRef<HTMLDivElement | null>(null);
  const restoreFocusRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (open) setEverOpened(true);
  }, [open]);

  // OPVARMNING. Gevinsten ved lazy-loading er at chunken ikke ligger foer
  // foerste frame — ikke at den skal hentes sent. Uden det her betales
  // gevinsten tilbage med renter ved foerste klik, fordi den dynamiske import
  // skal gennem Tauris asset-protokol (IPC + dekomprimering) foer parse.
  useEffect(() => {
    const varmOp = () => {
      void hentSettings();
    };
    if (typeof requestIdleCallback === "function") {
      const id = requestIdleCallback(varmOp, { timeout: 3_000 });
      // Tvillingen tjekkes for sig: antager oprydningen at `cancelIdleCallback`
      // findes bare fordi schedule-siden gjorde, kaster unmount i et miljoe der
      // kun har den ene — og en kastende cleanup river hele unmount-passet med.
      return () => {
        if (typeof cancelIdleCallback === "function") cancelIdleCallback(id);
      };
    }
    const frist = setTimeout(varmOp, 1_500);
    return () => clearTimeout(frist);
  }, []);

  // Occlusion-gaten: WebView2-boernene maler OVER DOM'et, saa et browser-kort
  // ville daekke hele vinduet. Aarsagen ryddes ogsaa ved unmount — ellers
  // overlever en stale occlusion et fjernet vindue, og browser-kortene bliver
  // usynlige resten af sessionen.
  useEffect(() => {
    setOcclusionReason("settings-panel", open);
    return () => setOcclusionReason("settings-panel", false);
  }, [open]);

  // Se punkt 3 i filens hoved: `document`, ikke `window`.
  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      // Shift+Esc er husets LAASTE exit-type-mode-kombination (keyRouting.ts) —
      // den maa ikke ogsaa lukke vinduet.
      if (event.ctrlKey || event.shiftKey || event.altKey || event.metaKey) return;
      event.preventDefault();
      onClose();
    };
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [open, onClose]);

  // Fokus ind ved aabning, tilbage til tandhjulet ved lukning. Elementet
  // fanges ved AABNING (hvor det stadig er tandhjulet der har fokus) og
  // kontrolleres for `isConnected` inden det faar fokus tilbage: rail'en kan
  // have gen-tegnet imens, og fokus paa et frakoblet element er en no-op der
  // efterlader fokus paa <body>.
  useEffect(() => {
    if (!open) return;
    restoreFocusRef.current =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    boxRef.current?.focus();
    return () => {
      const tilbage = restoreFocusRef.current;
      restoreFocusRef.current = null;
      if (tilbage !== null && tilbage.isConnected) tilbage.focus();
    };
  }, [open]);

  // Foer foerste aabning tegnes intet overhovedet — effekterne ovenfor
  // (opvarmning) koerer stadig, fordi komponenten selv er monteret.
  if (!open && !everOpened) return null;

  return (
    <div
      data-settings-window-root
      style={{ ...styles.root, ...(open ? null : styles.rootClosed) }}
    >
      <style>{settingsWindowCss}</style>

      <div
        data-settings-backdrop
        aria-hidden="true"
        style={styles.backdrop}
        onClick={onClose}
      />

      <div
        data-settings-window
        ref={boxRef}
        role="dialog"
        aria-labelledby="settings-window-title"
        // Uden den lander et klik i vinduets bundtekst paa <body>, og
        // tastaturbrugeren starter naeste Tab forfra i dokumentet.
        tabIndex={-1}
        style={styles.window}
      >
        <header style={styles.header}>
          <h2 id="settings-window-title" style={styles.title}>
            Indstillinger
          </h2>
          <button
            type="button"
            data-settings-close
            aria-label="Luk indstillinger"
            title="Luk indstillinger"
            style={styles.closeButton}
            onClick={onClose}
          >
            ✕
          </button>
        </header>

        <div style={styles.body}>
          <nav
            data-settings-nav
            aria-label="Kategorier"
            style={styles.nav}
          >
            {SETTINGS_CATEGORIES.map((punkt) => (
              <button
                key={punkt.id}
                type="button"
                data-settings-category-button={punkt.id}
                aria-current={punkt.id === category ? "true" : undefined}
                style={{
                  ...styles.navItem,
                  ...(punkt.id === category ? styles.navItemActive : null),
                }}
                onClick={() => setCategory(punkt.id)}
              >
                {punkt.label}
              </button>
            ))}
          </nav>

          <div data-settings-content style={styles.content}>
            <Suspense
              fallback={
                <div data-settings-loading style={styles.loading}>
                  Indlæser indstillinger…
                </div>
              }
            >
              {everOpened ? (
                <Settings category={category} onSaved={onSaved} />
              ) : null}
            </Suspense>

            {category === "voice" && showDebug && (
              <VoiceDebugSection
                dryRun={dryRun}
                onDryRunChange={onDryRunChange}
                dryRunForced={dryRunForced}
                capturePath={capturePath}
              />
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

/**
 * Dry-run og capture-stien. De persisteres IKKE i `settings.json` (de bor i
 * App'ens state og nulstilles ved genstart), og de er skrevet til udvikling:
 * "action_count=0" siger ikke noget til nogen udefra. Derfor tegnes blokken
 * kun naar kalderen siger til — se `showDebug`.
 *
 * API-lyd-kontakten laa her indtil 2026-07-29 og er fjernet: den satte kun
 * state, ingen laeste. Se noten i `voice/Hud.tsx`.
 */
function VoiceDebugSection({
  dryRun,
  onDryRunChange,
  dryRunForced,
  capturePath,
}: {
  dryRun: boolean;
  onDryRunChange(value: boolean): void;
  dryRunForced: boolean;
  capturePath: string | null;
}) {
  return (
    <div data-settings-debug style={styles.debug}>
      <div style={styles.debugTitle}>Fejlfinding (gemmes ikke)</div>
      <label style={styles.debugLabel}>
        <input
          type="checkbox"
          checked={dryRun}
          disabled={dryRunForced}
          onChange={(event) => onDryRunChange(event.target.checked)}
        />
        Voice dry-run (action_count=0)
      </label>
      <div style={styles.debugHint}>
        {dryRunForced
          ? "Tvunget af VITE_VOICE_DRY_RUN."
          : "Kører session, guard og resolver uden app-handlinger."}
      </div>
      {capturePath !== null && (
        <div style={styles.capturePath}>{capturePath}</div>
      )}
    </div>
  );
}

export const settingsWindowCss = `
  [data-settings-window] button:focus-visible,
  [data-settings-window] input:focus-visible,
  [data-settings-window] select:focus-visible {
    outline: 2px solid rgba(103, 183, 255, 0.78);
    outline-offset: 2px;
  }
  [data-settings-nav] button:hover { color: #e6f2ff; }
  [data-settings-close]:hover { color: #ffd7d3; }

  /* Knapperne saetter deres egen \`color\`, og saa graaner browseren dem ikke
     naar de er disabled — "Fjern" uden gemt noegle og "Nulstil" under
     optagelse saa ud til at kunne trykkes. Reglen daekker hele vinduet,
     HotkeyRecorder inklusive. */
  [data-settings-window] button:disabled {
    opacity: 0.45;
    cursor: default;
  }
`;

const styles: Record<string, CSSProperties> = {
  // `absolute`, ikke `fixed`: roden monteres i App'ens <main>, som ER
  // canvas-zonen. Dermed daekker inset:0 praecis den zone, og rail'en bliver
  // hverken daempet eller spaerret — det var hele pointen med ejer-valget.
  // zIndex 70 ligger over topbaren (60), bund-docken (50) og orb-docken (40).
  root: {
    position: "absolute",
    inset: 0,
    zIndex: 70,
    display: "grid",
    placeItems: "center",
    boxSizing: "border-box",
    padding: 32,
  },
  // Lukket, men monteret. Se punkt 1 i filens hoved.
  rootClosed: { display: "none" },
  backdrop: {
    position: "absolute",
    inset: 0,
    background: "rgba(3, 8, 18, 0.52)",
    backdropFilter: "blur(2px)",
    WebkitBackdropFilter: "blur(2px)",
  },
  window: {
    position: "relative",
    width: "min(880px, 100%)",
    height: "min(620px, 100%)",
    display: "flex",
    flexDirection: "column",
    boxSizing: "border-box",
    overflow: "hidden",
    border: "1px solid rgba(222, 241, 255, 0.2)",
    borderRadius: 14,
    background:
      "radial-gradient(120% 60% at 0% 0%, rgba(113, 180, 225, 0.1), transparent 62%), linear-gradient(180deg, rgba(10, 20, 34, 0.86), rgba(2, 8, 18, 0.92))",
    backdropFilter: FROSTED_BACKDROP,
    WebkitBackdropFilter: FROSTED_BACKDROP,
    boxShadow: "0 32px 80px rgba(0, 4, 18, 0.62)",
    color: "#b9c8d9",
    fontFamily: '"Segoe UI", system-ui, sans-serif',
    fontSize: 13,
  },
  header: {
    flex: "0 0 auto",
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
    padding: "13px 14px 13px 18px",
    borderBottom: "1px solid rgba(209, 232, 251, 0.1)",
  },
  title: {
    margin: 0,
    color: "#edf5fc",
    fontSize: 14,
    fontWeight: 600,
  },
  closeButton: {
    display: "grid",
    width: 28,
    height: 28,
    flex: "0 0 auto",
    placeItems: "center",
    border: 0,
    borderRadius: 8,
    padding: 0,
    background: "transparent",
    color: "#9eafc1",
    fontSize: 12,
    cursor: "pointer",
  },
  // minHeight: 0 er ikke pynt — uden den vokser flex-barnet med indholdet i
  // stedet for at scrolle, og bunden af en lang kategori falder ud af kassen.
  body: { flex: "1 1 auto", minHeight: 0, display: "flex" },
  nav: {
    flex: "0 0 auto",
    width: 188,
    display: "flex",
    flexDirection: "column",
    gap: 2,
    boxSizing: "border-box",
    overflowY: "auto",
    padding: "10px 8px",
    borderRight: "1px solid rgba(209, 232, 251, 0.08)",
  },
  navItem: {
    border: 0,
    borderRadius: 8,
    padding: "8px 10px",
    background: "transparent",
    color: "#b9c8d9",
    font: "inherit",
    fontSize: 12,
    textAlign: "left",
    cursor: "pointer",
    transition: "background 140ms ease, color 140ms ease",
  },
  navItemActive: {
    background: "rgba(122, 182, 232, 0.16)",
    boxShadow: "inset 0 0 0 1px rgba(160, 209, 255, 0.24)",
    color: "#edf5fc",
  },
  content: {
    flex: "1 1 auto",
    minWidth: 0,
    minHeight: 0,
    overflowY: "auto",
    padding: "16px 18px 20px",
  },
  loading: { color: "#718297", fontSize: 12 },
  debug: {
    display: "flex",
    flexDirection: "column",
    gap: 7,
    marginTop: 18,
    paddingTop: 14,
    borderTop: "1px solid rgba(209, 232, 251, 0.08)",
    color: "#8ea0b5",
    fontSize: 12,
  },
  debugTitle: {
    color: "#718297",
    fontSize: 10,
    fontWeight: 700,
    letterSpacing: "0.14em",
    textTransform: "uppercase",
  },
  debugLabel: { display: "flex", alignItems: "center", gap: 7 },
  debugHint: { color: "#6b7c90" },
  capturePath: {
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    color: "#8fd6c2",
    fontFamily: '"Cascadia Mono", monospace',
    fontSize: 10,
  },
};

export default SettingsWindow;
