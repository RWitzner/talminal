//! workspace.json-persistens + viewport + globale settings (Task 6/14;
//! B-light T4 flyttede settings til `settings.json` under global_base).
//!
//! Design (Task 6 + B-light T4):
//! - `workspace.json` bor i `cards::talminal_base()` (TALMINAL_HOME-override
//!   virker dermed i tests) og er kort-persistensens SANDHEDSKILDE efter
//!   foerste indlaesning. ALLE skrivninger er atomiske (tmp+rename — samme
//!   moenster som signals.rs). Settings bor GLOBALT i
//!   `project::global_base()/settings.json` (delt paa tvaers af projekter).
//! - EJER-AMENDMENT (2026-07-19): canvas starter ALTID tomt — filen laeses
//!   ALDRIG ved launch (ingen kort-restore, ingen cards.toml-import, taeller
//!   forfra). workspace.json er dermed KUN i-sessions-persist; nummer-
//!   monotonien gaelder inden for én session.
//! - Persist-kadence (bindende): debounced <=500 ms efter geometri-/viewport-
//!   aendring; synkront ved create/close; ved ExitRequested-teardown
//!   (persist_now fra main.rs).
//! - Modul-state er en global singleton (samme moenster som registry.rs);
//!   laasen holdes over de smaa fil-skrivninger (simpelt og korrekt — filen
//!   er lille, og interleaving med create/close-persist undgaas).

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread;
use std::time::Duration;
#[cfg(feature = "perf-trace")]
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::cards;
use crate::project;
use crate::registry;

/// Persist-formen af workspace.json (B-light T4: UDEN settings-felt).
/// Gamle filer med settings-felt loader stadig — serde ignorerer ukendte
/// felter (INGEN `deny_unknown_fields`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub schema_version: u32,
    pub next_card_number: u32,
    pub viewport: Viewport,
    pub cards: Vec<WorkspaceCard>,
}

/// Wire-formen returneret af `get_workspace` — komponeret af persist-filen +
/// `load_settings()`. Frontenden ser uændret shape (settings stadig på wiren).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkspaceResponse {
    pub schema_version: u32,
    pub next_card_number: u32,
    pub viewport: Viewport,
    pub settings: Settings,
    pub voice_routes: VoiceRoutes,
    pub settings_warning: Option<String>,
    pub cards: Vec<WorkspaceCard>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoiceRoutes {
    pub stt: &'static crate::providers::SttRoute,
    pub routing: &'static crate::providers::RouterRoute,
}

pub fn resolve_voice_routes(settings: &Settings) -> VoiceRoutes {
    VoiceRoutes {
        stt: crate::providers::stt_route(&settings.stt_provider).unwrap_or_else(|| {
            crate::providers::stt_route(crate::providers::DEFAULT_STT_SLUG)
                .expect("default stt route")
        }),
        routing: crate::providers::router_route(&settings.routing_provider).unwrap_or_else(|| {
            crate::providers::router_route(crate::providers::DEFAULT_ROUTER_SLUG)
                .expect("default router route")
        }),
    }
}

/// Hotkey-bindings (Task 14 / B-light T4). IKKE secrets — bor globalt i
/// `settings.json` og læses af frontenden via `get_workspace` (wire);
/// skrives via `set_settings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Push-to-talk (hold nede = optag). Accelerator-streng i
    /// tauri-plugin-global-shortcut-format.
    pub ptt_hotkey: String,
    /// Exit fra type-mode (planens spec-afvigelse: "Shift+Escape" — enkelt-Esc
    /// gaar ALTID til terminalen, og Esc-Esc er en CC-binding).
    pub exit_type_mode_hotkey: String,
    /// Voice-implementation. Feltet bevares for serde-kompatibilitet med
    /// eksisterende settings.json-filer, men normaliseres altid til pipeline.
    #[serde(default = "default_voice_engine")]
    pub voice_engine: String,
    /// Bundle-slug for canvas-baggrunden. Manglende felt i ældre
    /// settings.json-filer falder tilbage til standardbaggrunden.
    #[serde(default = "default_wallpaper")]
    pub wallpaper: String,
    /// Agent-slug brugt af `create_card_persisted` naar kaldet ikke selv
    /// angiver en profil (K2-choke-point). Manglende felt i aeldre
    /// settings.json-filer falder tilbage til "claude".
    #[serde(default = "default_agent")]
    pub default_agent: String,
    /// Rute-slug for tale-til-tekst. Se `providers::STT_ROUTES`.
    #[serde(default = "default_stt_provider")]
    pub stt_provider: String,
    /// Rute-slug for kommando-routing. Se `providers::ROUTER_ROUTES`.
    #[serde(default = "default_routing_provider")]
    pub routing_provider: String,
}

/// Default-PTT i "CmdOrCtrl+Space"-klassen (planens krav: implementer vaelger
/// uden Windows-reserverede chords). Valget er `CmdOrCtrl+Shift+Space` fordi:
/// - `Win+Space` er OS-reserveret (input-sprog-skift) og `Alt+Space` ligesaa
///   (vinduets systemmenu) — begge udelukket.
/// - Bar `Ctrl+Space` er ikke OS-reserveret, men er IME-toggle (CJK) og
///   completion-chord i stort set alle editorer — en GLOBAL shortcut ville
///   skygge den systemvidt. Shift-varianten er fri i Windows og de gaengse
///   editorer og kan holdes behageligt nede med én haand (PTT er hold-nede).
pub const DEFAULT_PTT_HOTKEY: &str = "CmdOrCtrl+Shift+Space";

