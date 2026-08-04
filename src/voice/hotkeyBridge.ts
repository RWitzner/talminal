// Arbitrationen mellem de TO lag der kan se det samme tastetryk: DOM'ens
// keydown/keyup og Rust-pollerens niveau-samplede kanter.
//
// Logikken stod tidligere inline i App.tsx som ét sæt `nativePtt` /
// `nativePttReady` / `domPttActive`-variabler. Den er flyttet hertil af to
// grunde, og den anden er den vigtigste:
//
// 1. Der er nu TO genveje (PTT og diktering), og hver skal have sit eget sæt
//    flag. Kopieret ville den subtile selvhealing nedenfor findes to steder.
// 2. Den var ikke daekket af en eneste test. Det er den nu — og netop denne
//    kode er der hvor "doede tryk" boede i tre omgange (2026-07-03, -19, -20).
//
// ARBITRATIONEN, i ord:
//
//   Polleren har PRIMATET, fordi den er fysisk sandhed: den laeser tastens
//   niveau og kan ikke miste et event. Men den er ikke klar med det samme
//   (kommandoen skal naa Rust), og DOM'en er den eneste der kan SUPPRESSE
//   tasten saa den ikke ogsaa naar terminalen. Derfor: DOM'en foerer indtil
//   polleren har bevist at den fyrer, hvorefter den traeder tilbage.
//
//   Selvhealingen er det led der koster mest at genopfinde: en RELEASE-kant
//   fra polleren MENS DOM-holdet staar aktivt beviser at DOM'ens keyup gik
//   tabt (WebView2-leverance). Uden healing stod `domActive` laast for evigt,
//   og alle senere native kanter doede — symptomet "flere doede tryk i traek
//   indtil naeste blur". En PRESS-kant under aktivt DOM-hold kan derimod KUN
//   vaere samme holds ≤15 ms-duplikat, og skal fortsat ignoreres.

export type HotkeyEdge = "press" | "release";

export interface HotkeyBridge {
  /** DOM'ens keydown for komboen. */
  domPress(): void;
  /** DOM'ens keyup for komboen. */
  domRelease(): void;
  /** En kant fra Rust-polleren. */
  nativeEdge(edge: HotkeyEdge): void;
  /** Kaldes naar `configure_*_hotkey` er kvitteret: polleren fyrer nu. */
  markNativeReady(): void;
  /** Bruges af blur-vejen til at afgoere om der overhovedet er et hold. */
  isDomActive(): boolean;
}

export function createHotkeyBridge(deps: {
  onPress(): void;
  onRelease(): void;
  /** Diagnostik-kanalen. Beskeder er de samme strenge som foer flytningen,
   *  saa eksisterende fejlsoegning i konsollen stadig finder dem. */
  onDebug?(message: string): void;
  onWarn?(message: string): void;
}): HotkeyBridge {
  let nativeHasPrimacy = false;
  let nativeReady = false;
  let domActive = false;

  return {
    domPress() {
      if (nativeHasPrimacy) {
        // Doede-tryk-diagnostik: DOM'en saa komboen, men polleren har
        // primatet. Fyrer polleren ikke tilsvarende, er Rust-gaten eller
        // -leverancen synderen (se wake_hotkey.rs-loggen).
        deps.onDebug?.("voice.ptt.dom_saw_combo_native_primacy");
        return;
      }
      domActive = true;
      deps.onPress();
    },

    domRelease() {
      if (nativeHasPrimacy) return;
      domActive = false;
      deps.onRelease();
      if (nativeReady) nativeHasPrimacy = true;
    },

    nativeEdge(edge) {
      if (domActive) {
        if (edge === "release") {
          deps.onWarn?.("voice.ptt.dom_hold_stale_selfhealed");
          domActive = false;
          nativeHasPrimacy = true;
          deps.onRelease();
        }
        return;
      }
      nativeHasPrimacy = true;
      if (edge === "press") deps.onPress();
      else deps.onRelease();
    },

    markNativeReady() {
      nativeReady = true;
      // Gaten er domActive og IKKE nativeHasPrimacy: staar der et DOM-hold
      // lige nu, skal DET hold koere faerdigt i DOM'en, og foerste native
      // event uden aktivt hold kraever saa selv primatet.
      if (!domActive) nativeHasPrimacy = true;
    },

    isDomActive() {
      return domActive;
    },
  };
}
