// Hedge-hjaelperen bor i sin EGEN fil, fordi run.mjs kalder main() ved import:
// en test der bare importerede hedgedCall derfra ville fyre hele Gate 1 af —
// rigtige API-kald, betalte tokens og en exitCode 1 midt i suiten. En argv-vagt
// i run.mjs loeser det ikke: runnerne koeres via `npx vite-node` (run.mjs
// importerer TS direkte fra ../src/voice/), og dér peger
// process.argv[1] paa vite-node's egen CLI. Vagten ville altsaa vaere
// falsk, main() aldrig koere, og Gate 1 slukke LYDLOEST med exit 0 og nul
// output. En gate der tavst bestaar er vaerre end ingen gate.

const ROUTER_HEDGE_DELAY_MS = 1_200;

// Spejler Rust-proxyens staggered hedge (tail-spikes paa 2-6 s ses selv paa
// varme forbindelser): skud nr. 2 affyres foerst naar det foerste har haengt
// i delay-vinduet. Foerste Ok vinder; afvisning sker KUN naar begge skud er
// faldet, og saa med det primaere skuds fejl (hedge-skuddets er en foelgefejl).
// Byttehandelen: haenger det primaere skud uden hverken at svare eller fejle
// mens hedgen falder, afvises den ydre promise ALDRIG (foer faldt kaldet straks
// paa hedge-fejlen) — kaldet venter i stedet paa undicis 300 s-loft.
export function hedgedCall(fire, delayMs = ROUTER_HEDGE_DELAY_MS) {
  return new Promise((resolve, reject) => {
    let settled = false;
    // Hvert skud gemmer SIN egen fejl, og vi taeller hvor mange der er faldet.
    // Afviste vi paa den foerste fejl der kom ind — uanset skud — ville et
    // senere Ok fra det andet skud blive slugt af settled-vagten: en hurtig
    // 429 paa hedgen kunne dermed spolere netop den langsomme tail-request
    // hedgen findes for at redde.
    let primaryError = null;
    let hedgeError = null;
    let failures = 0;
    const settleOk = (value) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      resolve(value);
    };
    const settleErr = (error, isPrimary) => {
      if (settled) return;
      failures += 1;
      if (isPrimary) primaryError = error;
      else hedgeError = error;
      if (failures < 2) return;
      settled = true;
      clearTimeout(timer);
      reject(primaryError ?? hedgeError);
    };
    const timer = setTimeout(() => {
      if (settled) return;
      fire().then(settleOk, (error) => settleErr(error, false));
    }, delayMs);
    fire().then(settleOk, (error) => settleErr(error, true));
  });
}