/// Bindende default fra planens spec-afvigelses-journal.
pub const DEFAULT_EXIT_TYPE_MODE_HOTKEY: &str = "Shift+Escape";

pub const WALLPAPER_SLUGS: &[&str] = &[
    "liquid-only",
    "blue-folds",
    "ember-dunes",
    "violet-tide",
    "jade-ripples",
    "aurora-mist",
    "golden-strata",
];

/// Kendte agent-slugs (T1's profiles-modul kender begge). Case-sensitivt
/// lowercase — se `normalize_settings` for den tolerante laese-side.
pub const AGENT_SLUGS: &[&str] = &["claude", "codex"];

fn default_voice_engine() -> String {
    "pipeline".to_string()
}

fn default_wallpaper() -> String {
    "blue-folds".to_string()
}

fn default_agent() -> String {
    "claude".to_string()
}

fn default_stt_provider() -> String {
    crate::providers::DEFAULT_STT_SLUG.to_string()
}

fn default_routing_provider() -> String {
    crate::providers::DEFAULT_ROUTER_SLUG.to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ptt_hotkey: DEFAULT_PTT_HOTKEY.to_string(),
            exit_type_mode_hotkey: DEFAULT_EXIT_TYPE_MODE_HOTKEY.to_string(),
            voice_engine: default_voice_engine(),
            wallpaper: default_wallpaper(),
            default_agent: default_agent(),
            stt_provider: default_stt_provider(),
            routing_provider: default_routing_provider(),
        }
    }
}

/// TOLERANT laese-side (spec §3, wallpaper-moenstret; GPT-review B6): en
/// ukendt/tom/whitespace/forkert-case `default_agent` i settings.json maa
/// ALDRIG blokere senere `create_card_persisted`-kald — den normaliseres
/// stille til "claude" (+ eprintln-advarsel). Skrive-siden (`set_settings`)
/// forbliver STRENG og afviser samme input med en Err.
pub fn normalize_settings(mut s: Settings) -> Settings {
    // Slut-review fix 2: den trimmede vaerdi tilskrives paa BEGGE grene. Foer
    // skrev kun fallback-grenen tilbage, saa en PADDED men gyldig slug
    // (" codex ") slap igennem allowlist-checket UDEN at blive trimmet — og
    // whitespacen naaede hele vejen til `profiles::profile(" codex ")` => None
    // => "unknown profile: ", som braekker HVER kortoprettelse uden eksplicit
    // profil (create_card_persisted's utrimmede unwrap_or_else-gren).
    let trimmed = s.default_agent.trim().to_string();
    s.default_agent = if AGENT_SLUGS.contains(&trimmed.as_str()) {
        trimmed
    } else {
        eprintln!(
            "[canvas] settings.json default_agent={:?} er ukendt — falder tilbage til \"claude\"",
            s.default_agent
        );
        default_agent()
    };
    // Realtime-motoren forlod appen (spec 2026-07-28 §0). En eksisterende
    // settings.json med "realtime" maa ikke braekke opstarten, saa den
    // normaliseres stille til den eneste tilbagevaerende motor.
    let engine = s.voice_engine.trim().to_string();
    s.voice_engine = if engine == "pipeline" {
        engine
    } else {
        if !engine.is_empty() {
            eprintln!(
                "[canvas] settings.json voice_engine={:?} er ikke laengere en mulighed — bruger \"pipeline\"",
                s.voice_engine
            );
        }
        default_voice_engine()
    };
    let trimmed = s.stt_provider.trim().to_string();
    s.stt_provider = if crate::providers::stt_route(&trimmed).is_some() {
        trimmed
    } else {
        eprintln!(
            "[canvas] settings.json stt_provider={:?} er ukendt — falder tilbage til {:?}",
            s.stt_provider,
            crate::providers::DEFAULT_STT_SLUG
        );
        default_stt_provider()
    };

    let trimmed = s.routing_provider.trim().to_string();
    s.routing_provider = if crate::providers::router_route(&trimmed).is_some() {
        trimmed
    } else {
        eprintln!(
            "[canvas] settings.json routing_provider={:?} er ukendt — falder tilbage til {:?}",
            s.routing_provider,
            crate::providers::DEFAULT_ROUTER_SLUG
        );
        default_routing_provider()
    };
    s
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceCard {
    pub number: u32,
    pub name: String,
    pub cwd: String,
    pub profile: String,
    pub command: Option<String>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub last_active_at: Option<String>,
}

/// Debounce-vindue for geometri-/viewport-persist (kravet er <=500 ms).
pub const DEBOUNCE_MS: u64 = 300;

// ---------------------------------------------------------------------------
// Modul-state
// ---------------------------------------------------------------------------

enum LoadState {
    /// startup_load er ikke koert (fx andre integrationstests) — kommandoerne
    /// fejler beskrivende, persist-hooks er no-ops.
    NotLoaded,
    Loaded,
    /// Historisk gren (pre-tom-start: defekt workspace.json). Konstrueres
    /// ikke laengere — startup laeser aldrig filen — men bevares saa match-
    /// fladerne ikke skal genforhandles, hvis en load-sti genopstaar.
    #[allow(dead_code)]
    Failed(String),
}

struct WsState {
    load: LoadState,
    file: WorkspaceFile,
    dirty: bool,
    flush_scheduled: bool,
}

static STATE: LazyLock<Mutex<WsState>> = LazyLock::new(|| {
    Mutex::new(WsState {
        load: LoadState::NotLoaded,
        file: empty_file(),
        dirty: false,
        flush_scheduled: false,
    })
});

/// Create er eksklusiv mod alle closes, saa et nyt `card-1` aldrig kan
/// ABA-krydse registry-reset/workspace-fjernelsen. Close-transaktioner er
/// derimod indbyrdes kompatible: registry- og workspace-lagets egne korte
/// laase beskytter mutationerne, mens deres langsomme PTY-teardowns overlapper.
static CARD_MUTATIONS: LazyLock<RwLock<()>> = LazyLock::new(|| RwLock::new(()));

fn lock_create_mutations() -> RwLockWriteGuard<'static, ()> {
    CARD_MUTATIONS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_close_mutations() -> RwLockReadGuard<'static, ()> {
    CARD_MUTATIONS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn committed_sequence_reset(workspace_reset: bool) -> Result<bool, String> {
    if !workspace_reset {
        return Ok(false);
    }
    registry::sequence_reset_ready()
}

fn empty_file() -> WorkspaceFile {
    WorkspaceFile {
        schema_version: 1,
        next_card_number: 1,
        viewport: Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        },
        cards: Vec::new(),
    }
}

