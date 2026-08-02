// Udklipsholder-skrivning (spec § 2.2). Kun skrivning: LAESNING gaar aldrig
// gennem navigator.clipboard.readText, fordi den kraever clipboard-read, som
// WebView2 haandterer med en permission-prompt vi ikke har en handler til.
// Indsaet loeses i stedet ved at lade browserens egen kommando koere — se
// terminalClipboard.ts.

export async function writeClipboardText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // Falder igennem: en afvist Async Clipboard er praecis det tilfaelde
    // fallbacken findes for.
  }
  return writeViaExecCommand(text);
}

/** Den gamle vej. Den er stadig den mest kompatible, og den er synkron —
 *  ingen permission, ingen promise der kan afvises tavst. */
function writeViaExecCommand(text: string): boolean {
  const scratch = document.createElement("textarea");
  scratch.value = text;
  scratch.setAttribute("aria-hidden", "true");
  scratch.style.position = "fixed";
  scratch.style.top = "-9999px";
  scratch.style.opacity = "0";
  document.body.append(scratch);
  // Fokus skal tilbage til terminalen bagefter — ellers ville en kopiering
  // koste ejeren sit skrivefelt.
  const previouslyFocused = document.activeElement;
  try {
    scratch.select();
    return document.execCommand("copy");
  } catch {
    return false;
  } finally {
    scratch.remove();
    if (previouslyFocused instanceof HTMLElement) previouslyFocused.focus();
  }
}
