// Ren planlægning af install/uninstall — testbar uden fs.

export function isTapCommand(command) {
  return typeof command === "string"
    && command.includes("Talminal")
    && command.includes("tap.mjs");
}

export function planInstall(settings, nodePath, tapPath, existingDelegate) {
  const current = settings?.statusLine?.command ?? null;
  const tapCommand = `"${nodePath}" "${tapPath}"`;
  if (isTapCommand(current)) {
    if (existingDelegate === undefined) {
      // undefined = tap-config-FILEN mangler; null = filen findes med legitim
      // delegate:null (installation uden præeksisterende statusline). Kun det
      // første er tab af kæden til den oprindelige statusline (review-fund 2):
      // at fortsætte ville skrive delegate:null og kappe den permanent. Stop
      // og lad ejeren gendanne fra .bak.
      return {
        abort:
          "tap'en er aktiv i settings.json men tap-config.json mangler (selve FILEN er væk) — kun backup-vejen gendanner den oprindelige statusLine: gendan fra settings.json.talminal-tap.bak. (--uninstall er ingen udvej: den afbryder på samme tilstand, fordi den ville FJERNE statusLine i stedet for at gendanne den.)",
      };
    }
    return {
      settings: { ...settings, statusLine: { type: "command", command: tapCommand } },
      delegate: existingDelegate,
      alreadyInstalled: true,
    };
  }
  return {
    settings: { ...settings, statusLine: { type: "command", command: tapCommand } },
    delegate: current,
    alreadyInstalled: false,
  };
}

export function planUninstall(settings, delegate) {
  const next = { ...settings };
  if (typeof delegate === "string" && delegate.trim() !== "") {
    next.statusLine = { type: "command", command: delegate };
  } else {
    delete next.statusLine;
  }
  return next;
}