/// `project::global_base()/settings.json` — delt på tværs af projekter.
pub fn settings_path() -> PathBuf {
    project::global_base().join("settings.json")
}

/// Fravær/parse-fejl → `Settings::default()` (bevidst mildere end project.json,
/// hvor korruption er fatal — hotkeys er ufarlige defaults).
pub fn load_settings() -> Settings {
    let path = settings_path();
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Settings::default(),
    };
    let settings: Settings = serde_json::from_str(&text).unwrap_or_default();
    normalize_settings(settings)
}

pub fn load_settings_checked() -> (Settings, Option<String>) {
    let mut settings = load_settings();
    if let Err(err) = crate::wake_hotkey::parse_accelerator(&settings.ptt_hotkey) {
        let warning = format!(
            "Voice-hotkeyen i settings.json kunne ikke laeses ({err}) — bruger standarden {DEFAULT_PTT_HOTKEY}"
        );
        settings.ptt_hotkey = DEFAULT_PTT_HOTKEY.to_string();
        return (settings, Some(warning));
    }
    (settings, None)
}

/// Atomisk skrivning via T1's delte `crate::atomic::write` (unik temp + rename).
pub fn save_settings(s: &Settings) -> Result<(), String> {
    let path = settings_path();
    let mut body =
        serde_json::to_string_pretty(s).map_err(|e| format!("settings serialize failed: {e}"))?;
    body.push('\n');
    crate::atomic::write(&path, body.as_bytes()).map_err(|e| format!("settings save failed: {e}"))
}

fn workspace_path() -> PathBuf {
    cards::talminal_base().join("workspace.json")
}

/// Samme tidsformat som resten af systemet: YYYY-MM-DDTHH:MM:SS.mmmZ.
fn now_iso_z() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

fn lock_state() -> Result<MutexGuard<'static, WsState>, String> {
    STATE.lock().map_err(|e| e.to_string())
}

fn ensure_loaded(st: &WsState) -> Result<(), String> {
    match &st.load {
        LoadState::Loaded => Ok(()),
        LoadState::NotLoaded => Err("workspace not loaded".to_string()),
        LoadState::Failed(e) => Err(e.clone()),
    }
}

/// Markerer state dirty og planlaegger hoejst ÉN debounced flush ad gangen.
/// Aendringer der lander mens flush-traaden sover, coalesces ind i samme
/// skrivning (snapshottet tages foerst ved flush).
fn mark_dirty(st: &mut WsState) {
    st.dirty = true;
    if !st.flush_scheduled {
        st.flush_scheduled = true;
        thread::spawn(flush_after_debounce);
    }
}

fn flush_after_debounce() {
    thread::sleep(Duration::from_millis(DEBOUNCE_MS));
    let path = workspace_path();
    let Ok(mut st) = STATE.lock() else { return };
    st.flush_scheduled = false;
    if !matches!(st.load, LoadState::Loaded) || !st.dirty {
        return;
    }
    st.dirty = false;
    let snapshot = st.file.clone();
    if let Err(e) = save_workspace_file(&path, &snapshot) {
        // Naeste aendring eller teardown-flushen proever igen.
        st.dirty = true;
        eprintln!("[canvas] workspace debounced save failed: {e}");
    }
}

// ---------------------------------------------------------------------------
// Fil-IO (path-parameteriseret => testbar uden env/registry)
// ---------------------------------------------------------------------------

fn tmp_path(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(".tmp");
    PathBuf::from(os)
}

/// Atomisk skrivning (tmp+rename, samme moenster som signals.rs): en crash
/// mellem tmp-write og rename efterlader den gamle fil intakt, og en
/// efterladt tmp-fil overskrives bare ved naeste save (MOVEFILE_REPLACE_EXISTING).
pub fn save_workspace_file(path: &Path, file: &WorkspaceFile) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("workspace dir create failed: {e}"))?;
    }
    // Deterministisk serialisering (fast feltorden) => roundtrip er byte-stabil.
    let mut body = serde_json::to_string_pretty(file)
        .map_err(|e| format!("workspace serialize failed: {e}"))?;
    body.push('\n');
    let tmp = tmp_path(path);
    let mut f = fs::File::create(&tmp).map_err(|e| format!("workspace tmp create failed: {e}"))?;
    f.write_all(body.as_bytes())
        .map_err(|e| format!("workspace tmp write failed: {e}"))?;
    f.sync_all()
        .map_err(|e| format!("workspace tmp sync failed: {e}"))?;
    drop(f);
    fs::rename(&tmp, path).map_err(|e| format!("workspace rename failed: {e}"))
}

