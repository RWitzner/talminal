// Delte TS-typer - spejler Rust-siden (src-tauri/src/main.rs).
// Feltnavne er snake_case praecis som serde serialiserer dem.

import type { Viewport } from "./viewport";

/** Rust: registry::CardInfo (Task 5-kontrakten, 1:1) PLUS `restore_action` —
 *  Rust-feltet lander i Task 11 i SAMME boelge; typen skrives komplet NU saa
 *  wire og typer ikke divergerer (plan Task 9). Indtil Task 11 er merget er
 *  feltet `undefined` paa wiren — ingen runtime-forbruger foer Task 10
 *  (restore-badgen), saa hullet er harmloest og lukkes i boelgens gate. */
interface CardInfoBase {
  number: number;
  name: string;
  cwd: string;
  profile: string;
  running: boolean;
  exited: number | null;
  restore_action: "resume" | "fresh_shared_cwd" | "fresh" | null;
}

/** Rust: registry::CardInfo — kind-diskrimineret union (browser-kort-sporet).
 *  Terminal-kort: opened_by/url/title er null paa wiren.
 *  Browser-kort: cwd/profile er tomme strenge; running = webview alive. */
export interface TerminalCardInfo extends CardInfoBase {
  kind: "terminal";
  opened_by: null;
  url: null;
  title: null;
}

export interface BrowserCardInfo extends CardInfoBase {
  kind: "browser";
  opened_by: string | null;
  url: string;
  title: string;
}

/** Rust: registry::CardInfo med Chat-backend. cwd/profile er tomme strenge;
 *  thread_id/purpose er kun sat for chat-kort.
 *
 *  Tilfoej IKKE thread_id/purpose til de to andre grene: felterne ville vaere
 *  paakraevede og braekke hver eksisterende kort-fixture i vitest-suiten.
 *  Unionen er diskrimineret paa `kind` alene, og at card.thread_id kun kan
 *  laeses efter isChatCard(card) er praecis den type-sikkerhed vi vil have. */
export interface ChatCardInfo extends CardInfoBase {
  kind: "chat";
  opened_by: null;
  url: null;
  title: null;
  thread_id: string;
  purpose: string;
}

export type CardInfo = TerminalCardInfo | BrowserCardInfo | ChatCardInfo;

export function isBrowserCard(card: CardInfo): card is BrowserCardInfo {
  return card.kind === "browser";
}

export function isChatCard(card: CardInfo): card is ChatCardInfo {
  return card.kind === "chat";
}

/** Rust: workspace::WorkspaceCard — get_workspace-wiren (Task 6).
 *  get_workspace er frontendens ENESTE kilde til gemt geometri/viewport. */
export interface WorkspaceCard {
  number: number;
  name: string;
  cwd: string;
  profile: string;
  command: string | null;
  x: number;
  y: number;
  w: number;
  h: number;
  last_active_at: string | null;
}

/** Rust: workspace::Settings — hotkeys på get_workspace-wiren (globalt
 *  settings.json; B-light T4). */
export interface WorkspaceSettings {
  ptt_hotkey: string;
  exit_type_mode_hotkey: string;
  voice_engine: string;
  wallpaper: string;
  default_agent: string;
  stt_provider?: string;
  routing_provider?: string;
}

export interface SttRoute {
  slug: string;
  label: string;
  endpoint: string;
  model: string;
  supports_partials: boolean;
  supports_domain_prompt: boolean;
  key_slot: string;
}

export interface RouterRoute {
  slug: string;
  label: string;
  endpoint: string;
  model: string;
  decoration: "none" | "vercel_gateway";
  hedge: boolean;
  key_slot: string;
}

export interface VoiceRoutes {
  stt: SttRoute;
  routing: RouterRoute;
}

/** Rust: workspace::WorkspaceResponse — returneres af get_workspace
 *  (persist-fil uden settings + load_settings() komponeret). */
export interface WorkspaceResponse {
  schema_version: number;
  next_card_number: number;
  viewport: Viewport;
  settings: WorkspaceSettings;
  voice_routes: VoiceRoutes;
  /** Sat når den tolerante læse-side måtte falde tilbage til en default. */
  settings_warning: string | null;
  cards: WorkspaceCard[];
}

/** @deprecated Alias — wire-typen hedder WorkspaceResponse (B-light T4). */
export type WorkspaceFile = WorkspaceResponse;

/** Kant-farvepalet (MVP-pathen, Task 4): farven er en funktion af lokal
 *  running/exited alene — se colors.ts. Pause-blaa og attention-gul udgik
 *  sammen med presence-laget. */
export type CardColor = "green" | "red" | "neutral" | "gray";

/** Rust: CardView { name, cwd } - returneres af get_cards. */
export interface CardView {
  name: string;
  cwd: string;
}

/** Rust: CardsStatus { path, missing, error } - returneres af get_cards_status
 *  (tomt-grid-grenen - missing => vis stien, error => vis den reelle
 *  fejltekst). */
export interface CardsStatus {
  path: string;
  missing: boolean;
  error: string | null;
}

/** Rust: CardState { running, exited, owner, epoch } - returneres af
 *  get_card_state (fix F10: mount-hydrering — pty'erne overlever
 *  webview-reload, lokal JS-state goer ikke).
 *  NB: `owner`/`epoch` er del af den FROSNE IPC-kontrakt (Global Constraints)
 *  og findes stadig paa wiren, men frontenden IGNORERER dem — supervision er
 *  parkeret bag cargo-featuren; default-state leverer altid
 *  owner="persona", epoch=0. */
export interface CardState {
  running: boolean;
  exited: number | null;
  owner: string;
  epoch: number;
}

/** Tauri-event "pty-output": raa pty-bytes, base64-encodet. */
export interface PtyOutputEvent {
  name: string;
  data_b64: string;
}

/** Tauri-event "card-exit": child-exit observeret (reader melder). */
export interface CardExitEvent {
  name: string;
}
