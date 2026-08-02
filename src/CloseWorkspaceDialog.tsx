import { useEffect, useRef, type CSSProperties } from "react";
import { FROSTED_BACKDROP } from "./canvas/liquidGlass";

/**
 * Bekraeftelsen foran en lukning der stopper koerende kort.
 *
 * Den vises ALTID i det synlige vindue — ogsaa naar det er et skjult workspace
 * i en anden proces der lukkes. En skjult proces kan ikke tegne en dialog, saa
 * brugeren skal se den paa den skaerm han faktisk kigger paa. Derfor tager
 * komponenten et navn ind i stedet for at gaette paa "dette vindue".
 *
 * Tilgaengeligheden er et ACCEPTANCEKRAV (spec §5.2: "bekraeftelsesdialogen
 * tager og genskaber fokus korrekt"), ikke pynt:
 *  - fokus flyttes ind i dialogen naar den aabner, og paa den SIKRE knap
 *    (Annullér). En destruktiv handling maa ikke ligge under den blindt
 *    trykkede mellemrumstast.
 *  - fokus gives tilbage til det element der havde den, naar dialogen lukker.
 *  - Escape annullerer (uden modifikatorer — Shift+Esc er husets LAASTE
 *    exit-type-mode-kombination, se keyRouting.ts).
 *  - Tab er faeldet inde i dialogen, saa tastaturbrugeren ikke kan tabbe ud i
 *    en flade der er modalt spaerret for musen.
 *
 * FOKUS-FAELDEN MAA IKKE HAENGE PAA HVOR FOKUS TILFAELDIGVIS STAAR (fix-runde 1,
 * fund 1). Den laa foer som en React-`onKeyDown` paa panelet. Klikkede brugeren
 * paa dialogens BROEDTEKST, flyttede Chromium fokus til naermeste fokuserbare
 * forfader — der var ingen, saa fokus landede paa <body>. React delegerer paa
 * #root, som er BARN af body, saa handleren saa aldrig det naeste Tab: native
 * tabulering flyttede da til foerste tabbable element i dokumentet, og rail'en
 * staar FOER <main> i DOM'en. Brugeren stod paa et andet workspaces ✕ bag en
 * flade der kun var modal for musen. Faelden er derfor nu tre ting der ikke
 * afhaenger af hinanden:
 *  1. panelet har `tabIndex={-1}`, saa et klik i broedteksten lander PAA panelet
 *     og ikke paa <body>,
 *  2. Tab og Escape hoeres paa `document` i CAPTURE-fasen — den hoerer eventet
 *     uanset om fokus staar i dialogen, paa <body> eller uden for,
 *  3. en `focusin`-vagt traekker fokus tilbage hvis den alligevel havner uden for.
 */
export type CloseWorkspaceRequest =
  | {
      /** Udeladt af kompatibilitet med de eksisterende workspace-kaldere. */
      kind?: "workspace";
      /** `null` = det aktuelle vindue; ellers workspace-navnet. */
      workspaceName: string | null;
      /** Antal kørende kort i dét workspace. */
      running: number;
      workspaces?: never;
    }
  | {
      kind: "application";
      /** Antal levende workspace-processer der afsluttes. */
      workspaces: number;
      /** Samlet antal kørende kort på tværs af processerne. */
      running: number;
      workspaceName?: never;
    };

export interface CloseWorkspaceDialogProps {
  /** `null` = ingen bekraeftelse i gang; komponenten tegner intet. */
  request: CloseWorkspaceRequest | null;
  onConfirm(): void;
  onCancel(): void;
  /**
   * Elementet fokus skal tilbage til. Udelades den, bruges det element der
   * havde fokus da dialogen blev monteret.
   *
   * Kalderen har brug for den, fordi luk-knappen i titelbjaelken bliver
   * `disabled` i SAMME commit som dialogen aabner — browseren har da allerede
   * flyttet fokus til <body>, og "det element der havde fokus" er tabt naar
   * vores effekt koerer.
   *
   * Proppen er PAAKRAEVET (fix-runde 1, fund 3). Den var valgfri, og fordi
   * App.tsx altid sendte den, var fallback-grenen doed i appen mens testen kun
   * maalte fallback-grenen: en regression hvor App holdt op med at sende den
   * ville vaere groen i test og oedelagt i appen. Nu er praecis den regression
   * en TYPEFEJL — `tsc` afviser en montering uden proppen.
   *
   * `null` = kalderen havde intet at give tilbage; saa falder vi tilbage paa
   * det element der havde fokus ved monteringen.
   */
  restoreFocusTo: HTMLElement | null;
}