/// `Ok(None)` = filen findes ikke (import-grenen). `Err` = defekt fil — den
/// maa ALDRIG besvares med import-fallback (ville overskrive brugerens data).
pub fn load_workspace_file(path: &Path) -> Result<Option<WorkspaceFile>, String> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("workspace read failed: {e}")),
    };
    let file: WorkspaceFile =
        serde_json::from_str(&text).map_err(|e| format!("workspace parse failed: {e}"))?;
    if file.schema_version != 1 {
        return Err(format!(
            "workspace schema_version {} not supported (expected 1)",
            file.schema_version
        ));
    }
    Ok(Some(file))
}

// ---------------------------------------------------------------------------
// Startup: ALTID tom canvas (ejer-amendment 2026-07-19)
// ---------------------------------------------------------------------------

/// Resultat af startup_load til get_cards_status-fladen. Konstant siden
/// tom-start-semantikken (ingen import-sti) — wire-formen er bevaret.
pub struct StartupReport {
    pub cards_missing: bool,
    pub cards_error: Option<String>,
}

/// Kaldes praecis én gang fra main() FOER nogen anden registry-mutation.
/// Genkald er et vaern-no-op.
///
/// EJER-AMENDMENT (2026-07-19, B-light dogfood): canvas starter ALTID tomt —
/// workspace.json fra tidligere sessioner laeses ALDRIG (ingen kort-restore,
/// ingen cards.toml-import, taeller forfra). Filen bruges KUN som i-sessions-
/// persist og overskrives ved foerste mutation. Restore-on-launch-logikken
/// (restore.rs) er parkeret som ren logik, ikke slettet.
pub fn startup_load() -> Result<StartupReport, String> {
    let mut st = lock_state()?;
    match &st.load {
        LoadState::NotLoaded => {}
        LoadState::Loaded => {
            return Ok(StartupReport {
                cards_missing: false,
                cards_error: None,
            })
        }
        LoadState::Failed(e) => return Err(e.clone()),
    }
    st.file = empty_file();
    st.load = LoadState::Loaded;
    eprintln!("[canvas] workspace: fresh canvas (tidligere sessioner genskabes aldrig)");
    Ok(StartupReport {
        cards_missing: false,
        cards_error: None,
    })
}

/// Pladsholder-kaskade til nye/importerede kort — frontenden (Task 7) saetter
/// reel geometri via update_card_geometry.
fn default_geometry(number: u32) -> (f64, f64, f64, f64) {
    let i = (number.saturating_sub(1) % 8) as f64;
    (48.0 * i, 40.0 * i, 960.0, 640.0)
}

// ---------------------------------------------------------------------------
// Kommandoflade (main.rs' tynde wrappers kalder disse)
// ---------------------------------------------------------------------------

pub fn update_card_geometry(name: String, x: f64, y: f64, w: f64, h: f64) -> Result<(), String> {
    // serde_json skriver ikke-endelige floats som null => korrupt fil ved
    // naeste load. Afvis dem ved doeren.
    if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) {
        return Err("geometry must be finite".to_string());
    }
    if w <= 0.0 || h <= 0.0 {
        return Err("geometry w/h must be > 0".to_string());
    }
    let mut st = lock_state()?;
    ensure_loaded(&st)?;
    let card = st
        .file
        .cards
        .iter_mut()
        .find(|c| c.name == name)
        .ok_or_else(|| format!("unknown card: {name}"))?;
    card.x = x;
    card.y = y;
    card.w = w;
    card.h = h;
    mark_dirty(&mut st);
    Ok(())
}

pub fn set_viewport(x: f64, y: f64, zoom: f64) -> Result<(), String> {
    if !(x.is_finite() && y.is_finite() && zoom.is_finite()) || zoom <= 0.0 {
        return Err("viewport must be finite (zoom > 0)".to_string());
    }
    let mut st = lock_state()?;
    ensure_loaded(&st)?;
    st.file.viewport = Viewport { x, y, zoom };
    mark_dirty(&mut st);
    Ok(())
}

/// LAESE-siden — frontendens eneste kilde til geometri/viewport/last_active_at
/// + settings (wire-formen: persist-fil komponeret med `load_settings()`).
pub fn get_workspace() -> Result<WorkspaceResponse, String> {
    let st = lock_state()?;
    ensure_loaded(&st)?;
    let (settings, normalization_warning) = load_settings_checked();
    let settings_warning = normalization_warning.or_else(crate::wake_hotkey::take_layout_warning);
    let voice_routes = resolve_voice_routes(&settings);
    Ok(WorkspaceResponse {
        schema_version: st.file.schema_version,
        next_card_number: st.file.next_card_number,
        viewport: st.file.viewport,
        settings,
        voice_routes,
        settings_warning,
        cards: st.file.cards.clone(),
    })
}

