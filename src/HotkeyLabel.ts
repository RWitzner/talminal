const FALLBACK: Record<string, string> = {
  Space: "Mellemrum",
  Escape: "Esc",
  Enter: "Enter",
  Tab: "Tab",
  Backspace: "Backspace",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Mouse1: "Mouse 1 (primær)",
  Mouse2: "Mouse 2 (sekundær)",
  Mouse3: "Mouse 3 (midter)",
  Mouse4: "Mouse 4 (tilbage)",
  Mouse5: "Mouse 5 (frem)",
};

async function codeLabel(code: string): Promise<string> {
  if (FALLBACK[code] !== undefined) return FALLBACK[code];
  const keyboard = (navigator as Navigator & {
    keyboard?: { getLayoutMap?: () => Promise<Map<string, string>> };
  }).keyboard;
  try {
    const map = await keyboard?.getLayoutMap?.();
    const label = map?.get(code);
    if (label) return label.toLocaleUpperCase();
  } catch {
    // Ikke tilgængeligt i denne WebView2 — fald igennem.
  }
  if (/^Key[A-Z]$/u.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/u.test(code)) return code.slice(5);
  return code;
}

/**
 * Ét led pr. tast — "Ctrl+Shift+Space" -> ["Ctrl", "Shift", "Mellemrum"].
 *
 * Delene eksponeres for sig, fordi HotkeyRecorder tegner hver tast som sin
 * egen tastekap. Splittes den sammenfoejede streng paa " + " igen, gaetter man
 * paa et separatortegn der ogsaa kan optraede i en tastes NAVN (fx layouts
 * hvor `getLayoutMap` giver "+").
 */
export async function hotkeyLabelParts(accel: string): Promise<string[]> {
  const parts = accel.split("+").map((part) => part.trim()).filter(Boolean);
  const out: string[] = [];
  for (const part of parts) {
    const lower = part.toLocaleLowerCase();
    if (lower === "ctrl" || lower === "cmdorctrl" || lower === "control") {
      out.push("Ctrl");
    } else if (lower === "shift") {
      out.push("Shift");
    } else if (lower === "alt" || lower === "option") {
      out.push("Alt");
    } else {
      out.push(await codeLabel(part));
    }
  }
  return out;
}

export async function hotkeyLabel(accel: string): Promise<string> {
  return (await hotkeyLabelParts(accel)).join(" + ");
}