/** Ren tekst-udledning — testbar uden DOM, som `workspaces.ts`' hjaelpere. */
export function closeDialogText(request: CloseWorkspaceRequest): {
  title: string;
  body: string;
  confirmLabel: string;
} {
  if (request.kind === "application") {
    const omfang =
      request.workspaces === 1
        ? "Det åbne workspace"
        : `Alle ${request.workspaces} åbne workspaces`;
    const kort =
      request.running > 0
        ? `, og ${request.running} kørende kort bliver stoppet`
        : "";
    return {
      title: "Afslut Talminal?",
      body: `${omfang} lukkes${kort}. Talminal afsluttes helt.`,
      confirmLabel: "Afslut Talminal",
    };
  }
  const hvor =
    request.workspaceName === null ? "dette vindue" : request.workspaceName;
  return {
    title:
      request.workspaceName === null
        ? "Luk vinduet?"
        : `Luk ${request.workspaceName}?`,
    // "kort" boejes ikke i flertal paa dansk, saa ét udtryk daekker begge tal.
    body: `Der kører ${request.running} kort i ${hvor}. Lukker du nu, bliver de stoppet.`,
    confirmLabel:
      request.workspaceName === null ? "Luk vinduet" : "Luk projektet",
  };
}

/** De elementer Tab maa lande paa inde i dialogen. Panelet selv har
 *  `tabindex="-1"` og er derfor bevidst UDE af listen. */
function tabbablesIn(root: HTMLElement | null): HTMLElement[] {
  if (root === null) return [];
  return Array.from(
    root.querySelectorAll<HTMLElement>(
      'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
    ),
  );
}

export function CloseWorkspaceDialog(props: CloseWorkspaceDialogProps) {
  // Den indre komponent monteres og unmountes med anmodningen. Det er ikke
  // kosmetik: fokus-optag og fokus-genskabelse ER mount/unmount, og en
  // komponent der blot skjuler sig selv har ingen unmount at rydde op i.
  if (props.request === null) return null;
  return <Dialog {...props} request={props.request} />;
}

