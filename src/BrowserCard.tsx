// Browser-kort (browser-kort-sporet, spec §4/§8a). Kortets header spejler
// Card.tsx' visuelle sprog (nummer-badge, titel), men i stedet for en
// xterm-krop bærer body'en KUN en tom placeholder-flade med
// data-browser-card-body: den reelle webview er et WebView2-barn Rust-side,
// som CanvasSurface positionerer via set_browser_bounds mod netop dette
// element. Klik i webviewen når aldrig DOM'et (accepteret §4-begrænsning), så
// kortet får hverken type-mode eller onBodyPointerDown.

import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import type { BrowserCardInfo } from "./types";
import { cardLabel } from "./cardLabel";
import {
  flushPerfTrace,
  markPerf,
  perfInvokeArgs,
  registerPendingClose,
  startPerfTrace,
  type PerfTrace,
} from "./perfTrace";

export interface BrowserCardProps {
  card: BrowserCardInfo;
  fullscreen: boolean;
  onToggleFullscreen(): void;
  onClosed?(trace?: PerfTrace | null): void;
}

const EXPLICIT_URL_SCHEME = /^[a-z][a-z\d+.-]*:/i;
const HOST_WITH_PORT =
  /^(?:[a-z\d-]+\.)*[a-z\d-]+:\d+(?:[/?#]|$)/i;

/** Gør den almindelige adressefelt-form (`github.com`) til en http(s)-URL.
 *  Eksplicitte schemes bevares, så Rust fortsat er den autoritative
 *  allowlist og kan afvise fx `file:`/`javascript:`. Host+port-former som
 *  `localhost:3000` er ikke schemes og får derfor også https-prefix — det
 *  samme gælder "schemes" med punktum (`example.com:8080abc`): punktum er
 *  gyldigt i et RFC-scheme, men i et adressefelt er det altid en host. */
export function normalizeBrowserUrlInput(raw: string): string | null {
  const trimmed = raw.trim();
  if (trimmed === "") return null;
  const scheme = EXPLICIT_URL_SCHEME.exec(trimmed);
  const hostLikeScheme = scheme !== null && scheme[0].includes(".");
  return scheme !== null && !hostLikeScheme && !HOST_WITH_PORT.test(trimmed)
    ? trimmed
    : `https://${trimmed}`;
}

export function BrowserCard({
  card,
  fullscreen,
  onToggleFullscreen,
  onClosed,
}: BrowserCardProps) {
  const { name, number, url, title, opened_by, running } = card;

  // URL-feltet er en redigerbar kopi af den aktuelle url. Den resynkes når
  // navigation Rust-side skifter card.url (browser-card-updated -> refresh) —
  // men ALDRIG midt i en indtastning: 1s-pollen må ikke snappe feltet
  // tilbage under brugerens fingre. Enter blur'er feltet, så navigationens
  // url-opdateringer (redirects mv.) igen slår igennem bagefter.
  const [urlDraft, setUrlDraft] = useState(url);
  const editingRef = useRef(false);
  useEffect(() => {
    if (!editingRef.current) setUrlDraft(url);
  }, [url]);

  // Backendens navigate-/close-fejl er hårde og handlingsbare — de må ikke
  // forsvinde i et tomt catch (præcis den fejlklasse bug 2/3 handlede om).
  const [actionError, setActionError] = useState<string | null>(null);

  const closeCard = () => {
    const trace = PERF_ENABLED
      ? startPerfTrace("close", { entry: "browser_x", name, number })
      : null;
    if (PERF_ENABLED) {
      registerPendingClose(name, trace);
      markPerf(trace, "close.frontend.invoke.begin", { name });
    }
    const args = PERF_ENABLED ? { name, ...perfInvokeArgs(trace) } : { name };
    void invoke("close_card", args)
      .then(() => {
        if (PERF_ENABLED) {
          markPerf(trace, "close.frontend.invoke.resolved", { name });
          void flushPerfTrace(trace);
          onClosed?.(trace);
        } else {
          onClosed?.();
        }
      })
      .catch((error: unknown) => {
        if (PERF_ENABLED) {
          markPerf(trace, "close.frontend.invoke.rejected", {
            name,
            error: String(error),
          });
          void flushPerfTrace(trace, true);
        }
        setActionError("Kunne ikke lukke kortet: " + String(error));
      });
  };

  // YAGNI-rettelse (v1): en død webview genåbnes ikke — kortet lukkes manuelt,
  // og operatøren opretter et nyt browser-kort ad normal vej.
  if (!running) {
    return (
      <section style={styles.card}>
        {actionError && (
          <div
            data-browser-error
            style={styles.errorStrip}
            title="Klik for at skjule"
            onClick={() => setActionError(null)}
          >
            {actionError}
          </div>
        )}
        <div data-browser-dead style={styles.dead}>
          <p style={styles.deadText}>Browserprocessen er død — luk kortet</p>
          <button
            type="button"
            data-browser-close-action
            onClick={closeCard}
            style={styles.deadButton}
          >
            Luk kort
          </button>
        </div>
      </section>
    );
  }

  const submitUrl = (ev: ReactKeyboardEvent<HTMLInputElement>) => {
    if (ev.key !== "Enter") return;
    ev.preventDefault();
    // Adressefeltet ejer Enter. Kortet kan stadig stå i CanvasSurface's
    // type-mode efter et tidligere terminalklik, men tastetrykket må ikke
    // boble videre som terminalinput.
    ev.stopPropagation();
    const next = normalizeBrowserUrlInput(ev.currentTarget.value);
    if (next === null) return;
    setUrlDraft(next);
    setActionError(null);
    ev.currentTarget.blur();
    void invoke("navigate_browser_card", { name, url: next }).catch(
      (error: unknown) =>
        setActionError(`Navigation afvist: ${String(error)}`),
    );
  };

  return (
    <section
      data-browser-fullscreen={fullscreen ? "true" : "false"}
      style={styles.card}
    >
      <header style={styles.header}>
        <span data-browser-number-badge style={styles.numberBadge}>
          {number}
        </span>
        {opened_by && (
          // opened_by ER kortnavnet (card-N) fra Rust — chippen viser hvem der
          // åbnede browseren, så den skal læses som resten af fladen.
          <span
            data-browser-opened-by
            style={styles.openedBy}
            title={`Åbnet af ${cardLabel(opened_by)}`}
          >
            {cardLabel(opened_by)}
          </span>
        )}
        <span data-browser-title style={styles.title} title={title}>
          {title}
        </span>
        <input
          data-browser-url-input
          value={urlDraft}
          spellCheck={false}
          onFocus={() => {
            editingRef.current = true;
          }}
          onBlur={() => {
            editingRef.current = false;
          }}
          onChange={(ev) => {
            editingRef.current = true;
            setUrlDraft(ev.currentTarget.value);
          }}
          onKeyDown={submitUrl}
          placeholder="https://…"
          style={styles.urlInput}
        />
        <button
          type="button"
          data-browser-fullscreen-action
          onClick={onToggleFullscreen}
          title={fullscreen ? "Afslut fuldskærm" : "Fuldskærm"}
          aria-label={fullscreen ? "Afslut fuldskærm" : "Fuldskærm"}
          style={styles.iconButton}
        >
          {fullscreen ? "⤡" : "⤢"}
        </button>
        <button
          type="button"
          data-browser-close-action
          onClick={closeCard}
          title="Luk kort"
          aria-label="Luk kort"
          style={styles.iconButton}
        >
          ✕
        </button>
      </header>
      {/* Fejl-striben ligger MELLEM header og body: den skubber body-rekten,
          som ResizeObserver'en så genrapporterer — webviewen dækker den
          dermed aldrig. Klik skjuler den igen. */}
      {actionError && (
        <div
          data-browser-error
          style={styles.errorStrip}
          title="Klik for at skjule"
          onClick={() => setActionError(null)}
        >
          {actionError}
        </div>
      )}
      {/* Ren placeholder — WebView2-barnet tegnes OVENpå denne flade, som
          CanvasSurface måler og rapporterer via set_browser_bounds. */}
      <div data-browser-card-body style={styles.body} />
    </section>
  );
}

const styles: Record<string, CSSProperties> = {
  card: {
    border: "none",
    borderRadius: 13,
    background:
      "linear-gradient(145deg, rgba(13, 22, 34, 0.98), rgba(2, 7, 14, 0.99))",
    display: "flex",
    flexDirection: "column",
    height: "100%",
    overflow: "hidden",
    boxShadow: "inset 0 1px 0 rgba(229, 244, 255, 0.1)",
  },
  header: {
    display: "flex",
    alignItems: "center",
    gap: 7,
    minHeight: 35,
    padding: "0 8px 0 9px",
    borderBottom: "1px solid rgba(207, 232, 255, 0.08)",
    background:
      "linear-gradient(180deg, rgba(31, 42, 56, 0.98), rgba(12, 18, 27, 0.98))",
    fontSize: 12,
  },
  numberBadge: {
    minWidth: 19,
    boxSizing: "border-box",
    padding: "1px 5px",
    border: "1px solid rgba(187, 211, 233, 0.14)",
    borderRadius: 6,
    background: "rgba(112, 137, 163, 0.15)",
    color: "#9eafc1",
    textAlign: "center",
    fontSize: 10,
    fontWeight: 700,
    fontVariantNumeric: "tabular-nums",
    flex: "0 0 auto",
  },
  openedBy: {
    padding: "1px 6px",
    border: "1px solid rgba(116, 192, 255, 0.24)",
    borderRadius: 5,
    background: "rgba(45, 92, 138, 0.28)",
    color: "#a9cdf0",
    fontSize: 9,
    whiteSpace: "nowrap",
    flex: "0 0 auto",
  },
  title: {
    color: "#e2ebf4",
    fontWeight: 600,
    whiteSpace: "nowrap",
    overflow: "hidden",
    textOverflow: "ellipsis",
    maxWidth: 160,
    flex: "0 1 auto",
  },
  urlInput: {
    minWidth: 0,
    flex: "1 1 auto",
    padding: "3px 8px",
    border: "1px solid rgba(151, 184, 218, 0.22)",
    borderRadius: 6,
    background: "rgba(3, 8, 16, 0.7)",
    color: "#cfe0f2",
    fontFamily: '"Cascadia Mono", monospace',
    fontSize: 10,
    outline: "none",
  },
  iconButton: {
    flex: "0 0 auto",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    width: 22,
    height: 22,
    padding: 0,
    border: "1px solid rgba(187, 211, 233, 0.14)",
    borderRadius: 6,
    background: "rgba(112, 137, 163, 0.12)",
    color: "#9eafc1",
    cursor: "pointer",
    fontSize: 12,
    lineHeight: 1,
  },
  body: {
    flex: 1,
    minHeight: 0,
    position: "relative",
    background: "#02060c",
  },
  errorStrip: {
    flex: "0 0 auto",
    padding: "4px 10px",
    background: "rgba(120, 32, 32, 0.55)",
    borderBottom: "1px solid rgba(255, 120, 120, 0.25)",
    color: "#f0b9b9",
    fontSize: 11,
    cursor: "pointer",
    whiteSpace: "nowrap",
    overflow: "hidden",
    textOverflow: "ellipsis",
  },
  dead: {
    flex: 1,
    minHeight: 0,
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    justifyContent: "center",
    gap: 10,
    padding: 20,
    background: "rgba(2, 8, 18, 0.86)",
  },
  deadText: {
    margin: 0,
    color: "#cdb0b0",
    fontSize: 13,
    textAlign: "center",
  },
  deadButton: {
    background: "#3b82f6",
    color: "#fff",
    border: "none",
    borderRadius: 4,
    padding: "5px 14px",
    cursor: "pointer",
    fontSize: 12,
  },
};
