// Kategori-listen for indstillings-vinduet.
//
// HVORFOR EGET MODUL: `SettingsWindow` tegner navigationen og skal derfor kende
// listen med det samme, mens `Settings` selv er lazy-loaded (App.tsx' §3-gevinst
// er at chunken ikke parses paa den opstarts-kritiske vej). Laa listen i
// `Settings.tsx`, ville vinduets statiske import traekke hele chunken ind i
// hovedbundtet igen, og lazy-loadingen ville vaere en attrap.
//
// Modulet maa af samme grund ikke faa afhaengigheder ud over typer.

export const SETTINGS_CATEGORIES = [
  { id: "voice", label: "Stemme" },
  { id: "keys", label: "Nøgler" },
  { id: "routing", label: "Model & routing" },
  { id: "agents", label: "Agenter" },
  { id: "appearance", label: "Udseende" },
] as const;

export type SettingsCategory = (typeof SETTINGS_CATEGORIES)[number]["id"];

export const DEFAULT_SETTINGS_CATEGORY: SettingsCategory = "voice";