function Dialog({
  request,
  onConfirm,
  onCancel,
  restoreFocusTo,
}: CloseWorkspaceDialogProps & { request: CloseWorkspaceRequest }) {
  const rootRef = useRef<HTMLDivElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const restoreRef = useRef<HTMLElement | null>(null);
  // Escape-lytteren registreres én gang; uden ref'en ville hver ny
  // onCancel-identitet af- og genregistrere den.
  const cancelHandlerRef = useRef(onCancel);
  cancelHandlerRef.current = onCancel;

  const text = closeDialogText(request);

  useEffect(() => {
    const active = document.activeElement;
    restoreRef.current =
      restoreFocusTo !== null
        ? restoreFocusTo
        : active instanceof HTMLElement
          ? active
          : null;
    cancelRef.current?.focus();

    // ÉN lytter paa dokumentet i CAPTURE-fasen daekker baade Escape og
    // Tab-faelden. Capture og ikke bobling, fordi dialogen er MODAL: eventet
    // maa ikke naa hverken en fokuseret terminal eller en React-handler
    // nedenfor. Og paa `document` og ikke paa panelet, fordi faelden ellers
    // holder op med at virke i praecis det oejeblik fokus er faldet ud af
    // dialogen — som er det eneste oejeblik den betyder noget.
    //
    // Arbitration: CanvasSurface lytter i capture paa `window`, altsaa FOER os
    // (window -> document i capture-vejen), og stopper Shift+Esc dér som
    // "exit-type-mode" (keyRouting.ts). Vi tager derfor kun Escape UDEN
    // modifikatorer og konkurrerer ikke med husets laaste kombination.
    const onKeyDownDocument = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        if (event.shiftKey || event.ctrlKey || event.altKey || event.metaKey) {
          return;
        }
        event.preventDefault();
        event.stopPropagation();
        cancelHandlerRef.current();
        return;
      }
      if (event.key !== "Tab") return;
      const items = tabbablesIn(rootRef.current);
      if (items.length === 0) return;
      event.preventDefault();
      event.stopPropagation();
      const index = items.indexOf(document.activeElement as HTMLElement);
      const step = event.shiftKey ? -1 : 1;
      const next =
        index === -1
          ? event.shiftKey
            ? items.length - 1
            : 0
          : (index + step + items.length) % items.length;
      items[next].focus();
    };
    document.addEventListener("keydown", onKeyDownDocument, true);

    // Sidste vagt: lander fokus alligevel uden for dialogen (native tabulering
    // fra <body>, et museklik i rail'en, et programmatisk `focus()`), haler vi
    // den tilbage til panelet med det samme.
    const onFocusIn = (event: FocusEvent) => {
      const root = rootRef.current;
      if (root === null) return;
      const target = event.target;
      if (target instanceof Node && root.contains(target)) return;
      root.focus();
    };
    document.addEventListener("focusin", onFocusIn);

    return () => {
      document.removeEventListener("keydown", onKeyDownDocument, true);
      document.removeEventListener("focusin", onFocusIn);
      const back = restoreRef.current;
      // `document.contains`: raden dialogen blev aabnet fra kan vaere vaek —
      // det workspace vi lige lukkede stod maaske i den.
      if (back !== null && document.contains(back)) back.focus();
    };
    // Bevidst tom: vaerdierne skal fastholdes fra AABNINGEN, ikke fra hver
    // render. `restoreFocusTo` laeses derfor kun her.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div
      data-close-dialog-backdrop
      style={styles.backdrop}
      onMouseDown={(event) => {
        // Kun et klik paa selve baggrunden — ikke et traek der slutter der.
        if (event.target === event.currentTarget) onCancel();
      }}
    >
      <style>{closeDialogCss}</style>
      <div
        ref={rootRef}
        data-close-dialog
        role="dialog"
        aria-modal="true"
        aria-labelledby="close-dialog-title"
        aria-describedby="close-dialog-body"
        style={styles.panel}
        /* Et klik i broedteksten skal lande PAA panelet, ikke paa <body>:
           Chromium flytter fokus til naermeste fokuserbare forfader. */
        tabIndex={-1}
      >
        <h2 id="close-dialog-title" style={styles.title}>
          {text.title}
        </h2>
        <p id="close-dialog-body" style={styles.body}>
          {text.body}
        </p>
        <div style={styles.actions}>
          <button
            type="button"
            data-close-cancel
            ref={cancelRef}
            style={styles.ghostButton}
            onClick={onCancel}
          >
            Annullér
          </button>
          <button
            type="button"
            data-close-confirm
            style={styles.dangerButton}
            onClick={onConfirm}
          >
            {text.confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}

/** Husets stil: komponenten shipper sin egen CSS lokalt (WindowControls.tsx:14,
 *  WorkspaceRail.tsx:80) — der er ingen global CSS i projektet. */
const closeDialogCss = `
  /* Panelet er kun fokuserbart for at holde fokus INDE i dialogen; det er ikke
     en kontrol og skal derfor ikke tegne en fokus-ring. */
  [data-close-dialog]:focus { outline: none; }
  [data-close-dialog] button:focus-visible {
    outline: 2px solid rgba(103, 183, 255, 0.78);
    outline-offset: 2px;
  }
  [data-close-dialog] [data-close-cancel]:hover { color: #e6f2ff; }
  [data-close-dialog] [data-close-confirm]:hover {
    background: rgba(224, 96, 88, 0.26);
    color: #ffe9e6;
  }
`;

const styles: Record<string, CSSProperties> = {
  // `fixed` og over ALT: bekraeftelsen daekker ogsaa rail'en, for rail'ens ✕ er
  // en af de veje der kan aabne den.
  backdrop: {
    position: "fixed",
    inset: 0,
    zIndex: 120,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    background: "rgba(2, 8, 18, 0.52)",
  },
  panel: {
    boxSizing: "border-box",
    width: "min(420px, calc(100vw - 48px))",
    border: "1px solid rgba(221, 240, 255, 0.2)",
    borderRadius: 14,
    padding: "18px 20px 16px",
    background:
      "linear-gradient(135deg, rgba(48, 76, 108, 0.34), rgba(3, 11, 23, 0.42))",
    boxShadow: "0 24px 60px rgba(0, 7, 24, 0.5), inset 0 1px 0 rgba(240, 249, 255, 0.22)",
    backdropFilter: FROSTED_BACKDROP,
    WebkitBackdropFilter: FROSTED_BACKDROP,
    color: "#b9c8d9",
    fontFamily: '"Segoe UI", system-ui, sans-serif',
  },
  title: {
    margin: "0 0 8px",
    color: "#edf5fc",
    fontSize: 15,
    fontWeight: 600,
  },
  body: {
    margin: "0 0 18px",
    color: "#aebdd0",
    fontSize: 12,
    lineHeight: 1.55,
  },
  actions: {
    display: "flex",
    justifyContent: "flex-end",
    gap: 8,
  },
  ghostButton: {
    border: "1px solid rgba(222, 241, 255, 0.18)",
    borderRadius: 8,
    padding: "7px 13px",
    background: "transparent",
    color: "#c8d9ea",
    font: "inherit",
    fontSize: 12,
    cursor: "pointer",
  },
  dangerButton: {
    border: "1px solid rgba(224, 96, 88, 0.5)",
    borderRadius: 8,
    padding: "7px 13px",
    background: "rgba(224, 96, 88, 0.16)",
    color: "#ffd7d3",
    font: "inherit",
    fontSize: 12,
    cursor: "pointer",
  },
};

export default CloseWorkspaceDialog;