/// SKRIVE-siden for settings (Task 14 / B-light T4): synkron, atomisk persist
/// til global `settings.json`. Hotkeys trimmes/valideres som hidtil, og engine
/// valideres mod den lukkede allowlist; læsning sker via `get_workspace`.
/// Wire-formen for skrive-siden. BEVIDST uden `serde(default)`: et udeladt
/// felt skal vaere en deserialiseringsfejl, ikke en stille default. `Settings`
/// selv kan derfor IKKE bruges her — den har defaults paa struct-niveau,
/// fordi laese-siden skal vaere tolerant.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsInput {
    pub ptt_hotkey: String,
    pub exit_type_mode_hotkey: String,
    pub voice_engine: String,
    pub wallpaper: String,
    pub default_agent: String,
    pub stt_provider: String,
    pub routing_provider: String,
}

pub fn set_settings(input: SettingsInput) -> Result<(), String> {
    let ptt = input.ptt_hotkey.trim().to_string();
    let exit = input.exit_type_mode_hotkey.trim().to_string();
    let engine = input.voice_engine.trim().to_string();
    let wallpaper = input.wallpaper.trim().to_string();
    let default_agent = input.default_agent.trim().to_string();
    let stt_provider = input.stt_provider.trim().to_string();
    let routing_provider = input.routing_provider.trim().to_string();
    if exit.is_empty() {
        return Err("hotkey bindings must be non-empty".to_string());
    }
    crate::wake_hotkey::parse_accelerator(&ptt)?;
    if engine != "pipeline" {
        return Err("voice_engine must be \"pipeline\"".to_string());
    }
    if !WALLPAPER_SLUGS.contains(&wallpaper.as_str()) {
        return Err(format!(
            "wallpaper must be one of: {}",
            WALLPAPER_SLUGS.join(", ")
        ));
    }
    if !AGENT_SLUGS.contains(&default_agent.as_str()) {
        return Err(format!(
            "default_agent must be one of: {}",
            AGENT_SLUGS.join(", ")
        ));
    }
    if crate::providers::stt_route(&stt_provider).is_none() {
        return Err(format!(
            "stt_provider must be one of: {}",
            crate::providers::STT_ROUTES
                .iter()
                .map(|r| r.slug)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if crate::providers::router_route(&routing_provider).is_none() {
        return Err(format!(
            "routing_provider must be one of: {}",
            crate::providers::ROUTER_ROUTES
                .iter()
                .map(|r| r.slug)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    save_settings(&Settings {
        ptt_hotkey: ptt,
        exit_type_mode_hotkey: exit,
        voice_engine: engine,
        wallpaper,
        default_agent,
        stt_provider,
        routing_provider,
    })
}

// ---------------------------------------------------------------------------
// Persist-hooks (create/close/aktivitet/teardown)
// ---------------------------------------------------------------------------

/// create_card + synkron persist (main.rs' create_card-wrapper kalder denne).
/// En persist-fejl ruller IKKE kortet tilbage (det er brugbart i sessionen)
/// — den logges, og kortet mangler saa blot efter genstart.
pub fn create_card_persisted(
    cwd: String,
    profile: Option<String>,
    command: Option<String>,
) -> Result<registry::CardInfo, String> {
    // K2-choke-point: en fravaerende/tom/whitespace profil resolves HER (foer
    // registry-laget) til den LIVE-laeste `default_agent`-setting — samme
    // moenster som wallpaper, INGEN restart-semantik (spec §3).
    let profile = profile
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| load_settings().default_agent);
    #[cfg(feature = "perf-trace")]
    let lock_started = Instant::now();
    let _mutation = lock_create_mutations();
    #[cfg(feature = "perf-trace")]
    let lock_wait_ms = lock_started.elapsed().as_secs_f64() * 1_000.0;
    #[cfg(feature = "perf-trace")]
    let registry_started = Instant::now();
    let info = registry::create_card(cwd, profile, command.clone())?;
    #[cfg(feature = "perf-trace")]
    let registry_ms = registry_started.elapsed().as_secs_f64() * 1_000.0;
    #[cfg(feature = "perf-trace")]
    let persist_started = Instant::now();
    if let Err(e) = record_card_created(&info, command) {
        eprintln!(
            "[canvas] workspace persist after create {} failed: {e}",
            info.name
        );
    }
    crate::perf_mark!(
        "create.workspace_persist.end",
        serde_json::json!({
            "card": info.name,
            "card_mutations_wait_ms": lock_wait_ms,
            "registry_create_ms": registry_ms,
            "persist_ms": persist_started.elapsed().as_secs_f64() * 1_000.0,
        }),
    );
    // Kendelse CORRECTIONS.md C-T6: status-hooket bor HER og ikke i main.rs'
    // kommando-wrappers, saa enhver kalder af den persisterede vej er daekket —
    // ogsaa `close_card_persisted` nedenfor, der er `pub` og gaar uden om
    // main.rs. No-op indtil `status::install` er kaldt i app-opstarten (CLI'en
    // og testene har ingen writer). Ingen kort-laas holdes her (laaseorden).
    //
    // GUARDEN SLIPPES FOERST. `refresh_card_counts` -> `registry::list_cards()`
    // laaser HVERT kort blokerende, og `spawn_into` holder ét korts laas
    // uafbrudt hen over hele PTY-spawnet (10-100+ ms). Blev refreshen kaldt
    // under den eksklusive guard, ville to kort oprettet taet paa hinanden
    // (voice-kaeder, eller create #2 mens #1 spawner) saette create #3 og
    // enhver close i koe bag et blokeret `list_cards` — praecis den
    // CREATE_LOCK-koe perf-sporet maalte og fjernede (4 creates: 1.302 ->
    // 103 ms). Samme moenster som `spawn_into`, der kalder refreshen efter
    // `drop(card)`. Regressionsvaern: unit-testen nederst i filen.
    drop(_mutation);
    crate::workspaces::status::refresh_card_counts();
    Ok(info)
}

fn record_card_created(info: &registry::CardInfo, command: Option<String>) -> Result<(), String> {
    let path = workspace_path();
    let mut st = lock_state()?;
    if !matches!(st.load, LoadState::Loaded) {
        // NotLoaded/Failed: registryet virker, persistensen er slaaet fra.
        return Ok(());
    }
    let (x, y, w, h) = default_geometry(info.number);
    st.file.cards.retain(|c| c.name != info.name); // defensivt mod desync
    st.file.cards.push(WorkspaceCard {
        number: info.number,
        name: info.name.clone(),
        cwd: info.cwd.clone(),
        profile: info.profile.clone(),
        command,
        x,
        y,
        w,
        h,
        last_active_at: None,
    });
    st.file.next_card_number = st.file.next_card_number.max(info.number + 1);
    let snapshot = st.file.clone();
    save_workspace_file(&path, &snapshot)?;
    st.dirty = false;
    Ok(())
}

/// Batch-close + præcis EN synkron workspace-persist. Close holder en delt
/// mutations-guard gennem teardown+persistence, saa uafhaengige closes kan
/// overlappe, mens create fortsat er eksklusiv og ABA-sikker.
pub fn close_cards_persisted(names: Vec<String>) -> Result<registry::CloseCardsResult, String> {
    #[cfg(feature = "perf-trace")]
    let total_started = Instant::now();
    #[cfg(feature = "perf-trace")]
    let lock_started = Instant::now();
    let _mutation = lock_close_mutations();
    #[cfg(feature = "perf-trace")]
    let lock_wait_ms = lock_started.elapsed().as_secs_f64() * 1_000.0;
    crate::perf_mark!(
        "close.card_mutations_lock_acquired",
        serde_json::json!({ "wait_ms": lock_wait_ms, "count": names.len() }),
    );
    #[cfg(feature = "perf-trace")]
    let registry_started = Instant::now();
    let mut result = registry::close_cards(names)?;
    #[cfg(feature = "perf-trace")]
    let registry_ms = registry_started.elapsed().as_secs_f64() * 1_000.0;

    // Registry-resultatet indeholder baade faktisk lukkede og ukendte navne.
    // Fjern begge fra workspace-snapshottet: unknown-grenen healer en evt.
    // gammel registry/workspace-desync ligesom single-close altid har gjort.
    let mut persisted_names = result.closed.clone();
    persisted_names.extend(
        result
            .errors
            .iter()
            .filter(|error| error.message.starts_with("no such card:"))
            .map(|error| error.name.clone()),
    );

    #[cfg(feature = "perf-trace")]
    let persist_started = Instant::now();
    match record_cards_closed(&persisted_names) {
        Ok(workspace_reset) => {
            // Registry-resultatet kan være stale under parallelle closes: A
            // kan have set B før B detached, mens A først persisterer sidst.
            // Workspace-reset er serialiseret af STATE og genlæsningen sker
            // efter egen teardown; create er stadig writer-blokeret her.
            result.sequence_reset = committed_sequence_reset(workspace_reset)?;
        }
        Err(error) => {
            result.sequence_reset = false;
            eprintln!("[canvas] workspace persist after close batch failed: {error}");
            result.errors.push(registry::CloseCardsError {
                name: "workspace".to_string(),
                message: error,
            });
        }
    }
    crate::perf_mark!(
        "close.workspace_persist.end",
        serde_json::json!({
            "card_mutations_wait_ms": lock_wait_ms,
            "registry_close_ms": registry_ms,
            "persist_ms": persist_started.elapsed().as_secs_f64() * 1_000.0,
            "duration_ms": total_started.elapsed().as_secs_f64() * 1_000.0,
            "closed_count": result.closed.len(),
            "error_count": result.errors.len(),
        }),
    );
    // Kendelse CORRECTIONS.md C-T6 — se noten i `create_card_persisted`,
    // inklusive hvorfor guarden slippes FOER refreshen. Close-guarden er kun en
    // laeser, men en laeser der blokerer i `list_cards` holder stadig enhver
    // create ude, saa koeen er den samme.
    drop(_mutation);
    crate::workspaces::status::refresh_card_counts();
    Ok(result)
}

/// Bagudkompatibel single-close; delegerer til batch-corens parallel-safe
/// lifecycle og bevarer den gamle `Result<(), String>`-wire.
pub fn close_card_persisted(name: String) -> Result<(), String> {
    let result = close_cards_persisted(vec![name.clone()])?;
    if let Some(error) = result.errors.into_iter().find(|error| error.name == name) {
        return Err(error.message);
    }
    Ok(())
}

/// Fjerner hele batchen fra memory-state og skriver højst ét snapshot.
/// Returnerer true præcis når workspacet er tomt og next-sekvensen derfor er
/// nulstillet til 1. Filfejl bevarer dirty=true til næste flush.
fn record_cards_closed(names: &[String]) -> Result<bool, String> {
    let path = workspace_path();
    let mut st = STATE.lock().map_err(|e| e.to_string())?;
    if !matches!(st.load, LoadState::Loaded) {
        return Ok(false);
    }
    let names: std::collections::HashSet<&str> = names.iter().map(String::as_str).collect();
    let before = st.file.cards.len();
    st.file
        .cards
        .retain(|card| !names.contains(card.name.as_str()));
    let workspace_reset = st.file.cards.is_empty();
    let old_next = st.file.next_card_number;
    if workspace_reset {
        st.file.next_card_number = 1;
    }
    if st.file.cards.len() == before && st.file.next_card_number == old_next {
        return Ok(workspace_reset);
    }
    let snapshot = st.file.clone();
    match save_workspace_file(&path, &snapshot) {
        Ok(()) => {
            st.dirty = false;
            Ok(workspace_reset)
        }
        Err(e) => {
            st.dirty = true;
            Err(e)
        }
    }
}

/// last_active_at (bindende WorkspaceCard-kontrakt: saettes ved write_pty/
/// spawn). Hot path (hvert tastetryk) => debounced persist, aldrig synkron.
pub fn touch_card_activity(name: &str) {
    let Ok(mut st) = STATE.lock() else { return };
    if !matches!(st.load, LoadState::Loaded) {
        return;
    }
    let Some(card) = st.file.cards.iter_mut().find(|c| c.name == name) else {
        return;
    };
    card.last_active_at = Some(now_iso_z());
    mark_dirty(&mut st);
}

/// Synkron flush af en evt. udestaaende debounced aendring — kaldes fra
/// ExitRequested-teardownen i main.rs.
pub fn persist_now() -> Result<(), String> {
    let path = workspace_path();
    let mut st = lock_state()?;
    if !matches!(st.load, LoadState::Loaded) || !st.dirty {
        return Ok(());
    }
    let snapshot = st.file.clone();
    save_workspace_file(&path, &snapshot)?;
    st.dirty = false;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        committed_sequence_reset, lock_close_mutations, lock_create_mutations, normalize_settings,
        Settings,
    };
    use crate::registry;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    /// Registryet og `CARD_MUTATIONS` er PROCES-GLOBAL tilstand, og cargo koerer
    /// unit-tests parallelt i én binary. `committed_sequence_reset`-testen
    /// afhaenger af at registryet er tomt, og laase-testene af hvem der holder
    /// guarden — saa de tre skal koere én ad gangen. Poison ignoreres: en
    /// panicking test maa ikke laase de oevrige ude.
    fn global_kort_tilstand() -> std::sync::MutexGuard<'static, ()> {
        static M: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        M.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn close_mutation_guards_overlap_while_create_guard_waits() {
        let _serial = global_kort_tilstand();
        let first_close = lock_close_mutations();

        let (second_close_tx, second_close_rx) = mpsc::channel();
        let second_close = thread::spawn(move || {
            let _guard = lock_close_mutations();
            second_close_tx.send(()).unwrap();
        });
        second_close_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("a second close reader queued behind the first");
        second_close.join().unwrap();

        let (writer_attempt_tx, writer_attempt_rx) = mpsc::channel();
        let (writer_acquired_tx, writer_acquired_rx) = mpsc::channel();
        let writer = thread::spawn(move || {
            writer_attempt_tx.send(()).unwrap();
            let _guard = lock_create_mutations();
            writer_acquired_tx.send(()).unwrap();
        });
        writer_attempt_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert!(
            matches!(
                writer_acquired_rx.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ),
            "create writer acquired while a close transaction was active"
        );

        drop(first_close);
        writer_acquired_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("create writer did not resume after closes released");
        writer.join().unwrap();
    }

    /// `refresh_card_counts()` -> `registry::list_cards()` laaser HVERT kort
    /// blokerende, og `spawn_into` holder ÉT korts laas uafbrudt hen over hele
    /// PTY-spawnet (10-100+ ms). Kaldes refreshen mens den eksklusive
    /// mutations-guard holdes, blokerer create #2 i `list_cards` MED guarden i
    /// haanden — og create #3 og enhver close koeer bag den. Det er praecis
    /// CREATE_LOCK-koeen P1-P7-perf-arbejdet maalte og fjernede (4 creates:
    /// 1.302 -> 103 ms).
    ///
    /// Testen bygger koeen med vilje: vi holder selv guarden mens skaberen
    /// startes, saa den med sikkerhed er naeste indehaver, og vi holder ét
    /// korts laas hele vejen, saa dens `list_cards` ikke kan komme igennem.
    /// Regressionen er dermed ikke timing-afhaengig: med fejlen slippes guarden
    /// ALDRIG mens kort-laasen holdes.
    #[test]
    fn create_slipper_mutations_guarden_foer_korttallene_genberegnes() {
        use crate::workspaces::status::StatusWriter;
        use std::sync::Arc;

        let _serial = global_kort_tilstand();

        // `refresh_card_counts` er en no-op uden en installeret writer, saa
        // regressionen kan kun observeres med en (sandkasset) writer paa plads.
        // TempDir'en laekkes bevidst: writeren er proces-global via OnceLock og
        // overlever denne test, og en slettet mappe ville give stoej i loggen.
        let base: &'static tempfile::TempDir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        crate::workspaces::status::install(Arc::new(StatusWriter::new(
            base.path().to_path_buf(),
            "w-mutations".to_string(),
            "inst-mutations".to_string(),
        )));
        assert!(
            crate::workspaces::status::installed().is_some(),
            "uden en installeret writer ville refreshen ikke laase nogen kort"
        );

        let cwd = tempfile::tempdir().unwrap();
        let blokerende =
            registry::create_card(cwd.path().display().to_string(), "claude".to_string(), None)
                .unwrap();
        let handle = registry::card_handle(&blokerende.name).unwrap();
        // Staar for `spawn_into`, der holder kortets laas hen over PTY-spawnet.
        let spawn_laas = handle.lock().unwrap_or_else(|p| p.into_inner());

        // 1) Vi holder guarden selv, saa skaberen koeer op bag os og med
        //    sikkerhed er den naeste der faar den.
        let vores = lock_create_mutations();
        let cwd_for_skaber = cwd.path().display().to_string();
        let skaber = thread::spawn(move || {
            super::create_card_persisted(cwd_for_skaber, Some("claude".to_string()), None)
        });
        thread::sleep(Duration::from_millis(150));
        drop(vores);
        // 2) Skaberen er nu indehaver — og blokerer i `list_cards` paa det kort
        //    vi holder.
        thread::sleep(Duration::from_millis(150));

        // 3) Naeste create/close skal kunne komme til imens.
        let (fik_tx, fik_rx) = mpsc::channel();
        let anden_create = thread::spawn(move || {
            let _guard = lock_create_mutations();
            fik_tx.send(()).unwrap();
        });
        let kom_til = fik_rx.recv_timeout(Duration::from_secs(3));

        // Ryd op FOER assert'en, saa en roed test ikke efterlader haengende traade.
        drop(spawn_laas);
        let skabt = skaber.join().unwrap();
        anden_create.join().unwrap();
        if let Ok(info) = &skabt {
            let _ = registry::close_cards(vec![info.name.clone()]);
        }
        let _ = registry::close_cards(vec![blokerende.name]);

        kom_til.expect(
            "create holdt CARD_MUTATIONS mens refresh_card_counts ventede paa et korts laas \
             — det er CREATE_LOCK-koeen igen",
        );
    }

    #[test]
    fn committed_sequence_reset_rechecks_registry_after_a_stale_close_observation() {
        let _serial = global_kort_tilstand();
        let cwd = tempfile::tempdir().unwrap();
        let first =
            registry::create_card(cwd.path().display().to_string(), "claude".to_string(), None)
                .unwrap();
        let second =
            registry::create_card(cwd.path().display().to_string(), "claude".to_string(), None)
                .unwrap();

        let stale = registry::close_cards(vec![first.name]).unwrap();
        assert!(
            !stale.sequence_reset,
            "the first close must observe the still-live second card"
        );
        let last = registry::close_cards(vec![second.name]).unwrap();
        assert!(last.sequence_reset);
        assert!(
            committed_sequence_reset(true).unwrap(),
            "workspace-empty commit must re-read current registry state, not AND the stale false"
        );
    }

    #[test]
    fn settings_default_agent_defaults_and_validates() {
        let s = Settings::default();
        assert_eq!(s.default_agent, "claude");
        // gammel settings.json uden feltet -> default (serde(default))
        let old: Settings =
            serde_json::from_str(r#"{"ptt_hotkey":"x","exit_type_mode_hotkey":"y"}"#).unwrap();
        assert_eq!(old.default_agent, "claude");
    }

    #[test]
    fn load_settings_normalizes_unknown_default_agent_tolerantly() {
        // Spec §3 (wallpaper-moenstret): TOLERANT laese-side — ukendt/tom/whitespace
        // slug maa ALDRIG faa senere creates til at fejle (GPT-review B6).
        //
        // Femte case (slut-review fix 2): en PADDED men gyldig slug er den ene
        // der slap igennem — den trimmer til noget i allowlisten, saa
        // fallback-grenen springes over, og feltet beholdt sin whitespace hele
        // vejen ned i `profiles::profile(" codex ")` => None => "unknown
        // profile". Den skal normaliseres til den TRIMMEDE gyldige slug, ikke
        // til fallbacken.
        for (raw, expected) in [
            ("cursor", "claude"),
            ("", "claude"),
            ("  ", "claude"),
            ("CLAUDE", "claude"),
            (" codex ", "codex"),
        ] {
            let s: Settings =
                serde_json::from_str(&format!(r#"{{"default_agent":"{raw}"}}"#)).unwrap();
            assert_eq!(
                normalize_settings(s).default_agent,
                expected,
                "slug: {raw:?}"
            );
        }
        // gyldig slug bevares (case-sensitivt lowercase-krav)
        let ok: Settings = serde_json::from_str(r#"{"default_agent":"codex"}"#).unwrap();
        assert_eq!(normalize_settings(ok).default_agent, "codex");
    }

    // `create_card_persisted(profile: None)` laeser `load_settings().default_agent`
    // — den afhaenger dermed af `project::global_base()` (TALMINAL_GLOBAL_HOME
    // hhv. den rigtige %LOCALAPPDATA%\Talminal). Et in-process unit-test-kald
    // her ville stille laekke udviklerens rigtige settings.json ind i assertet
    // (review-fund 1). Den hermetiske, TALMINAL_GLOBAL_HOME-isolerede udgave
    // bor derfor i `tests/workspace.rs`'s worker-proces-harness, se
    // `create_card_without_profile_uses_default_agent_setting` dér.
}
