//! Browser-kort: scope-/port-/profil-tilstand + synligheds-komposition
//! (spec §5 + §8a). REN logik — ingen tauri-typer; webview-ops bor i
//! browser_host.rs (main-lagets ansvar). Global singleton efter registry-
//! moensteret (tests linker lib'en).
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, OnceLock};

use crate::cards::talminal_base;

pub const PLAYWRIGHT_MCP_PIN: &str = "@playwright/mcp@0.0.78";

/// wry's default-args for WebView2 pr. tauri 2.11 — SKAL gentages, fordi
/// additional_browser_args ERSTATTER dem (spec §4). Re-check ved tauri-bump.
const WRY_DEFAULT_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

#[derive(Debug, Clone, PartialEq)]
pub struct ScopeInfo {
    pub key: String,
    pub owner: Option<String>,
    pub port: u16,
    pub profile_dir: PathBuf,
    /// Keeperens CDP-target-id (sat af browser_host efter keeper-ready).
    /// Pollerens drabs-immunitet er ID-baseret — URL-fritagelsen alene er
    /// utilstraekkelig, fordi en agent kan navigere keeperen vaek fra
    /// about:blank (2026-07-20-dogfood-bug: kapret keeper blev draebt og
    /// tog hele scopets browserproces med sig). NB: ScopeInfo-kloner er
    /// snapshots — `ensure_scope`/`ensure_scope_ready`-returvaerdier baerer
    /// IKKE noedvendigvis et frisk id; [`keeper_target`] er autoritativ.
    pub keeper_target_id: Option<String>,
}

static SCOPES: LazyLock<Mutex<HashMap<String, ScopeInfo>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static LAUNCH_ID: OnceLock<String> = OnceLock::new();

pub fn launch_id() -> &'static str {
    LAUNCH_ID.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

pub fn scope_key(opened_by: Option<&str>) -> String {
    match opened_by {
        Some(name) => format!("agent-{name}"),
        None => "canvas".to_string(),
    }
}

fn profiles_root() -> PathBuf {
    talminal_base().join("browser-profiles")
}

fn free_port() -> Result<u16, String> {
    // Pick-then-release: keeper-webviewen (Task 5) binder porten umiddelbart
    // efter — vinduet er millisekunder (spec §5, S0-verificeret).
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("port alloc: {e}"))
}

pub fn ensure_scope(opened_by: Option<&str>) -> Result<ScopeInfo, String> {
    let key = scope_key(opened_by);
    let mut scopes = SCOPES.lock().map_err(|e| e.to_string())?;
    if let Some(existing) = scopes.get(&key) {
        return Ok(existing.clone());
    }
    let info = ScopeInfo {
        key: key.clone(),
        owner: opened_by.map(str::to_string),
        port: free_port()?,
        profile_dir: profiles_root().join(launch_id()).join(&key),
        keeper_target_id: None,
    };
    scopes.insert(key, info.clone());
    Ok(info)
}

/// Registrerer/rydder keeperens target-id paa et eksisterende scope. No-op
/// hvis scopet er fjernet imens (teardown vandt racen — id'et doer med scopet).
pub fn set_keeper_target(key: &str, target_id: Option<String>) {
    if let Ok(mut scopes) = SCOPES.lock() {
        if let Some(scope) = scopes.get_mut(key) {
            scope.keeper_target_id = target_id;
        }
    }
}

/// Aktuelt keeper-target for et scope — SCOPES-mappet er autoritativt.
/// ScopeInfo-KLONER (fx ensure_scope's returvaerdi) kan baere et foraeldet
/// `keeper_target_id`; laes altid her naar det skal vaere friskt.
pub fn keeper_target(key: &str) -> Option<String> {
    SCOPES
        .lock()
        .ok()?
        .get(key)
        .and_then(|scope| scope.keeper_target_id.clone())
}

/// Keeperens sentinel-URL: strukturel identitet i CDP's `/json` (fragmentet)
/// OG bevaret `about:blank`-praefiks, saa pollerens blank-fritagelse daekker
/// den selv i vinduet FOER target-id'et er registreret. `validate_card_url`
/// kan aldrig producere denne URL for et kort (kun http/https/about:blank).
pub fn keeper_sentinel(scope_key: &str) -> String {
    format!("about:blank#keeper-{scope_key}")
}

pub fn scope_for(opened_by: Option<&str>) -> Option<ScopeInfo> {
    SCOPES.lock().ok()?.get(&scope_key(opened_by)).cloned()
}

pub fn remove_scope(key: &str) {
    if let Ok(mut scopes) = SCOPES.lock() {
        scopes.remove(key);
    }
}

pub fn all_scopes() -> Vec<ScopeInfo> {
    SCOPES
        .lock()
        .map(|s| s.values().cloned().collect())
        .unwrap_or_default()
}

/// App-start-sweep (spec §5): profiler er run-flygtige; alt fra tidligere
/// launches slettes. Best-effort — laaste filer efterlades til naeste sweep.
pub fn sweep_profiles() {
    let _ = std::fs::remove_dir_all(profiles_root());
}

pub fn additional_browser_args(port: u16) -> String {
    format!(
        "{WRY_DEFAULT_ARGS} --remote-debugging-port={port} --autoplay-policy=user-gesture-required"
    )
}

/// Spec §8a-kompositionen: occlusion-gate vinder over fuldskaerm; under
/// fuldskaerm vises KUN fuldskaerms-kortet. Doede kort vises ALDRIG — deres
/// zombie-webview maa ikke daekke dead-chromen ("luk kortet") i DOM'en.
pub fn webview_should_show(
    occluded: bool,
    fullscreen: Option<&str>,
    name: &str,
    alive: bool,
) -> bool {
    if !alive || occluded {
        return false;
    }
    match fullscreen {
        Some(fs) => fs == name,
        None => true,
    }
}
