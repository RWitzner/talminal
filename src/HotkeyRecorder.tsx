import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { parseAccelerator } from "./voice/ptt";
import { hotkeyLabelParts } from "./HotkeyLabel";

const MODIFIER_CODES = new Set([
  "ControlLeft",
  "ControlRight",
  "ShiftLeft",
  "ShiftRight",
  "AltLeft",
  "AltRight",
  "MetaLeft",
  "MetaRight",
]);

const MOUSE_BUTTON_CODE: Record<number, string> = {
  0: "Mouse1",
  2: "Mouse2",
  1: "Mouse3",
  3: "Mouse4",
  4: "Mouse5",
};

function buildAccel(
  parts: { ctrl: boolean; shift: boolean; alt: boolean },
  code: string,
): string {
  const out: string[] = [];
  if (parts.ctrl) out.push("Ctrl");
  if (parts.shift) out.push("Shift");
  if (parts.alt) out.push("Alt");
  out.push(code);
  return out.join("+");
}

export function HotkeyRecorder({
  value,
  onChange,
  onReset,
  disabled,
}: {
  value: string;
  onChange: (accel: string) => Promise<void>;
  /** "Nulstil" tegnes kun naar den er givet — se handlingslinjen nedenfor. */
  onReset?: () => void;
  disabled?: boolean;
}) {
  const [listening, setListening] = useState(false);
  const [parts, setParts] = useState<string[]>(() => value.split("+"));
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const suspendedRef = useRef(false);

  useEffect(() => {
    let alive = true;
    void hotkeyLabelParts(value).then((next) => {
      if (alive) setParts(next);
    });
    return () => {
      alive = false;
    };
  }, [value]);

  const setSuspended = useCallback(async (suspended: boolean) => {
    if (suspendedRef.current === suspended) return;
    suspendedRef.current = suspended;
    try {
      await invoke("suspend_wake_hotkey", { suspended });
    } catch {
      // I browser-tests findes Rust-polleren ikke.
    }
  }, []);

  useEffect(
    () => () => {
      void setSuspended(false);
    },
    [setSuspended],
  );

  const stop = useCallback(() => {
    setListening(false);
    void setSuspended(false);
  }, [setSuspended]);

  const commit = useCallback(
    async (accel: string) => {
      try {
        parseAccelerator(accel);
      } catch (caught) {
        setError(String(caught instanceof Error ? caught.message : caught));
        return;
      }
      setError(null);
      await setSuspended(false);
      setListening(false);
      await onChange(accel);
    },
    [onChange, setSuspended],
  );

  useEffect(() => {
    if (!listening) return;

    const onKeyDown = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopImmediatePropagation();
      if (event.code === "AltRight") {
        setNote(
          "Alt betyder venstre Alt — højre Alt er AltGr på danske og de fleste europæiske layouts.",
        );
      }
      if (MODIFIER_CODES.has(event.code)) return;
      if (
        event.code === "Escape" &&
        !event.ctrlKey &&
        !event.shiftKey &&
        !event.altKey &&
        !event.metaKey
      ) {
        stop();
        return;
      }
      void commit(
        buildAccel(
          {
            ctrl: event.ctrlKey || event.metaKey,
            shift: event.shiftKey,
            alt: event.altKey,
          },
          event.code,
        ),
      );
    };

    const onMouseDown = (event: MouseEvent) => {
      const code = MOUSE_BUTTON_CODE[event.button];
      if (code === undefined) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      void commit(
        buildAccel(
          {
            ctrl: event.ctrlKey || event.metaKey,
            shift: event.shiftKey,
            alt: event.altKey,
          },
          code,
        ),
      );
    };

    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("mousedown", onMouseDown, true);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("mousedown", onMouseDown, true);
    };
  }, [commit, listening, stop]);

  const begin = () => {
    setError(null);
    setNote(null);
    void setSuspended(true).then(() => setListening(true));
  };

  return (
    <div>
      {/* Selve visningen ER knappen. Den beholder `onMouseUp` og ikke
          `onClick`: optageren lytter paa `mousedown` i capture, mens den
          lytter, og et museklik der startede optagelsen ville ellers kunne
          naa at blive laest som Mouse1-bindingen. */}
      <button
        type="button"
        data-hotkey-change
        aria-label={listening ? "Optager genvej" : "Skift genvej"}
        onMouseUp={begin}
        disabled={disabled || listening}
        style={{
          ...styles.display,
          ...(listening ? styles.displayListening : null),
        }}
      >
        {listening ? (
          <span data-hotkey-value style={styles.listening}>
            Tryk kombinationen nu…
          </span>
        ) : (
          <span data-hotkey-value style={styles.caps}>
            {parts.map((part, index) => (
              <span key={`${part}-${index}`} style={styles.cap}>
                {part}
              </span>
            ))}
          </span>
        )}
      </button>

      <div style={styles.actions}>
        <span>{listening ? "Esc afbryder" : "Tryk for at ændre"}</span>
        {onReset !== undefined && (
          <>
            <span aria-hidden="true">·</span>
            <button
              type="button"
              data-hotkey-reset
              onClick={onReset}
              disabled={disabled || listening}
              style={styles.linkButton}
            >
              Nulstil
            </button>
          </>
        )}
      </div>

      {/* Reglerne staar kun mens der optages. De er en instruktion til den
          handling brugeren er i gang med — permanent er de tre linjer graa
          tekst, man holder op med at se. */}
      {listening && (
        <div style={styles.hint}>
          Alt du kan skrive kræver mindst én modifier. F1-F12 og musetaster må
          stå alene. Shift+Esc forlader type-mode og er ikke ledig.
        </div>
      )}
      {note !== null && <div style={styles.hint}>{note}</div>}
      {error !== null && <div style={styles.error}>{error}</div>}
    </div>
  );
}

const styles: Record<string, CSSProperties> = {
  display: {
    display: "flex",
    width: "100%",
    minHeight: 52,
    alignItems: "center",
    justifyContent: "center",
    gap: 6,
    boxSizing: "border-box",
    border: "1px solid rgba(222, 241, 255, 0.18)",
    borderRadius: 10,
    padding: "10px 12px",
    background:
      "radial-gradient(90% 120% at 0% 0%, rgba(113, 180, 225, 0.13), transparent 58%), linear-gradient(145deg, rgba(24, 46, 70, 0.4), rgba(2, 9, 20, 0.36))",
    color: "#edf5fc",
    font: "inherit",
    cursor: "pointer",
    transition: "border-color 140ms ease, background 140ms ease",
  },
  displayListening: {
    borderColor: "rgba(122, 182, 232, 0.6)",
    background: "rgba(122, 182, 232, 0.12)",
    cursor: "default",
  },
  caps: { display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap" },
  cap: {
    display: "inline-flex",
    minWidth: 30,
    alignItems: "center",
    justifyContent: "center",
    border: "1px solid rgba(222, 241, 255, 0.22)",
    borderRadius: 7,
    padding: "5px 9px",
    background: "rgba(8, 18, 32, 0.66)",
    boxShadow: "inset 0 -2px 0 rgba(0, 6, 16, 0.45)",
    color: "#edf5fc",
    fontFamily: '"Cascadia Mono", monospace',
    fontSize: 12,
    fontWeight: 600,
  },
  listening: { color: "#9fd0ff", fontSize: 12 },
  actions: {
    display: "flex",
    alignItems: "center",
    gap: 6,
    marginTop: 7,
    color: "#718297",
    fontSize: 11,
  },
  linkButton: {
    border: 0,
    borderRadius: 5,
    padding: 0,
    background: "transparent",
    color: "#8ea0b5",
    font: "inherit",
    fontSize: 11,
    textDecoration: "underline",
    textUnderlineOffset: 2,
    cursor: "pointer",
  },
  hint: { marginTop: 8, fontSize: 11, color: "#718297", lineHeight: 1.5 },
  error: { marginTop: 8, fontSize: 11, color: "#ff9b93" },
};
