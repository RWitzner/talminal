// Talminal — Tauri-kerne: kort-state, pty-wiring, epoch-gate, signalfiler.
// Task 1 (canvas-pivot): supervision (epoch-gate/signalfiler/presence) er cfg-gated
// bag `supervision`-featuren; default-staten stubber den FROSNE IPC-kontrakt via
// control-facaden (gate pass-through, owner "persona", epoch 0, ingen signalfiler).
// Pty-lifecyclen ejes af denne proces (spec §5): en webview-reload dræber ikke
// pty'erne — kun app-processens død gør.
// Modulerne (pty/cards/epoch/signals) bor i lib-craten talminal_canvas_lib
// (pub mod i lib.rs), så src-tauri/tests/-integrationstests kan linke dem.
//
// LÅS-HIERARKI (fix F2 — normativt for alle senere ændringer):
//   1) Registry-map-låsen (registry.rs, Task 5 — før: AppState.cards): holdes
//      KUN for opslag/insert/remove — ALDRIG hen over PtyHost::spawn,
//      host.write eller kill_and_teardown.
//   2) Pr.-kort Arc<Mutex<CardRuntime>>: gate-beslutninger + pty-Arc-klon;
//      må holdes over spawn (per-kort single-flight), aldrig over write.
//   3) PtyHost's interne Mutex'er (writer/child/master): child-Mutex'en er
//      ADSKILT fra writer-Mutex'en, så kill aldrig venter på en hængende write.
// Alle pty-rørende kommandoer er async fn og kører det blokerende arbejde via
// tauri::async_runtime::spawn_blocking — aldrig på UI-/main-tråden.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
#[cfg(feature = "perf-trace")]
use std::time::Instant;

use base64::Engine as _;
use tauri::Manager;
use tauri::{AppHandle, Emitter, State};

use talminal_canvas_lib::cards::{
    default_cards_path, guard_resume_control, guard_write_pty, talminal_base,
};
#[cfg(feature = "supervision")]
use talminal_canvas_lib::cards::{load_cards, CardsError};
// Task 1: al pause-/epoch-semantik gaar gennem control-facaden — supervision-ON
// re-eksporterer den rigtige EpochGate, default-state en ZST-stub med frossen
// IPC-kontrakt (gate pass-through, owner "persona", epoch 0, ingen signalfiler).
use talminal_canvas_lib::control::{self, WriteOutcome};
#[cfg(feature = "supervision")]
use talminal_canvas_lib::presence;
use talminal_canvas_lib::prompt_readiness::PromptReadiness;
use talminal_canvas_lib::pty::{PtyHost, PtySpawn};
// Task 5: registry.rs EJER kort-runtime-mappen (CardRuntime bor der);
// oevrige Del B-sockets er doede indtil deres task udfylder dem.
#[cfg(feature = "perf-trace")]
use talminal_canvas_lib::perf_trace;
use talminal_canvas_lib::{
    browser, browser_host, context_hud, instance, mcp, project, registry, secrets, submit, threads,
    transcripts, usage_hud, voice_capture, wake_hotkey, webview_permissions, worker_mcp, workspace,
    workspaces,
};

pub type CardId = String;

pub struct AppState {
    /// Supervision-only: pause-/resume-signalfilernes dir (spec §5).
    #[cfg(feature = "supervision")]
    pub signals_dir: PathBuf,
    pub cards_path: PathBuf,
    /// true ⇔ cards.toml fandtes ikke (Task 7-kontrakten: NotFound-grenen —
    /// FORVENTET tilstand; frontenden viser stien).
    pub cards_missing: bool,
    /// Some(tekst) ⇔ cards.toml fandtes men var defekt (Parse/Invalid/Io) —
    /// en REEL fejl; frontenden viser teksten, ikke "opret filen her".
    pub cards_error: Option<String>,
}

// Kort-tællingen i `status.json` genberegnes af
// `workspaces::status::refresh_card_counts()` i LIB-craten — ikke af en helper
// her. De fleste veje der ændrer "har kortet en levende session" har nemlig
// ingen `AppHandle`: `workspace.rs`' persist-hooks, exit-watcheren og
// kill-vejen. Se kendelse CORRECTIONS.md C-T6 og LÅSEORDEN i
// `workspaces/status.rs` (må aldrig kaldes med en `CardRuntime`-lås i hånden).

/// Fix F2: alt pty-rørende kommandoarbejde kører på async-runtimens
/// blocking-pool — aldrig på UI-/main-tråden.
async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
}

// Task 3 (bindende kontrakt): is_master-feltet er væk i BEGGE feature-states —
// supervision-master genopstår først på wiren ved un-parkering.
#[derive(Clone, serde::Serialize)]
pub struct CardView {
    pub name: String,
    pub cwd: String,
}

#[derive(Clone, serde::Serialize)]
struct PtyOutputEvent {
    name: String,
    data_b64: String,
}

#[derive(Clone, serde::Serialize)]
struct CardExitEvent {
    name: String,
}

#[derive(Clone, serde::Serialize)]
struct PauseStateEvent {
    name: String,
    owner: String,
    epoch: u64,
}

#[tauri::command]
fn get_cards() -> Vec<CardView> {
    // Task 5 (grid-kompat): læser fra registryet. Map-låsen slippes FØR
    // pr.-kort-låsene tages (lås-hierarkiet, fix F2 — inde i all_handles).
    let handles = registry::all_handles();
    let mut views: Vec<CardView> = handles
        .iter()
        .filter_map(|h| {
            h.lock().ok().map(|c| CardView {
                name: c.name().to_string(),
                // Browser-kort medtages med tom cwd (plan Task 2: get_cards
                // udelader dem IKKE).
                cwd: c
                    .terminal()
                    .map(|t| t.config.cwd.display().to_string())
                    .unwrap_or_default(),
            })
        })
        .collect();
    // Deterministisk rækkefølge: alfabetisk (Task 3: master-særbehandlingen
    // er ude af MVP-pathen).
    views.sort_by(|a, b| a.name.cmp(&b.name));
    views
}

#[derive(Clone, serde::Serialize)]
pub struct CardsStatus {
    pub path: String,
    pub missing: bool,
    pub error: Option<String>,
}

#[derive(Clone, serde::Serialize)]
pub struct CardState {
    pub running: bool,
    pub exited: Option<u32>,
    pub owner: String,
    pub epoch: u64,
}

/// Fix F10 (reload-overlevelse, spec §5): pty'erne overlever webview-reload —
/// lokal JS-state gør ikke. Frontenden hydrerer ownerRef/epochRef/paused/
/// exited herfra ved mount og spawner KUN når running=false && exited=None.
#[tauri::command]
fn get_card_state(name: String) -> Result<CardState, String> {
    let handle = registry::card_handle(&name)?;
    let card = handle.lock().map_err(|e| e.to_string())?;
    match &card.backend {
        registry::CardBackend::Terminal(term) => Ok(CardState {
            running: term.pty.is_some(),
            exited: card.exited,
            // Frossen IPC-kontrakt: default-state svarer altid "persona"/0.
            owner: control::card_owner(&term.gate),
            epoch: control::card_epoch(&term.gate),
        }),
        // Browser-gren (plan Task 2, frossen kontrakt): running = alive;
        // owner/epoch er supervisions-begreber og svarer altid "persona"/0.
        registry::CardBackend::Browser(browser) => Ok(CardState {
            running: browser.alive,
            exited: card.exited,
            owner: "persona".into(),
            epoch: 0,
        }),
        registry::CardBackend::Chat(_) => Ok(CardState {
            running: true,
            exited: card.exited,
            owner: "persona".into(),
            epoch: 0,
        }),
    }
}

/// Manglende/defekt cards.toml-grenene (Task 7-kontrakten): frontenden kalder
/// denne når get_cards er tom — missing ⇒ vis stien ("opret filen her"),
/// error ⇒ vis den reelle fejltekst. Journalført kommando-tilføjelse.
#[tauri::command]
fn get_cards_status(state: State<'_, AppState>) -> CardsStatus {
    CardsStatus {
        path: state.cards_path.display().to_string(),
        missing: state.cards_missing,
        error: state.cards_error.clone(),
    }
}

/// Degraderet worker-spawn (bug 6-fladen): Talminal-MCP/--no-chrome naaede ikke
/// med, saa agenten kan falde tilbage til Claude in Chrome. Meldingen skal
/// vaere synlig for operatoeren — frontenden viser eventet i HUD'ens
/// fejlkanal (App.tsx-listeneren) ud over dev-stderr.
fn emit_worker_degraded(app: &AppHandle, card: &str, reason: &str) {
    // B3 (GPT-review): en degraderet spawn (fejlet MCP-injektion) maa aldrig
    // efterlade et token gyldigt for kortet — det ville laekke MCP-adgang
    // til en worker der reelt koerer uden tools. Idempotent naar intet
    // token er sat (fx den uaendrede ClaudeFlags-vej).
    mcp::clear_card_token(card);
    eprintln!("[canvas] worker {card}: browser-tools degraderet: {reason}");
    let _ = app.emit(
        "worker-browser-tools-degraded",
        serde_json::json!({ "card": card, "reason": reason }),
    );
    // NB: her ryddes KUN tokenet — configfilen roeres bevidst ikke.
    // `emit_worker_degraded` er ikke en afslutning (workeren koerer videre,
    // bare uden tools), og den ligger i H1-racens mellem-vindue, hvor en taber
    // ikke maa have sideeffekter paa vinderens fil. Se `spawn_into`s doc og
    // `release_card_identity` nedenfor.
}

/// Frigiver et korts identitet paa en AFSLUTNINGSVEJ: MCP-tokenet ryddes fra
/// registryet, OG worker-mcp-configfilen fjernes fra disken.
///
/// De to hoerer sammen, fordi filen BAERER tokenet
/// (`"Authorization": "Bearer …"`). Et ryddet token uden en fjernet fil
/// efterlader en credential-formet fil paa disken som enhver proces under
/// samme bruger kan laese — inklusive de andre agent-kort, der pr. definition
/// har shell. Maalt foer gate 8: en `card-1.json` overlevede baade kort-luk og
/// app-exit i 16 timer.
///
/// At de to handlinger bor ét sted er selve pointen: der er seks
/// afslutningsveje i denne fil, og en syvende kan ikke komme til at huske den
/// ene halvdel og glemme den anden.
///
/// **Bruges IKKE af `emit_worker_degraded`** — den er ikke en afslutning, og
/// den ligger i H1-racens mellem-vindue.
fn release_card_identity(card: &str) {
    mcp::clear_card_token(card);
    worker_mcp::remove_config(card);
}

/// Payload for "card-spawn-failed" (N4, spec brief T9) — udtrukket som ren
/// funktion saa den er unit-testbar uden en AppHandle. Rust-fejlteksten (fx
/// fra `profiles::resolve_spawn_program`) er allerede brugerteksten og gaar
/// igennem uaendret.
fn spawn_failed_payload(name: &str, number: u32, error: &str) -> serde_json::Value {
    serde_json::json!({ "name": name, "number": number, "error": error })
}

/// Cold workers får kun allokeret deres uforanderlige scope/endpoint; den
/// første browser_card_open opretter keeperen. Et respawn med en eksisterende
/// keeper beholder derimod den hidtidige fast-path-sundhedsprobe/heal, så en
/// allerede indlæst Playwright-session ikke arver et dødt endpoint.
fn worker_scope_for_spawn(app: &AppHandle, name: &str) -> Result<browser::ScopeInfo, String> {
    let scope = browser::ensure_scope(Some(name))?;
    let keeper_label = format!("keeper-{}", scope.key);
    if browser::keeper_target(&scope.key).is_some() || app.get_webview(&keeper_label).is_some() {
        browser_host::ensure_scope_ready(app, Some(name))
    } else {
        Ok(scope)
    }
}

/// K1-VAGT (GPT-review-fund B1): oploeser kommandoens program-slot til
/// profilens FAKTISKE executable (PATH- eller vendor-fallback), men KUN naar
/// programmet allerede ER profilens eget (stem-match mod `spawn_command[0]`).
/// Supervision-masterens feed-kommando (`custom_command=false`, program
/// "uv") er derfor aldrig i farezonen: "uv" matcher ikke "claude"-stemmet,
/// saa den forbliver uroert uanset `custom_command`-flaget. Ren funktion —
/// ingen laase/PTY her — saa transformationen kan unit-testes isoleret
/// (se `mod tests` nedenfor).
fn resolve_command_program(
    profile_id: &str,
    custom_command: bool,
    command: &mut [String],
) -> Result<(), String> {
    if custom_command {
        return Ok(());
    }
    if let (Some(prof), Some(program)) = (
        talminal_canvas_lib::profiles::profile(profile_id),
        command.first_mut(),
    ) {
        let expected = std::path::Path::new(prof.spawn_command[0]).file_stem();
        let actual = std::path::Path::new(program.as_str()).file_stem();
        if expected.is_some()
            && expected.map(|s| s.to_ascii_lowercase()) == actual.map(|s| s.to_ascii_lowercase())
        {
            *program = talminal_canvas_lib::profiles::resolve_spawn_program(prof)?;
        }
    }
    Ok(())
}

/// Blokerende spawn-hjælper — kaldes altid via run_blocking (fix F2).
/// PR.-KORT-låsen (aldrig den globale map-lås) holdes i TO korte scopes, ALDRIG
/// hen over MCP-injektionen imellem: (1) validér single-flight + læs
/// injektions-beslutningen; (2) — efter injektionen — genlås, RE-validér de
/// samme invarianter og kør PtyHost::spawn. Kritisk review-fund: injektionens
/// `ensure_scope_ready` laver en `on_main`-rundtur (poster et callback til
/// main-tråden og BLOKERER paa `rx.recv()`), mens de synkrone kommandoer
/// `get_card_state`/`list_cards`/`set_card_visible` koerer PAA main og laaser
/// SAMME kort-mutex — at holde laasen hen over rundturen ville give cirkulaer
/// ventning og fryse hele appen. `worker_mcp::write_config` er derimod ren
/// fil-IO uden main-traads-rundtur, og den skrives BEVIDST inde i fase 2's laas
/// — se skrivestedet nedenfor (fund H1). Samme fund flyttede
/// `emit_worker_degraded` derind: mellem-vinduet maa ikke have NOGEN sideeffekt
/// paa kortets token, hverken skrivende eller ryddende.
fn spawn_into(app: &AppHandle, name: &str, use_resume: bool) -> Result<(), String> {
    #[cfg(feature = "perf-trace")]
    let spawn_started = Instant::now();
    talminal_canvas_lib::perf_mark!(
        "create.spawn_into.begin",
        serde_json::json!({ "card": name, "resume": use_resume }),
    );
    let handle = registry::card_handle(name)?;
    // Fase 1: validér single-flight-invarianterne og læs KUN injektions-
    // beslutningen; slip så låsen før main-tråd-rundturen nedenfor. Browser-
    // kort-tools (spec §6): KUN profil-spawnede CC-workers faar MCP-injektion
    // — custom commands (whitespace-split uden quoting) roeres aldrig.
    let (inject, inject_profile) = {
        let card = handle.lock().map_err(|e| e.to_string())?;
        if !card.active {
            return Err(format!("card is closing: {name}"));
        }
        // Plan Task 2-fejlkontrakt: PTY-veje paa browser-kort afvises mekanisk.
        let Some(term) = card.terminal() else {
            return Err(format!("card is a browser: {name}"));
        };
        if term.pty.is_some() {
            return Err(format!("card already running: {name}"));
        }
        // K1 (spec rev 1.1): begge profiler faar MCP naar de findes — gaten
        // adskiller nu kun custom kommandoer/ukendte profil-id'er. Injektions-
        // STRATEGIEN (ClaudeFlags vs. CodexOverrides, Task 4) laeses af
        // profilen nedenfor — `'static` reference overlever laasens scope.
        let profile = talminal_canvas_lib::profiles::profile(&term.profile);
        (!term.custom_command && profile.is_some(), profile)
    };
    // MELLEM lås-scopes (ingen kort-lås hen over main-tråds-rundturen):
    // best-effort — allokér det stabile scope/endpoint og TRÆF injektions-
    // beslutningen, men opret IKKE keeper-webviewen her. Den første
    // browser_card_open kalder fortsat ensure_scope_ready under CREATE_LOCK og
    // bærer dermed lazy keeper-starten. Selve config-FILEN skrives først i fase
    // 2, under kortlåsen (fund H1 — se skrivestedet).
    // Fejler scope/config, spawner workeren UDEN browser-tools OG uden
    // --no-chrome, dvs. den kan falde tilbage til Claude in Chrome.
    // Fallbacken er bevidst (voice/terminal-flowet maa aldrig blokeres af
    // dette), men den maa ikke vaere lydloes: eprintln naar ingen i en pakket
    // app, saa degraderingen emittes ogsaa til HUD'ens fejlkanal — fra fase 2,
    // se `pending_degraded`.
    let mut mcp_args: Vec<String> = Vec::new();
    let mut mcp_extra_env: Vec<(String, String)> = Vec::new();
    // Slut-review fix 1: tokenet GENERERES her (det skal med i extra_env), men
    // REGISTRERES foerst i fase 2 — se registreringspunktet nedenfor.
    let mut card_token: Option<String> = None;
    // Fund H1: ClaudeFlags-vejens `--mcp-config`-fil SKRIVES ogsaa foerst i
    // fase 2. Her baeres kun beslutningen med: (mcp_port, cdp_port, token).
    let mut pending_config: Option<(u16, u16, String)> = None;
    // H1, anden doer (runde 2): `emit_worker_degraded`s FOERSTE handling er
    // `mcp::clear_card_token`. Kaldt fra dette ulaaste mellem-vindue kunne
    // TABEREN af to samtidige spawns paa SAMME kort dermed rydde VINDERENS
    // netop registrerede token — den koerende workers MCP-kald doer, praecis
    // den fejlklasse H1 handler om. Aarsagen baeres derfor med som data og
    // anvendes foerst inde i fase 2's laas, efter revalideringen der faelder
    // taberen.
    let mut pending_degraded: Option<String> = None;
    if inject {
        #[cfg(feature = "perf-trace")]
        let scope_started = Instant::now();
        match mcp::mcp_port() {
            Some(mcp_port) => match worker_scope_for_spawn(app, name) {
                Ok(scope) => {
                    #[cfg(feature = "perf-trace")]
                    let keeper_ready = browser::keeper_target(&scope.key).is_some();
                    talminal_canvas_lib::perf_mark!(
                        "create.scope_allocated.end",
                        serde_json::json!({
                            "card": name,
                            "scope": scope.key,
                            "port": scope.port,
                            "keeper_ready": keeper_ready,
                            "duration_ms": scope_started.elapsed().as_secs_f64() * 1_000.0,
                        }),
                    );
                    #[cfg(feature = "perf-trace")]
                    let config_started = Instant::now();
                    // Injektions-STRATEGI (Task 4, spec §1.2): CodexOverrides
                    // bruger per-invocation `-c`-overrides + kort-nøglet
                    // Bearer-token (ingen config-fil, mcp.rs slaar tokenet
                    // op). ClaudeFlags-vejen (default/ukendt strategi) bruger
                    // fortsat write_config + launch_args — men skrivningen er
                    // flyttet ned i fase 2's laas (H1).
                    let is_codex = matches!(
                        inject_profile.map(|p| p.mcp),
                        Some(talminal_canvas_lib::profiles::McpInjection::CodexOverrides)
                    );
                    // Beslutning 12: ét token pr. profil-spawnet kort, uanset
                    // profil. Leveringen er profil-specifik (env-var for codex,
                    // header i configen for CC), identiteten er det ikke.
                    let token = uuid::Uuid::new_v4().to_string();
                    if is_codex {
                        mcp_args.extend(worker_mcp::codex_launch_args(mcp_port, scope.port));
                        mcp_extra_env.push((worker_mcp::MCP_TOKEN_ENV.to_string(), token.clone()));
                        card_token = Some(token);
                        talminal_canvas_lib::perf_mark!(
                            "create.mcp_config.end",
                            serde_json::json!({
                                "card": name,
                                "duration_ms": config_started.elapsed().as_secs_f64() * 1_000.0,
                                "ok": true,
                            }),
                        );
                    } else {
                        // H1: skrivningen UDSKYDES til fase 2. Stien er
                        // deterministisk pr. kort ({worker-mcp}\{kort}.json),
                        // saa to samtidige spawns paa samme kort skriver til
                        // SAMME fil — og her, uden for begge laase-scopes,
                        // kunne taberens skrivning lande sidst. Registryet holdt
                        // saa vinderens token, mens filen (og dermed den
                        // spawnede CC-proces) bar taberens.
                        pending_config = Some((mcp_port, scope.port, token));
                    }
                }
                // Begge degraderings-aarsager UDSAETTES (H1): de rydder kortets
                // token, og det maa kun ske naar vi ved at kortet er vores.
                Err(e) => pending_degraded = Some(format!("browser-scope fejlede: {e}")),
            },
            None => pending_degraded = Some("mcp-serveren koerer ikke".to_string()),
        }
    }
    // LAASEORDEN (workspaces/status.rs' modul-doc): StatusWriter maa ALDRIG
    // laases mens en CardRuntime-laas holdes. Badge-tick'et gaar den anden vej
    // (StatusWriter -> CardRuntime via attention_kind_now), og de to ordener sammen
    // er en deadlock der ogsaa fryser pollertraaden og dermed hele
    // synligheds-protokollen. Synligheden laeses derfor HER — foer fase 2's
    // kortlaas tages — og bruges laengere nede naar maskinen skal foedes skjult.
    //
    // Laesningen er ET FOEDSELSGAET, ikke sandheden ved spawnets slutning: der
    // gaar hundreder af ms, og pollertraaden kan naa en hel Reveal-gren imens.
    // Den TOCTOU lukkes af `sync_initial_visibility` EFTER `drop(card)` — se
    // dér. Laesningen kan IKKE bare flyttes ned under kortlaasen; det ville
    // genindfoere netop den laaseorden-inversion noten ovenfor forbyder.
    let workspace_skjult = app
        .try_state::<Arc<workspaces::status::StatusWriter>>()
        .is_some_and(|writer| !writer.snapshot().visible);
    // Fase 2: genlås SAMME handle (instans-identitet: en samtidig close har sat
    // active=false paa netop denne runtime) og RE-validér de invarianter fase 1
    // holdt under én lås — en samtidig spawn/close kan have ramt kortet mens
    // låsen var sluppet til injektionen. Samme fejlflader som fase 1; ingen nye
    // tilstande opfindes.
    let mut card = handle.lock().map_err(|e| e.to_string())?;
    if !card.active {
        return Err(format!("card is closing: {name}"));
    }
    let Some(term) = card.terminal_mut() else {
        return Err(format!("card is a browser: {name}"));
    };
    if term.pty.is_some() {
        return Err(format!("card already running: {name}"));
    }
    // H1: mellem-vinduets degraderings-aarsag anvendes FOERST her — efter de tre
    // revalideringer ovenfor, som faelder taberen af to samtidige spawns.
    // Taberen returnerer paa `card already running` og naar aldrig at rydde
    // vinderens token. Rammer en af de tidlige returns, emittes ingenting: da
    // fejler hele spawnet, og der kommer ingen worker der kan koere degraderet.
    // Kaldet er lovligt under kortlaasen af samme grund som config-skrivningen
    // nedenfor (ingen main-traads-rundtur), og laaseordenen kort->token er den
    // samme som paa PtyHost-fejlvejen laengere nede.
    if let Some(reason) = pending_degraded {
        emit_worker_degraded(app, name, &reason);
    }
    // kill/close slukker den gamle reader-emit før teardown. Et legitimt
    // respawn paa samme aktive runtime aabner gaten igen.
    term.visible
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let mut command = if use_resume {
        term.config.resume_command.clone()
    } else {
        term.config.command.clone()
    };
    // K1-VAGT (GPT-review-fund B1): oploes KUN naar kommandoens program ER
    // profilens eget (stem-match) — masterens feed-kommando (custom_command=
    // false, program "uv") roeres ALDRIG. Ren funktion, unit-testet nedenfor.
    resolve_command_program(&term.profile, term.custom_command, &mut command)?;
    // `custom_command=false` alene er ikke nok: supervision-masteren er også
    // seedet som en claude-profil, men dens faktiske kommando er et read-only
    // feed. Stem-checket er data-drevet pr. profil (K1, spec rev 1.1).
    let readiness_spec = if term.custom_command {
        None
    } else {
        command.first().and_then(|program| {
            talminal_canvas_lib::profiles::readiness_for_command(&term.profile, program)
        })
    };
    // Spawn før xterm-attach er OK: PtyHost besvarer selv ConPTY's første
    // ESC[6n i reader-tråden (spike-FUND 1). Workeren arver brugerens env
    // (OAuth-arv, spec §20.3) + trusted identitet via TALMINAL_SESSION_ID.
    // TALMINAL_RUN_ID (fix F6): frisk uuid v4 pr. spawn — emitten
    // (persona_emit.py) stempler den ind i envelope-feltet run_id, saa
    // events kan korreleres til praecis dette spawn (respawn => nyt id).
    // Foer blev variablen aldrig sat, og run_id var altid null.
    let run_id = uuid::Uuid::new_v4().to_string();
    // Env-politik-split (Task 2): deny-felterne udfyldes fra kortets profil
    // (Task 5: registry-kort bærer profile-id'et; CC-only i MVP) —
    // nettoeffekt identisk med den tidligere hardcodede liste i pty.rs.
    // Nested-værnet er ubetinget i selve spawn-stien.
    let cc = talminal_canvas_lib::profiles::profile(&term.profile)
        .ok_or_else(|| format!("unknown agent profile: {}", term.profile))?;
    // Slut-review fix 1: HER registreres kort-tokenet (beslutning 12: begge
    // profiler, ikke kun codex) — inde i fase 2's
    // laas, EFTER `card already running`-revalideringen og efter de to
    // fallible trin (resolve_command_program, profil-opslaget). Tidligere
    // skete det i det bevidst ULAASTE mellem-vindue, hvor fase 1's
    // `pty.is_some()` ikke er nogen single-flight-garanti: to samtidige
    // spawn_card-kald (Card.tsx' spawnPromises-guard deles IKKE med
    // voice/dispatch.ts' executeRestartCard) kunne skrive hver sit token i
    // kortets ENE slot, hvorefter taberen returnerede paa
    // `card already running` UDEN at rydde. Kortets levende proces koerte da
    // med vinderens token, mens registryet holdt taberens => card_for_token
    // misser => mcp.rs afviser HVER request med -32600 foer method-dispatch,
    // ogsaa `initialize`. Browser-tools var doede for kortets levetid, uden
    // HUD-signal. Efter flytningen er den eneste tilbagevaerende fejlvej
    // PtyHost::spawn's Err, som allerede rydder (se nedenfor) — foerst da er
    // mcp.rs' "clear kaldes paa ALLE afslutningsveje" faktisk sand.
    // Laaseorden er uaendret kort->token (clear paa fejlvejen holder samme
    // `card`-guard), saa flytningen introducerer ingen inversion.
    // replace-semantikken i set_card_token daekker fortsat respawn: et evt.
    // tidligere token for kortet invalideres automatisk (GPT-review B3).
    //
    // FUND H1: config-FILEN skrives af samme grund her, umiddelbart foer
    // registreringen. Registry-halvdelen var lukket, men filen blev stadig
    // skrevet i det ulaaste mellem-vindue til en sti der er deterministisk pr.
    // kort. Taberens skrivning kunne derfor lande SIDST: registryet holdt
    // vinderens token, mens filen — og dermed den CC-proces vinderen spawnede —
    // bar taberens. Konsekvensen er total: siden H2 (mcp.rs) er Bearer
    // autoritativ for ALLE tools, saa et token-mismatch giver `InvalidToken` og
    // dermed en JSON-RPC-afvisning af hver eneste request foer method-dispatch,
    // ogsaa `initialize` — ikke laengere kun de tre Bearer-kraevende traad-tools.
    // Mellem-vinduets TREDJE doer — `emit_worker_degraded`s token-rydning — er
    // lukket samme sted, ved revalideringen ovenfor.
    //
    // Det er LOVLIGT at holde kortlaasen her, selvom funktions-doc'en forbyder
    // det for `worker_scope_for_spawn`: forbuddet gaelder kald der laver en
    // `on_main`-rundtur og blokerer paa svaret, mens main-traadens synkrone
    // kommandoer laaser samme kort-mutex. `write_config` er ren fil-IO
    // (create_dir_all + tmp-fil + rename, worker_mcp.rs) uden main-traads-
    // rundtur og uden andre laase, saa den cirkulaere ventning kan ikke opstaa.
    // Prisen er en fsync inde i laasen — den rammer kun samtidige kald paa
    // PRAECIS DETTE kort, og de er netop dem der skal serialiseres.
    //
    // Taberen naar aldrig hertil: den returnerer paa `card already running`
    // ovenfor og roerer hverken fil eller registry.
    if let Some((mcp_port, cdp_port, token)) = pending_config {
        #[cfg(feature = "perf-trace")]
        let config_started = Instant::now();
        match worker_mcp::write_config(name, mcp_port, cdp_port, &token) {
            Ok(path) => {
                talminal_canvas_lib::perf_mark!(
                    "create.mcp_config.end",
                    serde_json::json!({
                        "card": name,
                        "duration_ms": config_started.elapsed().as_secs_f64() * 1_000.0,
                        "ok": true,
                    }),
                );
                mcp_args.extend(worker_mcp::launch_args(&path));
                // Kun paa Ok-vejen: tokenet er beviset for at injektionen SKETE
                // (T13's verifikationstrin laeser `mcp::has_card_token`).
                // Fejlede config-skrivningen, spawner workeren uden tools — og
                // skal derfor heller ikke staa som token-baerer.
                card_token = Some(token);
            }
            Err(e) => {
                talminal_canvas_lib::perf_mark!(
                    "create.mcp_config.end",
                    serde_json::json!({
                        "card": name,
                        "duration_ms": config_started.elapsed().as_secs_f64() * 1_000.0,
                        "ok": false,
                    }),
                );
                emit_worker_degraded(app, name, &format!("worker-mcp config fejlede: {e}"));
            }
        }
    }
    // Injektions-argerne haeftes paa efter base-kommandoen. Punktet ligger nu
    // EFTER config-skrivningen (H1), saa begge injektionsveje — codex' `-c`-
    // overrides fra mellem-vinduet og ClaudeFlags' `--mcp-config` fra linjerne
    // ovenfor — samles i ét og samme push. Et kort gaar kun ad den ene vej, saa
    // den resulterende kommandolinje er uaendret.
    command.extend(mcp_args);
    if let Some(token) = &card_token {
        mcp::set_card_token(name, token);
    }
    let spec = PtySpawn {
        cwd: term.config.cwd.clone(),
        command,
        cols: 120,
        rows: 30,
        env_deny_prefixes: cc.env_deny_prefixes.iter().map(|s| s.to_string()).collect(),
        env_deny_exact: cc.env_deny_exact.iter().map(|s| s.to_string()).collect(),
        extra_env: {
            let mut env = vec![
                ("TALMINAL_SESSION_ID".to_string(), name.to_string()),
                ("TALMINAL_RUN_ID".to_string(), run_id),
            ];
            // Codex-injektionens Bearer-token (Task 4): tomt for alle andre
            // profiler/gates (ClaudeFlags-vejen faar identiteten via
            // header i --mcp-config-filen i stedet).
            env.extend(mcp_extra_env);
            env
        },
    };
    let out_app = app.clone();
    let out_name = name.to_string();
    // Task 8: output-gating — emitten wrappes i registry::gated_emit (delt
    // med tests/gating.rs), som dropper chunks når kortet er skjult (FUND 2:
    // alt-screen har ingen historik at miste). Læsningen i pty.rs' reader-
    // tråd fortsætter uanset (backpressure må aldrig ramme child'en).
    let visible = Arc::clone(&term.visible);
    // En frisk gate pr. PTY-run er også ABA-værnet: sen output fra en gammel
    // reader kan kun signalere sin gamle Arc, aldrig et respawn. Custom
    // commands har ingen Claude-promptkontrakt og beholder den hidtidige
    // direkte submit-adfærd.
    let submit_readiness = readiness_spec.map(|spec| Arc::new(PromptReadiness::new(spec)));
    let output_readiness = submit_readiness.as_ref().map(Arc::clone);
    // Opmaerksomheds-maskinen (T7): selvstaendig af readiness — den staar READY
    // gennem hele en manuelt tastet arbejdsgang, saa prikken kan ikke udledes af
    // den. Oprettes ved HVERT terminal-spawn med profilens moenstre (ogsaa for
    // custom commands: profilen er stadig kortets); browser-/chat-kort naar
    // aldrig hertil.
    let attention = Arc::new(workspaces::attention::CardAttention::new(
        cc.attention_patterns,
    ));
    // Maskinen fødes "synlig". Spawnes kortet mens workspacet er skjult (fx
    // restore i et vindue der endnu ikke er revealed), ville den derfor aldrig
    // tænde før næste conceal-kant — pollertråden sender kun kanter.
    // (`workspace_skjult` er laest FOER kortlaasen — se laaseorden-noten der;
    // den efterproeves af `sync_initial_visibility` efter `drop(card)`.)
    if workspace_skjult {
        attention.on_conceal();
    }
    let output_attention = Arc::clone(&attention);
    #[cfg(feature = "perf-trace")]
    let trace_id = perf_trace::current_trace_id();
    // Ingen atomisk operation paa den normale PTY-hotpath, naar maaling er
    // slaaet fra. Under en create-trace registreres kun den foerste chunk.
    #[cfg(feature = "perf-trace")]
    let first_output_emit = trace_id
        .as_ref()
        .map(|_| std::sync::atomic::AtomicBool::new(false));
    let visible_emit = registry::gated_emit(visible, move |bytes: &[u8]| {
        #[cfg(feature = "perf-trace")]
        if first_output_emit
            .as_ref()
            .is_some_and(|first| !first.swap(true, std::sync::atomic::Ordering::Relaxed))
        {
            talminal_canvas_lib::perf_mark_for!(
                trace_id.as_deref(),
                "create",
                "create.pty.first_output_rust",
                serde_json::json!({ "card": out_name, "bytes": bytes.len() }),
            );
        }
        let data_b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        let _ = out_app.emit(
            "pty-output",
            PtyOutputEvent {
                name: out_name.clone(),
                data_b64,
            },
        );
    });
    #[cfg(feature = "perf-trace")]
    let pty_started = Instant::now();
    let host = match PtyHost::spawn_sequenced(spec, move |sequence, bytes: &[u8]| {
        // Readiness-observation ligger bevidst UDEN FOR visibility-gaten:
        // skjulte kort skal stadig kunne blive klar, og readeren må aldrig
        // blokere på frontendens LOD/occlusion-state.
        if let Some(readiness) = &output_readiness {
            readiness.observe(sequence, bytes);
        }
        // Samme grund som readiness: fodringen ligger UDEN FOR visibility-gaten
        // — det er praecis de skjulte kort, prikken findes for. Laasen holdes
        // kun over feltopdateringer, aldrig over IO.
        output_attention.on_output(workspaces::attention::now_ms(), bytes);
        visible_emit(bytes);
    }) {
        Ok(host) => host,
        Err(error) => {
            if let Some(readiness) = &submit_readiness {
                readiness.cancel();
            }
            // Attention-maskinen naaede aldrig ind i registryet (den saettes
            // foerst efter et lykkedes spawn nedenfor), saa den doer med denne
            // ramme sammen med readerens klon — intet kort kan holde en prik.
            drop(attention);
            // B3: injektionen kan have saettet et token (codex-vejen) foer
            // selve PTY-spawnet fejlede — den doede koersel maa ikke
            // efterlade et gyldigt token. Idempotent for ClaudeFlags/ingen
            // injektion.
            release_card_identity(name);
            return Err(error.to_string());
        }
    };
    talminal_canvas_lib::perf_mark!(
        "create.pty_spawn.end",
        serde_json::json!({
            "card": name,
            "duration_ms": pty_started.elapsed().as_secs_f64() * 1_000.0,
        }),
    );
    // Fix F17: deterministisk PID-log ved hvert spawn — Task 12's run-book
    // kill-verificerer KUN mod disse dokumenterede worker-PID'er (aldrig
    // `tasklist | findstr claude`, som også matcher fremmede CC-sessioner).
    match host.process_id() {
        Some(pid) => eprintln!(
            "[canvas] card={name} agent_pid={pid} profile={}",
            term.profile
        ),
        None => eprintln!(
            "[canvas] card={name} agent_pid=unknown profile={}",
            term.profile
        ),
    }
    let host = Arc::new(host);
    term.submit_readiness = submit_readiness;
    // Klonen tages FOERST her, saa spawn-fejlvejen ovenfor forbliver eneejer og
    // dens eksplicitte `drop(attention)` beholder sin betydning.
    let attention_sync = Arc::clone(&attention);
    term.attention = Some(attention);
    term.pty = Some(Arc::clone(&host));
    card.exited = None; // fix F10: nyt run — ryd sidste exit-status
    drop(card);
    // TOCTOU-lukning. Maskinen er nu i registryet, men foedsels-gaten ovenfor
    // blev laest FOER kortlaasen og altsaa foer hele PTY-spawnet: naaede
    // pollertraaden sin Reveal-gren imens, fandt dens
    // `set_attention_visibility(true)` ingen maskine paa kortet, og kortet ville
    // staa `workspace_visible = false` MENS workspacet er fremme — prikken
    // taendte efter 2 s stilhed paa det workspace ejeren sad og kiggede paa, og
    // rettede sig foerst ved naeste conceal->reveal (kanttrigget). Kaldes EFTER
    // `drop(card)`, saa laaseordenen fra noten ved foedsels-gaten holder.
    workspaces::attention::sync_initial_visibility(
        &attention_sync,
        workspaces::attention_visibility_now,
        !workspace_skjult,
    );
    // Kortet har nu en levende session. `running_cards` i status.json skal
    // afspejle det med det samme — spawn/respawn gaar ikke gennem
    // create_card_persisted, saa uden dette kald ville tallet foerst blive
    // rettet af et tilfaeldigt create/close et helt andet sted. Kaldes EFTER
    // `drop(card)`: refresh laaser kortene selv (laaseorden).
    workspaces::status::refresh_card_counts();
    start_exit_watcher(app.clone(), name.to_string(), Arc::clone(&handle), host);
    talminal_canvas_lib::perf_mark!(
        "create.spawn_into.end",
        serde_json::json!({
            "card": name,
            "duration_ms": spawn_started.elapsed().as_secs_f64() * 1_000.0,
        }),
    );
    Ok(())
}

/// Melder "card-exit" når childen dør — via non-blocking try_exit_status
/// (Task 6's kanoniske exit-poll), ALDRIG via reader-EOF (spike-FUND 11:
/// EOF kommer først efter master-drop). Single-fire: pty.take() FØR teardown.
fn start_exit_watcher(
    app: AppHandle,
    name: CardId,
    runtime: Arc<std::sync::Mutex<registry::CardRuntime>>,
    watched_host: Arc<PtyHost>,
) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(500));
        if let Some(code) = watched_host.try_exit_status() {
            let exited = {
                let Ok(mut card) = runtime.lock() else { return };
                // Instans-identitet, ikke blot kortnavn: efter et sequence-
                // reset kan et nyt card-1 eksistere, mens en gammel watcher
                // vaagner. Den maa aldrig tage eller emitte for den nye PTY.
                // Terminal-only pr. konstruktion (watcheren startes kun af
                // spawn_into) — browser-grenen falder ud via terminal().
                let is_watched = card
                    .terminal()
                    .and_then(|t| t.pty.as_ref())
                    .is_some_and(|host| Arc::ptr_eq(host, &watched_host));
                if !card.active || !is_watched {
                    return;
                }
                card.exited = Some(code); // fix F10: status til get_card_state
                let Some(term) = card.terminal_mut() else {
                    return;
                };
                if let Some(readiness) = term.submit_readiness.take() {
                    readiness.cancel();
                }
                // Kortets proces er doed — prikken maa doe med den.
                term.attention = None;
                term.pty.take()
            };
            // B3: naturlig proces-exit er en afslutningsvej — naaes kun her
            // naar `is_watched` bekraeftede at det VAR den aktive koersel for
            // dette kortnavn, saa tokenet (hvis noget) skal ryddes nu.
            release_card_identity(&name);
            // Sessionen doede af sig selv. Kortlaasen er sluppet ovenfor, saa
            // tallet kan genberegnes her (laaseorden) — ellers ville
            // lukke-bekraeftelsen advare om en koerende session der er doed.
            workspaces::status::refresh_card_counts();
            // Childen er allerede død; teardown-rækkefølgen (drop writer →
            // dræn → drop master → join reader) frigør reader-tråden. Kører
            // på watcher-tråden — aldrig på UI-tråden (fix F2).
            if let Some(host) = exited {
                let _ = host.kill_and_teardown();
            }
            // Hold runtime-laasen over sidste active-check + emit. En samtidig
            // close maa saette active=false, foer den kan resettere navnet;
            // enten lander eventet foer reset, eller ogsaa undertrykkes det.
            if let Ok(card) = runtime.lock() {
                if card.active {
                    let _ = app.emit("card-exit", CardExitEvent { name: name.clone() });
                }
            }
            // T11 daekker close_card/close_cards, men et kort hvis proces doer
            // af sig selv gaar gennem exit-watcheren. Uden dette hook venter en
            // delegering paa et kort der ikke laengere koerer, indtil uret
            // loeber ud. Kaldes UDEN kort-laas holdt (laaseorden §5.6).
            let chat_thread = registry::chat_thread_id(&name);
            threads::on_card_gone(&name, chat_thread.as_deref());
            return;
        }
    });
}

#[tauri::command]
async fn spawn_card(app: AppHandle, name: String) -> Result<(), String> {
    run_blocking(move || {
        spawn_into(&app, &name, false)?;
        // Task 6 (bindende WorkspaceCard-kontrakt): last_active_at ved spawn.
        workspace::touch_card_activity(&name);
        Ok(())
    })
    .await
}

/// Recovery-vejen (M0b-gate): respawn bruger resume_command (claude --continue).
#[tauri::command]
async fn respawn_card(app: AppHandle, name: String) -> Result<(), String> {
    run_blocking(move || {
        spawn_into(&app, &name, true)?;
        // Task 6: last_active_at ved (re)spawn.
        workspace::touch_card_activity(&name);
        Ok(())
    })
    .await
}

/// Input-serialiseringspunktet (spec §5 + §6 regel 1/4).
/// source=="human": første tast mens personaen har roret → SYNKRONT epoch-bump
/// (auto-pause, sker i gate_write) + pause-signalfil + "pause-state"-event, FØR
/// bytes rører pty'en; bytes skrives ALTID igennem. source=="terminal" (fix F1):
/// xterm's protokol-auto-svar (CPR/DA/fokus/kitty) — altid PassThrough, ingen
/// pause-semantik. source=="persona": mekanisk epoch-check; stale ⇒
/// Err("stale_epoch") — ubrugt live i v0 (dispatcheren er M2.5), men
/// semantikken er testet i epoch::tests.
/// Fix F2: kort-låsen holdes KUN over gate-beslutning + Arc-klon — den
/// blokerende write sker EFTER låsen er sluppet (ét korts backpressure må
/// hverken spærre andre kommandoer eller kill/teardown).
#[tauri::command]
async fn write_pty(
    app: AppHandle,
    name: String,
    data: String,
    epoch: u64,
    source: String,
) -> Result<(), String> {
    // Task 11 (ejer-beslutning 1, supervision-sporet): master er et read-only
    // feed-viewport uden pause-semantik. Mekanisk guard: ingen bytes og ingen
    // pause-signalfiler for "master", uanset frontend-fejl —
    // Err("master_readonly"), testet i tests/cards_master.rs. (NB: afviser også
    // source=="terminal"-writes for master — harmløst: persona feed enabler
    // hverken mode 1004 eller sender queries efter den udsplejsede første.)
    // Task 3: i default-state er guarden en no-op (master-kortet er ude af
    // MVP-pathen; testet i tests/master_off.rs) — kald-stedet er ens i begge
    // feature-states.
    guard_write_pty(&name)?;
    run_blocking(move || {
        let handle = registry::card_handle(&name)?;
        let host = {
            let card = handle.lock().map_err(|e| e.to_string())?;
            // Plan Task 2-fejlkontrakt: PTY-veje afviser browser-kort.
            let Some(term) = card.terminal() else {
                return Err(format!("card is a browser: {name}"));
            };
            let host = term
                .pty
                .as_ref()
                .cloned()
                .ok_or_else(|| format!("card not running: {name}"))?;
            // T7: den ENESTE vej brugerens tastetryk gaar. Agent-drevne
            // skrivninger OG xterm's protokol-auto-svar gaar ogsaa herigennem,
            // saa `source` skal med — maskinen rydder kun for "human". Ellers
            // ville webviewets fokus-svar (\x1b[I paa mode 1004, som CC enabler)
            // slukke prikken i samme oejeblik ejeren kigger paa workspacet.
            if let Some(attention) = &term.attention {
                attention.on_input(&source);
            }
            // Task 1: beslutningen bor i control-facaden (begge feature-states).
            // Supervision: epoch er allerede bumpet atomart og pause-signalfilen
            // allerede skrevet i facaden ved WriteAfterPause — eventet her er
            // notifikation og må ALDRIG blokere menneskets tastetryk.
            // Default-state: altid Write — alle epoch/source-værdier accepteres,
            // ingen signalfiler, intet pause-state-event (frossen IPC-kontrakt).
            #[cfg(feature = "supervision")]
            let outcome = {
                let state = app.state::<AppState>();
                control::gate_pty_write(&term.gate, &state.signals_dir, &name, &source, epoch)
            };
            #[cfg(not(feature = "supervision"))]
            let outcome = control::gate_pty_write(&term.gate, &source, epoch);
            match outcome {
                WriteOutcome::RejectStale => return Err("stale_epoch".to_string()),
                WriteOutcome::WriteAfterPause { new_epoch } => {
                    let _ = app.emit(
                        "pause-state",
                        PauseStateEvent {
                            name: name.clone(),
                            owner: "human".to_string(),
                            epoch: new_epoch,
                        },
                    );
                }
                WriteOutcome::Write => {}
            }
            host
        }; // kort-låsen slippes HER — før den blokerende write (fix F2)
        host.write(data.as_bytes()).map_err(|e| e.to_string())?;
        // Task 6 (bindende WorkspaceCard-kontrakt): last_active_at ved
        // write_pty — debounced persist, blokerer aldrig tastetrykket.
        workspace::touch_card_activity(&name);
        Ok(())
    })
    .await
}

#[tauri::command]
async fn resize_pty(name: String, cols: u16, rows: u16) -> Result<(), String> {
    run_blocking(move || {
        let handle = registry::card_handle(&name)?;
        let host = {
            let card = handle.lock().map_err(|e| e.to_string())?;
            let Some(term) = card.terminal() else {
                return Err(format!("card is a browser: {name}"));
            };
            term.pty
                .as_ref()
                .cloned()
                .ok_or_else(|| format!("card not running: {name}"))?
        };
        host.resize(cols, rows).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn kill_card(app: AppHandle, name: String) -> Result<(), String> {
    run_blocking(move || {
        let handle = registry::card_handle(&name)?;
        let host = {
            let mut card = handle.lock().map_err(|e| e.to_string())?;
            // Browser-kort roeres ALDRIG af kill (overlevelse, spec §3).
            let Some(term) = card.terminal_mut() else {
                return Err(format!("card is a browser: {name}"));
            };
            // pty'en tages FØR teardown — exit-watcheren ser None og melder
            // ikke dobbelt (single-fire bevaret). Kill-vejen er operatør-
            // initieret: exit-status kendes ikke — markér som stoppet.
            let host = term
                .pty
                .take()
                .ok_or_else(|| format!("card not running: {name}"))?;
            if let Some(readiness) = term.submit_readiness.take() {
                readiness.cancel();
            }
            // Drabt koersel: prikken maa ikke overleve den.
            term.attention = None;
            term.visible
                .store(false, std::sync::atomic::Ordering::Relaxed);
            card.exited = Some(0); // fix F10: get_card_state viser 'ikke kørende, har kørt'
            host
        }; // kort-låsen slippes FØR teardown (fix F2)
           // B3: kill er en afslutningsvej — kortets token (hvis noget) maa
           // ikke overleve den drabte koersel.
        release_card_identity(&name);
        // Operatoer-drab fjerner en levende session uden at gaa gennem
        // close_cards_persisted. Kortlaasen er sluppet, saa tallet genberegnes
        // her (laaseorden) — ellers ville status.json blive staaende og advare
        // om sessioner der er draebt.
        workspaces::status::refresh_card_counts();
        // kill_and_teardown(&self) går via child-Mutex'en (adskilt fra
        // writer-Mutex'en) — en hængende writer blokerer ikke kill.
        host.kill_and_teardown().map_err(|e| e.to_string())?;
        // Samme ABA-værn som exit-watcheren: hvis close har detached denne
        // runtime under teardown, maa dens sene event ikke ramme et nyt card-N.
        let card = handle.lock().map_err(|e| e.to_string())?;
        if card.active {
            let _ = app.emit("card-exit", CardExitEvent { name: name.clone() });
        }
        Ok(())
    })
    .await
}

/// Genoptag-knappen: personaen får roret igen (nyt epoch-vindue) + resume-
/// signalfil til controlleren + "pause-state"-event til UI'et.
/// Fix F20: signalfilen skrives FØR gaten muteres (i control-facaden) —
/// fejler skrivningen, returneres Err og gate/UI/controller er stadig i sync.
/// Kort-låsen holdes over compute+write+bump (samme lås som write_pty's
/// gate-vej), så intet andet bump kan skyde sig ind.
/// Default-state (frossen IPC-kontrakt): no-op der returnerer Ok(()) —
/// ingen signalfil, intet bump, intet pause-state-event.
#[tauri::command]
async fn resume_card_control(app: AppHandle, name: String) -> Result<(), String> {
    // Task 11 (supervision-sporet): master har ingen pause-semantik at genoptage
    // (Err("master_has_no_pause"), testet i tests/cards_master.rs). Task 3:
    // default-state er guarden en no-op (testet i tests/master_off.rs).
    guard_resume_control(&name)?;
    run_blocking(move || {
        let handle = registry::card_handle(&name)?;
        let card = handle.lock().map_err(|e| e.to_string())?;
        // Browser-kort har ingen pause-semantik (plan Task 2-fejlkontrakt).
        let Some(term) = card.terminal() else {
            return Err(format!("card is a browser: {name}"));
        };
        #[cfg(feature = "supervision")]
        {
            let state = app.state::<AppState>();
            let new_epoch = control::resume_control(&term.gate, &state.signals_dir, &name)?;
            let _ = app.emit(
                "pause-state",
                PauseStateEvent {
                    name: name.clone(),
                    owner: "persona".to_string(),
                    epoch: new_epoch,
                },
            );
        }
        #[cfg(not(feature = "supervision"))]
        {
            let _ = &app; // default-state: app bruges kun i supervision-grenen
            control::resume_control(&term.gate)?;
        }
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// Task 1 Del B: kommando-sockets — TYNDE wrappers over lib-modulerne.
// Task 5 udfyldte registry-socketsene (create/close/list), Task 6 workspace-
// socketsene (geometri/viewport/get_workspace) + synkron persist ved
// create/close; resten er doede (Err("not implemented")) indtil deres task
// udfylder dem.
// ---------------------------------------------------------------------------

/// Task 6: create + SYNKRON workspace-persist (lib-helperen kalder
/// registry::create_card og skriver workspace.json atomisk).
/// EJER-AMENDMENT (2026-07-19): kortet AUTOSTARTER straks efter create —
/// ingen Start-knap for nye kort ("åbn 3 terminaler" = 3 kørende Claude).
/// Spawn er best-effort: fejler den (fx død cwd), returneres kortet
/// u-spawnet, og exit-overlayets manuelle vej består som fallback.
/// Async + run_blocking: PtyHost::spawn må aldrig køre på UI-/main-tråden.
#[cfg(feature = "perf-trace")]
#[tauri::command]
async fn create_card(
    app: AppHandle,
    cwd: String,
    profile: Option<String>,
    command: Option<String>,
    perf_trace: Option<String>,
    perf_started_ms: Option<f64>,
) -> Result<registry::CardInfo, String> {
    create_card_impl(app, cwd, profile, command, perf_trace, perf_started_ms).await
}

#[cfg(not(feature = "perf-trace"))]
#[tauri::command]
async fn create_card(
    app: AppHandle,
    cwd: String,
    profile: Option<String>,
    command: Option<String>,
) -> Result<registry::CardInfo, String> {
    create_card_impl(app, cwd, profile, command, None, None).await
}

#[inline(always)]
#[cfg_attr(not(feature = "perf-trace"), allow(unused_variables))]
async fn create_card_impl(
    app: AppHandle,
    cwd: String,
    profile: Option<String>,
    command: Option<String>,
    perf_trace: Option<String>,
    perf_started_ms: Option<f64>,
) -> Result<registry::CardInfo, String> {
    #[cfg(feature = "perf-trace")]
    let trace_id = perf_trace.clone();
    talminal_canvas_lib::perf_mark_for!(
        trace_id.as_deref(),
        "create",
        "create.command.received",
        serde_json::json!({ "cwd": cwd, "profile": profile }),
    );
    #[cfg(feature = "perf-trace")]
    let context = perf_trace.map(|id| perf_trace::new_context(id, "create", perf_started_ms));
    let result = run_blocking(move || {
        talminal_canvas_lib::perf_with_context!(context, {
            talminal_canvas_lib::perf_mark!("create.blocking.enter", serde_json::json!({}));
            #[cfg(feature = "perf-trace")]
            let persist_started = Instant::now();
            let info = workspace::create_card_persisted(cwd, profile, command)?;
            talminal_canvas_lib::perf_mark!(
                "create.workspace.end",
                serde_json::json!({
                    "card": info.name,
                    "duration_ms": persist_started.elapsed().as_secs_f64() * 1_000.0,
                }),
            );
            let created = match spawn_into(&app, &info.name, false) {
                Ok(()) => {
                    workspace::touch_card_activity(&info.name);
                    // Genlæs info så running=true når frontenden ser svaret.
                    Ok(registry::list_cards()
                        .into_iter()
                        .find(|c| c.name == info.name)
                        .unwrap_or(info))
                }
                Err(e) => {
                    // N4 (spec brief T9): en fejlet best-effort-spawn maa
                    // ikke vaere lydloes for operatoeren — HUD'ens fejlkanal
                    // skal vise den. Triggerne er exe-oploesningen
                    // (resolve_command_program, fx codex ikke paa PATH),
                    // profil-opslaget og selve PTY-spawnet. En SLETTET cwd er
                    // IKKE en trigger: portable-pty's current_directory
                    // falder tavst tilbage til %USERPROFILE%.
                    // T4's kort-noeglede MCP-token-livscyklus: et evt. token
                    // sat i spawn_into's fase 2 (Codex-injektion) skal ryddes
                    // naar kortet aldrig kom i gang. Efter slut-review fix 1
                    // er dette baelte + seler — spawn_into rydder selv paa
                    // sin ene tilbagevaerende fejlvej — men clear er
                    // idempotent, og vagten daekker fremtidige fejlveje.
                    release_card_identity(&info.name);
                    eprintln!("[canvas] create: card '{}' not autostarted: {e}", info.name);
                    let _ = app.emit(
                        "card-spawn-failed",
                        spawn_failed_payload(&info.name, info.number, &e),
                    );
                    Ok(info)
                }
            };
            workspaces::status::refresh_card_counts();
            created
        })
    })
    .await;
    talminal_canvas_lib::perf_mark_for!(
        trace_id.as_deref(),
        "create",
        "create.command.returning",
        serde_json::json!({
            "ok": result.is_ok(),
            "card": result.as_ref().ok().map(|info| info.name.as_str()),
        }),
    );
    result
}

/// Async + run_blocking (fix F2): close river en pty ned (normativ teardown,
/// op til ~10 s kill-vent) — det maa ALDRIG ske paa UI-/main-traaden.
/// IPC-wire-signaturen (name → Result<(), String>) er uaendret fra Del B.
/// Browser-cards: kaskade-luk — `expand_close_targets` FOERST (tager ejede
/// browser-kort med), derefter native webview-preclose, persisted registry-
/// close og til sidst scope-/event-finish. Single-close-fejlfladen bevares.
#[cfg(feature = "perf-trace")]
#[tauri::command]
async fn close_card(
    app: AppHandle,
    name: String,
    perf_trace: Option<String>,
    perf_started_ms: Option<f64>,
) -> Result<(), String> {
    close_card_impl(app, name, perf_trace, perf_started_ms).await
}

#[cfg(not(feature = "perf-trace"))]
#[tauri::command]
async fn close_card(app: AppHandle, name: String) -> Result<(), String> {
    close_card_impl(app, name, None, None).await
}

/// Enkelt-close er batch-close med ét target plus et opslag i fejllisten —
/// samme reduktion som `registry::close_card` allerede laver over
/// `close_cards` et lag laengere nede. De to kommandoer havde tidligere hver
/// sin kopi af hele kaskaden (expand → lifecycle → token-clear → count-refresh);
/// enhver aendring i close-stien skulle skrives to steder, og token-loekken var
/// skrevet to gange.
#[inline(always)]
async fn close_card_impl(
    app: AppHandle,
    name: String,
    perf_trace: Option<String>,
    perf_started_ms: Option<f64>,
) -> Result<(), String> {
    let result = close_cards_impl(
        app,
        vec![name.clone()],
        "single",
        perf_trace,
        perf_started_ms,
    )
    .await?;
    match result.errors.iter().find(|error| error.name == name) {
        Some(error) => Err(error.message.clone()),
        None => Ok(()),
    }
}

/// Batch-close til voice/UI: én lifecycle-transaktion, én workspace-persist
/// og parallel PTY-teardown. Resultatet er altid struktureret pr. target.
/// Browser-cards: samme kaskade som close_card (expand → native preclose →
/// persist → finish). kill_card/respawn_card roerer ALDRIG browser-kort (§3).
#[cfg(feature = "perf-trace")]
#[tauri::command]
async fn close_cards(
    app: AppHandle,
    names: Vec<String>,
    perf_trace: Option<String>,
    perf_started_ms: Option<f64>,
) -> Result<registry::CloseCardsResult, String> {
    close_cards_impl(app, names, "batch", perf_trace, perf_started_ms).await
}

#[cfg(not(feature = "perf-trace"))]
#[tauri::command]
async fn close_cards(
    app: AppHandle,
    names: Vec<String>,
) -> Result<registry::CloseCardsResult, String> {
    close_cards_impl(app, names, "batch", None, None).await
}

#[inline(always)]
#[cfg_attr(not(feature = "perf-trace"), allow(unused_variables))]
async fn close_cards_impl(
    app: AppHandle,
    names: Vec<String>,
    mode: &'static str,
    perf_trace: Option<String>,
    perf_started_ms: Option<f64>,
) -> Result<registry::CloseCardsResult, String> {
    #[cfg(feature = "perf-trace")]
    let trace_id = perf_trace.clone();
    talminal_canvas_lib::perf_mark_for!(
        trace_id.as_deref(),
        "close",
        "close.command.received",
        serde_json::json!({ "names": names, "mode": mode }),
    );
    #[cfg(feature = "perf-trace")]
    let context = perf_trace.map(|id| perf_trace::new_context(id, "close", perf_started_ms));
    let result = run_blocking(move || {
        talminal_canvas_lib::perf_with_context!(context, {
            talminal_canvas_lib::perf_mark!(
                "close.blocking.enter",
                serde_json::json!({ "mode": mode }),
            );
            #[cfg(feature = "perf-trace")]
            let expanded_started = Instant::now();
            let expanded = registry::expand_close_targets(names);
            talminal_canvas_lib::perf_mark!(
                "close.expand.end",
                serde_json::json!({
                    "names": expanded,
                    "duration_ms": expanded_started.elapsed().as_secs_f64() * 1_000.0,
                }),
            );
            let result = browser_host::close_cards_lifecycle(
                &app,
                expanded,
                workspace::close_cards_persisted,
            )?;
            // B3: close er en afslutningsvej — ryd token for hvert kort der
            // faktisk blev detached fra registryet (kaskaden kan omfatte
            // browser-kort uden token; clear er idempotent).
            for closed_name in &result.closed {
                release_card_identity(closed_name);
            }
            workspaces::status::refresh_card_counts();
            Ok(result)
        })
    })
    .await;
    talminal_canvas_lib::perf_mark_for!(
        trace_id.as_deref(),
        "close",
        "close.command.returning",
        serde_json::json!({
            "ok": result.is_ok(),
            "mode": mode,
            "closed": result.as_ref().ok().map(|result| result.closed.len()),
            // Pr.-target-fejl er ikke en Err paa transaktionen; uden dette felt
            // ville en enkelt-close der fejlede se groen ud i traceet.
            "errors": result.as_ref().ok().map(|result| result.errors.len()),
        }),
    );
    result
}

#[tauri::command]
fn list_cards() -> Vec<registry::CardInfo> {
    registry::list_cards()
}

#[tauri::command]
fn update_card_geometry(name: String, x: f64, y: f64, w: f64, h: f64) -> Result<(), String> {
    workspace::update_card_geometry(name, x, y, w, h)
}

#[tauri::command]
fn set_viewport(x: f64, y: f64, zoom: f64) -> Result<(), String> {
    workspace::set_viewport(x, y, zoom)
}

#[tauri::command]
fn get_workspace() -> Result<workspace::WorkspaceResponse, String> {
    workspace::get_workspace()
}

#[tauri::command]
fn set_settings(settings: workspace::SettingsInput) -> Result<(), String> {
    workspace::set_settings(settings)
}

/// Pty-gating (Task 8): flaget bor paa CardRuntime (registry.rs) og
/// suppressions-logikken i registry::gated_emit — wrapperen er tynd
/// (Testbarhed-reglen: tests/gating.rs naar logikken via lib-cratet).
#[tauri::command]
fn set_card_visible(name: String, visible: bool) -> Result<(), String> {
    registry::set_card_visible(name, visible)
}

// ---------------------------------------------------------------------------
// Browser-kort (browser-cards plan Task 5). Webview-livscyklussen bor i
// browser_host.rs (AppHandle-laget); disse er tynde kommando-wrappers.
//
// TRAAD-KONTRAKT (spike S1): kommandoer der OPRETTER/navigerer webviews er
// async (koerer paa run_blocking-poolen off-main; webview-ops via
// run_on_main_thread). De lette praesentations-kommandoer er SYNKRONE — de
// koerer paa main og roerer eksisterende webviews direkte (ingen add_child,
// ingen reentrancy-deadlock).
// ---------------------------------------------------------------------------

/// Aabn et browser-kort. `url` None ⇒ about:blank; ellers kun http/https.
/// `opened_by` = ejer-terminalens navn (None = canvas-aabnet). Fejler
/// webview-/CDP-oprettelsen, ryddes det halve kort (browser_host).
#[tauri::command]
async fn create_browser_card(
    app: AppHandle,
    url: Option<String>,
    opened_by: Option<String>,
) -> Result<registry::CardInfo, String> {
    run_blocking(move || {
        browser_host::open_browser_card(&app, url.as_deref(), opened_by.as_deref())
            .map(|opened| opened.info)
    })
    .await
}

#[tauri::command]
async fn navigate_browser_card(app: AppHandle, name: String, url: String) -> Result<(), String> {
    run_blocking(move || browser_host::navigate_card(&app, &name, &url)).await
}

#[tauri::command]
fn set_browser_bounds(
    app: AppHandle,
    name: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Result<(), String> {
    browser_host::set_bounds(&app, &name, x, y, w, h)
}

#[tauri::command]
fn set_browser_occlusion(app: AppHandle, occluded: bool) -> Result<(), String> {
    browser_host::set_occlusion(&app, occluded)
}

#[tauri::command]
fn set_browser_fullscreen(app: AppHandle, name: Option<String>) -> Result<(), String> {
    browser_host::set_fullscreen(&app, name)
}

#[tauri::command]
fn focus_browser_card(app: AppHandle, name: String) -> Result<(), String> {
    browser_host::focus_card(&app, &name)
}

#[tauri::command]
fn store_secret(key: String, value: String) -> Result<(), String> {
    secrets::store_secret(key, value)
}

#[tauri::command]
fn load_secret(key: String) -> Result<Option<bool>, String> {
    secrets::load_secret(key).map(secrets::secret_presence)
}

#[tauri::command]
async fn mint_transcription_secret() -> Result<secrets::RealtimeClientSecret, String> {
    secrets::mint_transcription_secret().await
}

#[cfg(feature = "perf-trace")]
#[tauri::command]
async fn router_chat_completion(
    body: String,
    perf_trace: Option<String>,
    perf_started_ms: Option<f64>,
) -> Result<String, String> {
    router_chat_completion_impl(body, perf_trace, perf_started_ms).await
}

#[cfg(not(feature = "perf-trace"))]
#[tauri::command]
async fn router_chat_completion(body: String) -> Result<String, String> {
    router_chat_completion_impl(body, None, None).await
}

#[inline(always)]
#[cfg_attr(not(feature = "perf-trace"), allow(unused_variables))]
async fn router_chat_completion_impl(
    body: String,
    perf_trace: Option<String>,
    perf_started_ms: Option<f64>,
) -> Result<String, String> {
    #[cfg(feature = "perf-trace")]
    let started = Instant::now();
    talminal_canvas_lib::perf_mark_for!(
        perf_trace.as_deref(),
        "voice",
        "voice.router.rust.begin",
        serde_json::json!({
            "body_bytes": body.len(),
            "client_started_ms": perf_started_ms,
        }),
    );
    let result = secrets::router_chat_completion(body).await;
    talminal_canvas_lib::perf_mark_for!(
        perf_trace.as_deref(),
        "voice",
        "voice.router.rust.end",
        serde_json::json!({
            "duration_ms": started.elapsed().as_secs_f64() * 1_000.0,
            "ok": result.is_ok(),
        }),
    );
    result
}

#[cfg(feature = "perf-trace")]
#[tauri::command]
fn perf_trace_frontend(
    trace_id: String,
    kind: String,
    client_started_ms: f64,
    marks: Vec<perf_trace::FrontendMark>,
    final_batch: bool,
) {
    perf_trace::write_frontend_batch(&trace_id, &kind, client_started_ms, marks, final_batch);
}

// Keep the IPC command name/argument contract present in normal builds. The
// frontend's normal bundle never calls it, but a mismatched cached webview is
// accepted and discarded without pulling trace payload types into the binary.
#[cfg(not(feature = "perf-trace"))]
#[tauri::command]
fn perf_trace_frontend(
    trace_id: String,
    kind: String,
    client_started_ms: f64,
    marks: Vec<serde_json::Value>,
    final_batch: bool,
) {
    let _ = (trace_id, kind, client_started_ms, marks, final_batch);
}

#[tauri::command]
async fn tts_speech(body: String) -> Result<String, String> {
    secrets::tts_speech(body).await
}

#[tauri::command]
async fn probe_router_route() -> Result<String, String> {
    secrets::probe_router_route().await
}

#[tauri::command]
async fn warm_voice_connections() {
    secrets::warm_voice_connections().await;
}

/// Aabner Windows' mikrofon-privatlivsside.
///
/// Den indstilling ligger OVER WebView2: er adgangen slaaet fra dér — af
/// brugeren eller af en politik paa en arbejds-PC — hjaelper ingen
/// permission-handler, og `getUserMedia` fejler uanset hvad appen goer. Uden
/// denne genvej skal brugeren selv gaette sig frem gennem Indstillinger mens
/// mikrofonen er stum.
///
/// `ms-settings:` er Windows' egen URI-ordning og aabnes af skallen. Kaldet
/// gaar gennem `explorer.exe`, som er den vej der ikke kraever en shell-API og
/// virker ens paa alle understoettede versioner.
#[tauri::command]
fn open_microphone_settings() -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg("ms-settings:privacy-microphone")
        .spawn()
        // explorer.exe returnerer en ikke-nul exitkode selv naar siden AABNES,
        // saa der ventes bevidst ikke paa status: at processen blev startet er
        // alt vi kan og skal love.
        .map(|_| ())
        .map_err(|error| format!("kunne ikke aabne mikrofon-indstillingerne: {error}"))
}

#[tauri::command]
async fn tts_speech_stream(
    body: String,
    on_chunk: tauri::ipc::Channel<String>,
) -> Result<(), String> {
    secrets::tts_speech_stream(body, move |chunk| {
        let _ = on_chunk.send(chunk);
    })
    .await
}

#[tauri::command]
fn delete_secret(key: String) -> Result<(), String> {
    secrets::delete_secret(key)
}

#[tauri::command]
async fn submit_prompt(name: String, text: String) -> Result<(), String> {
    // Prompt-readiness kan vente op til den eksplicitte deadline, og den
    // eksisterende submit-koreografi sover 350 ms mellem tekst og CR. Ingen af
    // delene må ligge på Tauri-/UI-tråden.
    run_blocking(move || submit::submit_prompt(name, text)).await
}

#[tauri::command]
fn read_transcript_tail(
    name: String,
    max_entries: u32,
) -> Result<transcripts::TranscriptTail, String> {
    // T8: kortets profil slaas op i registryet her (main.rs-laget), saa
    // transcripts.rs' profil-bevidste indgang kan vaelge den rigtige rod
    // (claude vs. codex) uden selv at kende registryet.
    let card = registry::list_cards()
        .into_iter()
        .find(|card| card.name == name)
        .ok_or_else(|| format!("unknown card '{name}'"))?;
    // Browser-kort har ingen cwd/CC-session at laese transcript fra
    // (plan Task 2-fejlkontrakt; voice-laget svarer med titel/URL i stedet).
    if card.kind == "browser" {
        return Err(format!("card is a browser: {name}"));
    }
    transcripts::read_transcript_tail_for_profile(&card.profile, &card.cwd, max_entries)
}

#[tauri::command]
fn reset_voice_capture() -> Result<String, String> {
    voice_capture::reset_capture()
}

#[tauri::command]
fn append_voice_capture(entry: serde_json::Value) -> Result<String, String> {
    voice_capture::append_capture(&entry)
}

#[tauri::command]
fn read_usage_snapshot() -> Option<usage_hud::UsageSnapshot> {
    usage_hud::read_snapshot()
}

#[tauri::command]
fn read_context_snapshots() -> Vec<context_hud::ContextSnapshot> {
    context_hud::read_snapshots()
}

/// Voice-wake-hotkey (fix "foerste tryk er doedt", 2026-07-19): frontenden
/// konfigurerer den Rust-side niveau-poller med sin accelerator (samme
/// grammatik som ptt.ts). Polleren emitter "wake-hotkey"-eventet og er den
/// PRIMAERE wake-kilde; DOM-handleren i ptt.ts bestaar som suppression +
/// fallback. Se wake_hotkey.rs' modul-doc for hele rationalet.
#[tauri::command]
fn configure_wake_hotkey(accel: String) -> Result<(), String> {
    wake_hotkey::set_accelerator(&accel)
}

#[tauri::command]
fn suspend_wake_hotkey(suspended: bool) -> Result<(), String> {
    wake_hotkey::set_suspended(suspended);
    Ok(())
}

#[tauri::command]
fn get_project() -> Result<project::ProjectMeta, String> {
    match project::read_project_meta(&talminal_base())? {
        Some(meta) => Ok(meta),
        // Missing project.json is only lawful for default-/legacy-homes.
        None => Ok(project::ProjectMeta {
            root: instance::user_home_dir()?,
            name: "default".into(),
            added_at: None,
        }),
    }
}

// --- Chat-kortets tre kommandoer (spec §6) -------------------------------
// Hver er ÉN delegering: beslutningerne bor i threads-lib'en, ikke her.

#[tauri::command]
fn chat_thread_read(thread: String, from_seq: u64) -> Result<threads::ThreadView, String> {
    threads::view(&thread, from_seq)
}

/// Ejerens indskudte besked. `from_kind: Human` saettes af BACKENDEN — fladen
/// kan ikke vaelge sin egen art (spec §3.1).
#[tauri::command]
fn chat_thread_post(thread: String, text: String) -> Result<(), String> {
    threads::post(threads::PostRequest {
        thread,
        from_card: threads::OWNER.to_string(),
        from_kind: threads::FromKind::Human,
        intent: threads::Intent::Sparring,
        text,
    })
    .map(|_| ())
}

/// "Stop samarbejde" (spec §5.4). Idempotent: et andet klik er en no-op.
#[tauri::command]
fn chat_thread_stop(thread: String) -> Result<(), String> {
    threads::close_thread(&thread, threads::TerminalReason::OwnerStopped, None);
    Ok(())
}

// --- Traad-seams: rene adaptere, ingen beslutninger ----------------------

/// Kortet regnes som midt i en tur hvis det har produceret output for under
/// dette stykke tid siden. Adapter-tuning, ikke et spec-tal.
const BUSY_QUIET_MS: u64 = 1_500;
/// "Modparten arbejdede faktisk" (A9's fjerde udfaldsklasse). Bevidst laengere
/// end BUSY_QUIET_MS: en agent der taenker mellem tool-kald er stadig aktiv.
const PEER_ACTIVE_WINDOW_MS: u64 = 60_000;

struct SystemClock;
impl threads::dispatch::Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// Ren maaling, ingen beslutning: har kortets PTY produceret output inden for
/// `window_ms`? `PtyHost::output_sequence()` er monoton, saa en aendring siden
/// sidste sample ER aktivitet. Et kort vi aldrig har set foer regnes som aktivt
/// i ét vindue — den konservative retning for begge kaldere.
fn output_activity(card: &str, window_ms: u64) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<HashMap<String, (u64, u64)>>> = OnceLock::new();
    let now = threads::dispatch::Clock::now_ms(&SystemClock);
    let sequence = registry::card_handle(card)
        .ok()
        .and_then(|handle| {
            let guard = handle.lock().unwrap_or_else(|p| p.into_inner());
            guard
                .terminal()
                .and_then(|t| t.pty.as_ref().map(|pty| pty.output_sequence()))
        })
        .unwrap_or(0);
    let mut seen = SEEN
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let entry = seen.entry(card.to_string()).or_insert((sequence, now));
    if entry.0 != sequence {
        *entry = (sequence, now);
        return true;
    }
    now.saturating_sub(entry.1) < window_ms
}

/// Leveringskanalen. `write_notice` gaar gennem submit-koreografien med
/// kilde-labelen `agent`, saa control-laget aldrig ser en agent-wake som
/// menneske-input (spec §3.4).
struct PtyNotifier;
impl threads::dispatch::Notifier for PtyNotifier {
    fn is_busy(&self, card: &str) -> bool {
        output_activity(card, BUSY_QUIET_MS)
    }
    fn write_notice(&self, card: &str, text: &str) -> Result<(), String> {
        submit::submit_prompt_as(
            card.to_string(),
            text.to_string(),
            submit::WriteSource::Agent,
        )
    }
}

/// Kompensationen for fund M1: `create_card` + `spawn_into` er ÉN transaktion.
/// Lykkes oprettelsen mens spawnet fejler — fx naar agentens CLI hverken er paa
/// PATH eller har et `exe_fallback` (claude har ingen, profiles.rs) — bliver et
/// tomt terminal-kort med `pty: None` liggende paa canvaset for evigt: parringen
/// returnerer fejlen, og ingen rydder op efter den. Udtrukket som ren funktion
/// saa kompensationen kan unit-testes uden en AppHandle (samme moenster som
/// `spawn_failed_payload`).
fn spawn_or_close(
    name: String,
    spawn: impl FnOnce(&str) -> Result<(), String>,
    close: impl FnOnce(&str),
) -> Result<String, String> {
    if let Err(e) = spawn(&name) {
        close(&name);
        return Err(e);
    }
    Ok(name)
}

/// `spawn_into` bor i binary-craten — derfor denne adapter til `card_pair`.
struct CardSpawner {
    app: AppHandle,
}

impl threads::pair::Spawner for CardSpawner {
    fn spawn(&self, agent: &str, cwd: &str) -> Result<threads::pair::SpawnedCard, String> {
        // Profil-spawn med arvet cwd og INGEN custom command: kun profil-
        // spawnede kort faar MCP-injektion, og uden den er partneren ubrugelig.
        let info = registry::create_card(cwd.to_string(), agent.to_string(), None)?;
        // M1: fejler spawnet, lukkes kortet FOER fejlen returneres — samme
        // registry-vej som trait'ens egen `close` nedenfor.
        let name = spawn_or_close(
            info.name,
            |name: &str| spawn_into(&self.app, name, false),
            |name: &str| {
                let _ = registry::close_card(name.to_string());
            },
        )?;
        Ok(threads::pair::SpawnedCard {
            name,
            agent: agent.to_string(),
        })
    }

    /// En degraderet agent uden Talminal-tools kan ikke kalde `card_inbox` og
    /// er derfor ikke en partner. Tre uafhaengige beviser kraeves.
    fn verify_running_with_mcp(&self, card: &str) -> Result<(), String> {
        let handle = registry::card_handle(card)?;
        let guard = handle.lock().unwrap_or_else(|p| p.into_inner());
        let term = guard
            .terminal()
            .ok_or_else(|| format!("card is not a terminal: {card}"))?;
        if term.pty.is_none() {
            return Err(format!("card is not running: {card}"));
        }
        if term.custom_command {
            return Err(format!("custom-command card has no talminal tools: {card}"));
        }
        // Beslutning 12: injektionen er beviselig ved at kortet HAR et token.
        if !mcp::has_card_token(card) {
            return Err(format!("mcp capability was not injected for {card}"));
        }
        Ok(())
    }

    fn close(&self, card: &str) {
        let _ = registry::close_card(card.to_string());
    }
}

/// Hjerteslagets event til chat-kortet.
#[derive(Clone, serde::Serialize)]
struct ChatThreadUpdated {
    thread: String,
}

/// Teksten til M11's Failed-gren. Ren funktion, saa indholdet kan unit-testes
/// uden en dialog — og EN tekst, saa konsollen og dialogen aldrig kan komme til
/// at sige noget forskelligt. Alle tre led skal med: slug'en (hvilket
/// workspace), laasefilens sti (hvor operatoeren skal kigge) og OS-fejlen
/// (hvorfor). Stavemaaden er bevidst ASCII: teksten gaar ogsaa til stderr, og en
/// konsol med en legacy-kodeside mangler UTF-8-danske tegn.
fn startup_lock_failure_message(slug: &str, lock_path: &std::path::Path, error: &str) -> String {
    format!(
        "Talminal kunne ikke afgoere instans-laasen for '{slug}'.\n\n\
         Laasefil: {}\n\
         Aarsag: {error}\n\n\
         Appen starter ikke, fordi den ikke kan vide om en anden instans allerede koerer.",
        lock_path.display()
    )
}

/// Sidste udvej naar opstarten fejler FOER der findes et vindue at melde i.
///
/// M11's Failed-gren var usynlig i praksis: release-builden bygges med
/// `windows_subsystem = "windows"` (linje 20) og har ingen konsol, saa
/// `eprintln!` skriver i ingenting. Og exit-koden ser heller ingen — startes
/// appen fra `talminal`-launcheren, sker det via `workspaces::spawn_workspace`
/// med DETACHED_PROCESS og null-stdio, som baade kapper stderr og enhver
/// venten paa koden. Resultatet var praecis den tavse "appen starter bare
/// ikke", M11 blev rejst for at fjerne.
///
/// Valget af kanal: en MessageBox, ikke en logfil ved siden af `.lock`.
/// Fejlklassen ER "state-dir'en kan ikke aabnes/skrives" (ACCESS_DENIED, fuld
/// disk, read-only mappe), saa en logfil i praecis den mappe er den kanal der er
/// mest tilboejelig til ogsaa at fejle — og den kraever at brugeren paa forhaand
/// ved hvor den ligger. Dialogen kraever ingenting og kan copy-pastes ind i en
/// fejlrapport. `eprintln!` beholdes ved siden af, saa en debug-build startet
/// direkte fra en terminal (den ene vej hvor stderr overlever) stadig faar
/// teksten uden et klik.
fn show_startup_error(message: &str) {
    use talminal_canvas_lib::instance::to_wide;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND,
    };
    let text = to_wide(message);
    let caption = to_wide("Talminal");
    // Ingen ejer-HWND: der ER intet vindue endnu. MB_SETFOREGROUND, fordi
    // launcheren netop har givet fokus til en anden proces.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_ICONERROR | MB_OK | MB_SETFOREGROUND,
        );
    }
}

fn main() {
    // B-light T2 startup order (invariant):
    // (1) resolve_startup_home → set TALMINAL_HOME BEFORE anything else
    // (2) acquire_instance_lock; AlreadyRunning → focus_existing_instance +
    //     exit 0, Failed → fejldialog + exit 1 (M11: de to maa ALDRIG slaas
    //     sammen — en I/O-fejl der afslutter med 0 er en usynlig opstartsfejl.
    //     Dialogen, ikke exit-koden, ER meldingen: release har ingen konsol)
    // (3) lock leaked for process lifetime
    // (4) setup: title + instance.json (NO last_project — see the setup hook)
    // (5) last_project is written only once a workspace actually becomes
    //     visible, by two writers: the poller's Action::Reveal arm
    //     (workspaces/mod.rs) and Focused(true) — both via
    //     write_last_project_for_active
    // T1: `test-seams` bytter secrets-backenden ud med en in-memory HashMap.
    // Det er rigtigt under `cargo test` — og en tavs datatabs-faelde hvis
    // nogen starter en binaer der er bygget med featuren (fx den
    // `target/debug/talminal-canvas.exe` som `cargo test` selv efterlader).
    // Appen ville se helt normal ud, brugeren ville indtaste sin API-noegle,
    // og den ville vaere vaek ved exit. `compile_error!` duer ikke, fordi
    // `cargo test` netop SKAL kunne bygge bin-targetet; saa kanalen er en
    // dialog, af samme grund som M11 nedenfor: release har ingen konsol, og
    // en advarsel man ikke ser, er ingen advarsel.
    #[cfg(feature = "test-seams")]
    {
        let warning = "Talminal TEST-BUILD (test-seams)\n\n\
             Denne binaer gemmer API-noegler i hukommelsen, ikke i Windows \
             Credential Manager. Alt hvad du indtaster forsvinder naar \
             appen lukkes.\n\n\
             Byg uden `--features test-seams` for en rigtig app.";
        eprintln!("{warning}");
        show_startup_error(warning);
    }

    let startup_home = match instance::resolve_startup_home() {
        Ok(h) => h,
        Err(e) => {
            // Samme fejlklasse som `AcquireOutcome::Failed` nedenfor, og derfor
            // samme kanal. Grenen her skrev indtil OSS-fase 2 KUN `eprintln!`
            // og kaldte `exit(1)` — i et release-byg uden konsol betoed det at
            // appen forsvandt uden et ord, mens den anden gren tre snese linjer
            // nede allerede brugte dialogen af praecis den grund. Det var ikke
            // en afvejning; den ene gren blev bare ikke opdateret.
            // `show_startup_error` er en ren Win32 `MessageBoxW` uden
            // afhaengighed af Tauri-runtimen og kan derfor kaldes saa tidligt.
            // (Fundet af Codex-fuldscanningen 2026-08-02.)
            let message = format!("Talminal startup failed: {e}");
            eprintln!("{message}");
            show_startup_error(&message);
            std::process::exit(1);
        }
    };
    std::env::set_var("TALMINAL_HOME", &startup_home);
    if let Err(error) = secrets::migrate_provider_key_slots() {
        // Best-effort: fejler en kopi, er intet slettet, og naeste opstart
        // proever migrationen igen.
        eprintln!("[canvas] noegle-migration udskudt: {error}");
    }
    // Initialiser den asynkrone trace-writer under proces-start, saa foerste
    // maalte create/close aldrig betaler fil-open eller thread-spawn.
    #[cfg(feature = "perf-trace")]
    let _ = perf_trace::enabled();

    let slug = startup_home
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "default".into());
    let instance_id = format!("{}-{}", std::process::id(), workspaces::now_iso_z());
    // Commands udsteder requests med DENNE identitet; frontenden får aldrig
    // lov at vælge den, så den sættes én gang her.
    workspaces::set_my_instance_id(instance_id.clone());

    let instance_lock = match instance::acquire_instance_lock(&startup_home, &slug) {
        instance::AcquireOutcome::Acquired(lock) => lock,
        // M11: "vi ved det ikke" er IKKE "en anden koerer". En I/O-fejl paa
        // `.lock` (ACCESS_DENIED, fuld disk, read-only mappe) afsluttede foer
        // tavst med exit-kode 0 og saa ud som en helt normal start — den eneste
        // fejlrapport der kunne komme ud af det var "appen starter bare ikke".
        // (Et TREDJEPARTS eksklusive hold paa `.lock` — scanner, editor — giver
        // derimod ERROR_SHARING_VIOLATION og er umuligt at skelne fra en aegte
        // dublet; det lander bevidst i `AlreadyRunning`-grenen nedenfor.)
        // Kanalen SKAL vaere en dialog: i release har processen hverken konsol
        // eller en kalder der ser exit-koden (se `show_startup_error`).
        instance::AcquireOutcome::Failed(e) => {
            let message =
                startup_lock_failure_message(&slug, &startup_home.join(".lock"), &e.to_string());
            eprintln!("{message}");
            show_startup_error(&message);
            std::process::exit(1);
        }
        instance::AcquireOutcome::AlreadyRunning => {
            // Vi er ved at afslutte som DUBLET. Med workspace-protokollen er det
            // ikke længere nok at fokusere: fokuseringen henter et vindue frem
            // som `active_workspace.json` ikke peger paa, og poll-loekken i det
            // faktisk aktive vindue skjuler det igen ved naeste tick — eller
            // viser to vinduer paa én gang. Requesten skal derfor UDSTEDES FOER
            // fokuseringen.
            //
            // Issueren er `dup-<pid>` og ikke `my_instance_id()`: denne proces
            // exit(0)'er om et oejeblik, og et request-id der ligner en levende
            // instans ville forvirre rollback-korrelationen.
            let global_base = project::global_base();
            let issuer = format!("dup-{}", std::process::id());
            let request = workspaces::next_request(
                &global_base,
                &slug,
                &issuer,
                workspaces::deadline_from_now(),
            );
            // Fejlen maa ikke vaere tavs: uden requesten er fokuseringen
            // kortvarig, og brugeren ser vinduet forsvinde igen. Vi fortsaetter
            // alligevel — et blinkende taskbar-ikon er en bedre degradering end
            // ingenting.
            if let Err(e) = workspaces::write_active(&global_base, &request) {
                eprintln!("Talminal: kunne ikke udstede aktiverings-request for '{slug}': {e}");
            }
            let focused = instance::focus_existing_instance(&startup_home);
            if focused {
                eprintln!("Talminal: focused existing window for '{slug}'");
            } else {
                eprintln!("Talminal: instance for '{slug}' already running (could not bring to foreground)");
            }
            // Focus failure must NEVER spawn a duplicate (B4).
            std::process::exit(0);
        }
    };
    let _instance_lock: &'static instance::InstanceLock = Box::leak(Box::new(instance_lock));

    let project_meta = match project::read_project_meta(&startup_home) {
        Ok(Some(meta)) => meta,
        Ok(None) => project::ProjectMeta {
            root: instance::user_home_dir().unwrap_or_else(|_| startup_home.clone()),
            name: "default".into(),
            added_at: None,
        },
        Err(e) => {
            eprintln!("Talminal: corrupt project.json: {e}");
            std::process::exit(1);
        }
    };
    let meta_for_setup = project_meta.clone();
    let home_for_setup = startup_home.clone();
    // `on_window_event`s Focused(true)-gren skriver `last_project` gennem
    // `write_last_project_for_active`, som tager en SLUG — ikke en root. Den
    // gamle `meta_for_focus` (en ProjectMeta) er derfor afloest af slug'en selv:
    // for default-workspacet er `meta.root` brugerens hjemmemappe, og roden ville
    // sende naeste opstart til en hash-baseret, TOM tvillingemappe.
    let slug_for_focus = slug.clone();

    // Base-dir-oploesningen GENBRUGES fra Task 7 (cards::talminal_base /
    // cards::default_cards_path): TALMINAL_HOME er nu sat til projekt-state-dir.
    #[cfg(feature = "supervision")]
    let signals_dir = talminal_base().join("signals");
    let cards_path = default_cards_path();

    // Task 6 (load-rækkefølge): workspace.json er kort-persistensens
    // sandhedskilde. Findes den → indlæs + seed registryet + genindlæs den
    // monotone tæller (cards.toml læses ALDRIG igen for workers). Findes den
    // IKKE → engangsimport af cards.toml's worker-kort via registry::create_card
    // (friske numre, card-{number}-navne, [master] skippes) og workspace.json
    // skrives. SKAL køre FØR nogen anden registry-mutation (tæller-alignmentet
    // forudsætter et frisk registry — se workspace.rs' modul-doc).
    let (cards_missing, cards_error) = match workspace::startup_load() {
        Ok(report) => (report.cards_missing, report.cards_error),
        Err(e) => {
            // Defekt workspace.json: ALDRIG import-fallback (ville genoplive
            // legacy-kort og overskrive brugerens workspace). Persistensen er
            // slået fra i denne session; fejlen når UI'et via get_cards_status.
            eprintln!("workspace.json load failed: {e}");
            (false, Some(e))
        }
    };

    // EJER-AMENDMENT (2026-07-19): restore-on-launch er AFKOBLET — canvas
    // starter altid tomt (startup_load seeder intet), så der findes ingen
    // kort at restore. restore.rs består som parkeret ren logik (samme
    // mønster som supervision — intet slettes).

    // Supervision-sporet (parkeret; Task 3: master ude af MVP-pathen):
    // master-kortet lever UDEN FOR workspace.json og seedes fortsat fra
    // cards.toml's [master] — EFTER workspace-loadet, så tæller-alignmentet
    // (Task 6) ikke forstyrres.
    #[cfg(feature = "supervision")]
    match load_cards(&cards_path) {
        Ok(cards_file) => {
            if let Some(master) = cards_file.master {
                if let Err(e) = registry::seed_card(master, "claude") {
                    eprintln!("cards.toml: master not seeded: {e}");
                }
            }
        }
        Err(CardsError::NotFound(_)) => {}
        Err(e) => eprintln!("cards.toml: master load skipped: {e}"),
    }

    let app = tauri::Builder::default()
        // B-light T2: global-shortcut-plugin fjernet (Rust-side). JS-pakken
        // bliver til T5 — registerPttShortcut fejler blødt via .catch.
        //
        // Projektets FØRSTE plugin-registrering (Task 12). Den native
        // mappevælger bag "+ Tilføj projekt" bruges KUN fra Rust-siden
        // (`workspaces::commands::add_workspace`), som ligger uden for
        // capability-ACL'en — derfor er `capabilities/default.json` urørt.
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            #[cfg(feature = "supervision")]
            signals_dir,
            cards_path,
            cards_missing,
            cards_error,
        })
        .invoke_handler(tauri::generate_handler![
            get_cards,
            get_cards_status,
            get_card_state,
            spawn_card,
            respawn_card,
            write_pty,
            resize_pty,
            kill_card,
            resume_card_control,
            // Task 1 Del B-sockets (doede indtil deres task udfylder dem):
            create_card,
            close_card,
            close_cards,
            list_cards,
            update_card_geometry,
            set_viewport,
            get_workspace,
            set_settings,
            set_card_visible,
            // Browser-kort (plan Task 5):
            create_browser_card,
            navigate_browser_card,
            set_browser_bounds,
            set_browser_occlusion,
            set_browser_fullscreen,
            focus_browser_card,
            store_secret,
            load_secret,
            mint_transcription_secret,
            router_chat_completion,
            perf_trace_frontend,
            tts_speech,
            probe_router_route,
            open_microphone_settings,
            tts_speech_stream,
            warm_voice_connections,
            delete_secret,
            submit_prompt,
            read_transcript_tail,
            reset_voice_capture,
            append_voice_capture,
            read_usage_snapshot,
            read_context_snapshots,
            configure_wake_hotkey,
            suspend_wake_hotkey,
            get_project,
            // Chat-kort (agent-til-agent, spec §6):
            chat_thread_read,
            chat_thread_post,
            chat_thread_stop,
            // Workspace-rail'en (plan Task 8) — tynde wrappers i lib'en:
            workspaces::commands::list_workspaces,
            workspaces::commands::activate_workspace,
            workspaces::commands::set_workspace_hidden,
            // "+ Tilføj projekt" (plan Task 12) — async, fordi mappevælgeren
            // blokerer sin egen tråd:
            workspaces::commands::add_workspace,
            // Lukkeprotokollen (plan Task 11) — funnelen bor i on_window_event,
            // tilstandsmaskinen i workspaces::close:
            workspaces::commands::request_close_workspace,
            workspaces::commands::confirm_close
        ])
        .setup(move |app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title(&meta_for_setup.name);
                // Mikrofonen skal ikke kunne afvises permanent ved et uheld.
                // Fejler registreringen, falder WebView2 tilbage til sin egen
                // prompt — altsaa dagens adfaerd — saa den maa ikke vaelte
                // opstarten af en app der ogsaa kan bruges uden stemme.
                if let Err(e) = webview_permissions::grant_microphone(&window) {
                    eprintln!("[canvas] mikrofon-permission blev ikke registreret: {e}");
                }
                let hwnd = window.hwnd().map(|h| h.0 as isize).unwrap_or(0);
                if let Err(e) =
                    instance::write_instance_info(&home_for_setup, std::process::id(), hwnd)
                {
                    eprintln!("[canvas] write_instance_info failed: {e}");
                }
                let geometry = Arc::new(workspaces::geometry::GeometryWriter::new(
                    project::global_base(),
                ));
                app.manage(Arc::clone(&geometry));
                let status_writer = Arc::new(workspaces::status::StatusWriter::new(
                    project::global_base(),
                    slug.clone(),
                    instance_id.clone(),
                ));
                app.manage(Arc::clone(&status_writer));
                // Gør writeren naaelig for de veje der ikke har en AppHandle:
                // workspace.rs' persist-hooks, exit-watcheren og kill-vejen
                // (kendelse CORRECTIONS.md C-T6). Skal ske FOER det foerste
                // refresh — ellers er kaldet en no-op.
                workspaces::status::install(Arc::clone(&status_writer));
                // FRISK TAVLE FØR POLLEREN — begge filer er efterladenskaber
                // fra den FORRIGE proces for dette slug, og begge kan lyve om
                // NUVAERENDE tilstand:
                //
                // `status.json` overlever et crash med `visible: true` og sit
                // gamle `acked_request`. Liveness er den navngivne mutex
                // (`instance_alive`), og den er sand igen i samme oejeblik jeg
                // starter — laenge foer jeg selv har skrevet en linje. I det
                // vindue laeser de andre processer den doede instans' fil som
                // et LEVENDE svar: en kilde kan skjule sig mod en kvittering
                // ingen har givet, `any_visible` kan spaerre for at nogen tager
                // over, og forgrundsretten kan blive overdraget til en doed pid.
                // Spec §3.3 vil have `instance_id` med i liveness-vurderingen;
                // mutexen baerer ingen identitet, saa i stedet goer vi filen
                // uinteressant: den tilhoerer pr. konstruktion den nulevende
                // instans, fordi den skrives foer nogen kan naa at laese den.
                //
                // `control.json` er en luk-anmodning stillet til den FORRIGE
                // instans. `request_close_workspace` skriver kun til et levende
                // workspace, men processen kan doe i mellemrummet mellem
                // liveness-tjekket og skrivningen — og saa ligger anmodningen og
                // venter som en landmine der lukker vinduet igen ved naeste
                // opstart. En anmodning udstedt foer jeg fandtes, er ikke til
                // mig.
                status_writer.update(|status| {
                    status.acked_request = None;
                    status.visible = false;
                });
                let _ = workspaces::control::take_pending(&project::global_base(), &slug);
                workspaces::status::refresh_card_counts();
                // Lukkeprotokollen (Task 11). Tilstanden manages og
                // control-kanalen kobles FØR polleren starter: pollerens første
                // tick kan finde en ventende anmodning, og den skal have en
                // fase at rykke.
                app.manage(Arc::new(workspaces::close::CloseState::new(
                    project::global_base(),
                    slug.clone(),
                )));
                workspaces::close::install_peer_close(app.handle().clone());
                let badge_writer = Arc::clone(&status_writer);
                let surface: Arc<dyn workspaces::surface::WindowSurface> = Arc::new(
                    workspaces::surface::TauriSurface::new(window.clone(), geometry),
                );
                let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let global_base = project::global_base();
                let (me, inst) = (slug.clone(), instance_id.clone());
                let handle = {
                    let stop = Arc::clone(&stop);
                    std::thread::spawn(move || {
                        workspaces::run_poller(global_base, me, inst, status_writer, surface, stop)
                    })
                };
                // Badge-tick'et (1 s): egen tråd, samme stop-flag som polleren,
                // så app-luk stopper begge. Emitten er den ENESTE ting main.rs
                // bidrager med — diffen og løkken bor i lib'en.
                {
                    let stop = Arc::clone(&stop);
                    let base = project::global_base();
                    let emitter = app.handle().clone();
                    std::thread::spawn(move || {
                        workspaces::run_badge_tick(base, badge_writer, stop, move |list| {
                            let _ = emitter.emit("workspaces-changed", list);
                        })
                    });
                }
                app.manage(workspaces::Poller { stop, handle });
                // BEVIDST: her skrives INTET `last_project`-hint. Setup kører ved
                // boot — før processen har været synlig — og et baggrundsspawn
                // der aldrig når frem må ikke bestemme næste opstart. Hintet
                // skrives derfor først når workspacet FAKTISK bliver synligt:
                // pollerens `Action::Reveal`-arm (workspaces/mod.rs) og
                // `Focused(true)` nedenfor. Bootstrap er dækket, fordi
                // `Action::TakeOver` skriver en frisk request, hvorefter næste
                // tick giver `Reveal` for én selv.

                // Wake-hotkey-polleren maaler fokus-gaten niveau-baseret mod
                // dette HWND (GetForegroundWindow pr. poll) — aldrig via
                // Focused-events (fix "4-5 doede tryk", 2026-07-20; se
                // wake_hotkey.rs' modul-doc).
                wake_hotkey::set_window_hwnd(hwnd);
            }
            wake_hotkey::spawn_wake_poller(app.handle().clone());

            // Supervision-only: presence-polleren startes ALDRIG i default-state
            // (frossen kontrakt — ingen presence-events, ingen poll-tråd).
            #[cfg(feature = "supervision")]
            presence::spawn_presence_poller(app.handle().clone(), presence::presence_dir());

            // Browser-kort (plan Task 5): (1) sweep gamle browser-profiler FOER
            // noget scope oprettes (spec §5/§9); (2) start MCP-serveren med den
            // registry-/webview-backede HostOps (Task 4's flade); (3) start
            // reconciliation-/liveness-polleren (den ENESTE doeds-detektor, S7).
            browser::sweep_profiles();
            // Gate 8: samme kontrakt som profil-sweepet. Daekker de
            // afslutningsveje et haardt exit eller et crash aldrig koerte.
            worker_mcp::sweep_configs();
            let host_ops = Arc::new(browser_host::HostOps {
                app: app.handle().clone(),
            });
            match mcp::start_mcp_server(mcp::McpOps {
                browser: host_ops,
                threads: Arc::new(threads::ops::LiveThreadOps),
            }) {
                Ok(port) => eprintln!("[canvas] browser-cards MCP server on 127.0.0.1:{port}"),
                Err(e) => eprintln!("[canvas] browser-cards MCP server failed to start: {e}"),
            }
            browser_host::spawn_reconciliation_poller(app.handle().clone());

            // --- Traade (agent-til-agent). Raekkefoelgen er bindende. -------
            // Politikporten FOERST: uden den falder hver agent-besked tilbage
            // paa human-only, og featuren er tavst doed (spec §4.3).
            threads::policy::set_port(Arc::new(registry::RegistryPolicyPort));

            // Traad-taelleren lever i hukommelsen og starter forfra ved hver
            // app-start; traad-ARKIVET paa disken goer ikke. Uden en seeding
            // ville foerste parring efter en genstart genbruge et id der
            // allerede staar i arkivet (M7). Kaldet ligger FOER
            // `set_spawner` — det er den foerste vej der kan allokere et id.
            threads::seed_counter_from_disk();

            // Arkivet er ARKIV: en traad der stod awaiting da appen doede faar
            // restart_abort som sidste linje (spec §5.5).
            match threads::archive::terminalize_awaiting_on_startup() {
                Ok(0) => {}
                Ok(n) => {
                    eprintln!("threads: terminalized {n} awaiting thread(s) from a previous run")
                }
                Err(e) => eprintln!("threads: startup terminalization failed: {e}"),
            }

            // Panicer hvis trappen er skaev ELLER porten mangler. Begge ville
            // ellers vaere tavse driftsfejl.
            threads::sweep::assert_startup_invariants();

            threads::dispatch::set_seams(Arc::new(SystemClock), Arc::new(PtyNotifier));
            threads::set_activity_probe(Box::new(|card| {
                output_activity(card, PEER_ACTIVE_WINDOW_MS)
            }));
            threads::pair::set_spawner(Arc::new(CardSpawner {
                app: app.handle().clone(),
            }));
            // Kort som en AGENT skaber (card_pair's partner og chat-kort) skal
            // ind i fladens kortliste. Browser-kort har browser-card-updated;
            // det her er den tilsvarende vej. Uden den ligger kortene usynlige,
            // fordi fladen ikke poller (dogfood-fund 2026-07-25).
            let cards_app = app.handle().clone();
            registry::set_cards_changed(Box::new(move || {
                let _ = cards_app.emit("cards-changed", ());
            }));

            // Hjerteslaget. 250 ms: hurtigt nok at en notits foeles
            // oejeblikkelig, langsomt nok at scanningen over nogle titaller
            // traade er gratis. Adapter-tuning; raekkefoelgen og sweep-
            // forholdet bor i heartbeat::beat.
            let beat_app = app.handle().clone();
            thread::spawn(move || {
                let mut tracker = threads::heartbeat::ChangeTracker::default();
                loop {
                    thread::sleep(Duration::from_millis(250));
                    for thread in threads::heartbeat::beat(&mut tracker).changed {
                        let _ = beat_app.emit("chat-thread-updated", ChatThreadUpdated { thread });
                    }
                }
            });

            // Restore-autospawn fjernet (ejer-amendment 2026-07-19: tom
            // canvas hver gang — der er intet at genoptage ved launch).
            Ok(())
        })
        .on_window_event(move |window, event| {
            if matches!(
                event,
                tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)
            ) {
                if let Some(geometry) = window
                    .app_handle()
                    .try_state::<Arc<workspaces::geometry::GeometryWriter>>()
                {
                    let maximized = window.is_maximized().unwrap_or(false);
                    // OUTER position + INNER size. Blandingen er ikke en fejl —
                    // den spejler de to kald `apply_placement` bruger, saa det vi
                    // gemmer er praecis det vi kan saette igen. Se `Rect`s doc.
                    if let (Ok(position), Ok(size)) = (window.outer_position(), window.inner_size())
                    {
                        let previous = workspaces::geometry::read(&project::global_base());
                        // None = "intet at gemme" (maksimeret uden en tidligere
                        // placering); at gemme fuldskaerms-rect'en som
                        // normal-rect ville braekke brugerens "gendan".
                        if let Some(placement) = workspaces::geometry::placement_from(
                            maximized,
                            (position.x, position.y),
                            (size.width as i32, size.height as i32),
                            previous,
                        ) {
                            geometry.schedule(placement);
                        }
                    }
                }
            }
            // OBS: wake-hotkey-polleren maa ALDRIG gates fra Focused-events —
            // WebView2-fokusevents kan udeblive/omrokeres og efterlod gaten
            // stale-false ("4-5 doede tryk", fikset 2026-07-20). Polleren
            // maaler selv forgrundsvinduet pr. poll.
            if let tauri::WindowEvent::Focused(true) = event {
                // Den ANDEN af hintets to skrivere (den første er pollerens
                // `Action::Reveal`-arm i workspaces/mod.rs). Vejen går gennem den
                // SLUG-baserede `write_last_project_for_active`, som er no-op for
                // "default", og det er ikke pedanteri: for default-workspacet var
                // `meta_for_focus.root` brugerens hjemmemappe, så en root-baseret
                // skrivning ville ved allerførste fokus lægge præcis det hint hele
                // kontrakten er bygget for at undgå.
                if let Err(e) =
                    project::write_last_project_for_active(&project::global_base(), &slug_for_focus)
                {
                    eprintln!(
                        "[canvas] last_project-hint (fokus) for {slug_for_focus} fejlede: {e}"
                    );
                }
            }

            // FUNNELEN (Task 11 + global exit). Direkte vinduesluk —
            // titelbjaelkens ✕, Alt+F4 og taskbaren — spoerger nu ALTID om hele
            // Talminal skal afsluttes. Rail/control-vejen saetter derimod fasen
            // til HandingOff foerst; dens interne `window.close()` passerer kun,
            // naar netop det workspace er Approved.
            //
            // Den maa IKKE blokere: handleren koerer paa event-loopet, og en
            // synkron venten paa en successors kvittering ville fryse hele
            // appen inklusive den overdragelse der ventes paa. Derfor kun to
            // ting her: spoerg maskinen, udfoer svaret. Alt der tager tid, sker
            // i `spawn_handoff_then_close`s egen traad.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Kun hovedvinduet har en lukkeprotokol.
                if window.label() == "main" {
                    let handle = window.app_handle().clone();
                    if let Some(state) = handle.try_state::<Arc<workspaces::close::CloseState>>() {
                        // Advarslen er app-global, saa payloaden laeses frisk
                        // over ALLE levende processers statusfiler. Frontendens
                        // workspace-liste kan vaere op til ét sekund gammel.
                        let summary = workspaces::close::quit_summary(&project::global_base());
                        match state.on_close_requested(summary.running_cards) {
                            workspaces::close::CloseAction::Allow => {}
                            workspaces::close::CloseAction::Prevent => api.prevent_close(),
                            workspaces::close::CloseAction::PreventAndConfirm { .. } => {
                                api.prevent_close();
                                if let Err(e) = window.emit("close-confirm-requested", summary) {
                                    // Kan spoergsmaalet ikke stilles, maa vi
                                    // IKKE godkende en lokal lukning som den
                                    // gamle workspace-flow gjorde. Rul tilbage,
                                    // saa et nyt tryk kan proeve igen.
                                    eprintln!("[canvas] close-confirm-requested fejlede: {e}");
                                    let _ = state.on_confirm(false);
                                }
                            }
                            // Kun `confirm_close(true)` kan producere denne.
                            workspaces::close::CloseAction::QuitAll => api.prevent_close(),
                        }
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    // Fix F4: normal vinduesluk SKAL gå gennem den NORMATIVE teardown
    // (Global Constraint: kill child → drop writer → dræn → drop master →
    // join reader) — uden denne handler exiter Tauri uden at destruere
    // PtyHosts, og pty-børnene efterlades til OS'ets forgodtbefindende.
    app.run(|app_handle, event| {
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            use std::sync::atomic::{AtomicU8, Ordering};
            const IDLE: u8 = 0;
            const TEARING_DOWN: u8 = 1;
            const READY_TO_EXIT: u8 = 2;
            // Tre tilstande, ikke en bool: ethvert ekstra exit-signal MENS
            // PTY-teardown kører skal fortsat standses. Kun trådens eget sidste
            // `app.exit()` efter alle joins må passere.
            static TEARDOWN_PHASE: AtomicU8 = AtomicU8::new(IDLE);
            match TEARDOWN_PHASE.compare_exchange(
                IDLE,
                TEARING_DOWN,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {}
                Err(TEARING_DOWN) => {
                    api.prevent_exit();
                    return;
                }
                Err(READY_TO_EXIT) => return,
                Err(_) => {
                    api.prevent_exit();
                    return;
                }
            }
            if let Some(poller) = app_handle.try_state::<workspaces::Poller>() {
                poller
                    .inner()
                    .stop
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            api.prevent_exit();
            // Task 6: flush en evt. udestående debounced workspace-persist
            // FØR teardown — geometri/viewport-ændringer fra de sidste
            // øjeblikke må ikke tabes ved app-luk.
            if let Err(e) = workspace::persist_now() {
                eprintln!("[canvas] workspace persist at exit failed: {e}");
            }
            // ...og en evt. armeret VINDUES-geometri. `persist_now` ovenfor
            // rører kun workspace.json (kort-layoutet paa canvas) — geometrien
            // bor i window_geometry.json og har sin egen debounce, hvis
            // baggrundstraad ellers doer med processen inden for DEBOUNCE_MS.
            if let Some(geometry) =
                app_handle.try_state::<Arc<workspaces::geometry::GeometryWriter>>()
            {
                if let Err(e) = geometry.flush() {
                    eprintln!("[canvas] window geometry flush at exit failed: {e}");
                }
            }
            // Dræn ALLE hosts fra registryet — kun take, intet blokerende
            // arbejde under lås (F2-disciplinen; all_handles slipper
            // map-låsen før kort-låsene tages).
            let hosts: Vec<Arc<PtyHost>> = registry::all_handles()
                .into_iter()
                .filter_map(|h| {
                    h.lock().ok().and_then(|mut card| {
                        let terminal = card.terminal_mut()?;
                        if let Some(readiness) = terminal.submit_readiness.take() {
                            readiness.cancel();
                        }
                        terminal.pty.take()
                    })
                })
                .collect();
            let exit_code = code.unwrap_or(0);
            let handle = app_handle.clone();
            // Dedikeret tråd (aldrig UI-tråden): normativ teardown pr. host
            // parallelt (op til ~15 s pr. kort ellers serielt), join alle,
            // og kald FØRST derefter app.exit — så er alt revet ned, når
            // processen dør.
            std::thread::spawn(move || {
                let mut teardown_threads = Vec::new();
                for host in hosts {
                    teardown_threads.push(std::thread::spawn(move || {
                        let _ = host.kill_and_teardown();
                    }));
                }
                for t in teardown_threads {
                    let _ = t.join();
                }
                TEARDOWN_PHASE.store(READY_TO_EXIT, Ordering::SeqCst);
                handle.exit(exit_code);
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        resolve_command_program, spawn_failed_payload, spawn_or_close, startup_lock_failure_message,
    };

    // PATH er PROCES-global, og cargo koerer unit-tests multi-traadet i én
    // binary. En test der peger PATH et andet sted skal derfor have laasen i
    // haanden — samme moenster som `profiles.rs`' ENV_LOCK.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    // T3: unit-daekning af selve exe-oploesnings-transformationen, isoleret
    // fra laase/PTY-spawn (K1-VAGT, GPT-review-fund B1).

    #[test]
    fn resolve_command_program_leaves_foreign_feed_command_untouched() {
        // Supervision-masterens facon: claude-profil + "uv"-feed-kommando.
        // "uv" matcher ikke claude-stemmet => uroert, uanset custom_command.
        let mut command = vec!["uv".to_string(), "run".to_string(), "feed".to_string()];
        resolve_command_program("claude", false, &mut command).expect("no resolution attempted");
        assert_eq!(
            command[0], "uv",
            "fremmed feed-kommando maa aldrig omskrives"
        );
    }

    /// PATH-hit-vejen returnerer inputtet uaendret (spike §5.6).
    ///
    /// Testen forudsatte indtil OSS-fase 2 at `claude.exe` laa paa maskinens
    /// PATH — samme ejer-maskine-antagelse som
    /// `profiles::tests::resolve_spawn_program_returns_input_on_path_hit...`,
    /// og kommentaren pegede endda paa den som praecedens. CI fandt begge to
    /// paa foerste koersel: groen hos den ene der havde Claude Code
    /// installeret, roed for enhver anden. Her SKAL profilen vaere den rigtige
    /// "claude" (det er profil-opslaget der testes), saa i stedet peges PATH
    /// paa en temp-mappe med en tom `claude.exe`. `resolve_spawn_program`
    /// spoerger kun `is_file()`.
    #[test]
    fn resolve_command_program_keeps_path_hit_bit_identical() {
        let _g = env_lock();
        let dir = tempfile::tempdir().expect("temp path-mappe");
        std::fs::write(dir.path().join("claude.exe"), b"").expect("laeg claude.exe paa PATH");

        let foer = std::env::var_os("PATH");
        std::env::set_var("PATH", dir.path());
        let mut command = vec!["claude".to_string()];
        let udfald = resolve_command_program("claude", false, &mut command);
        match foer {
            Some(p) => std::env::set_var("PATH", p),
            None => std::env::remove_var("PATH"),
        }

        udfald.expect("PATH-hit resolution");
        assert_eq!(
            command[0], "claude",
            "PATH-hit skal vaere bit-identisk med input"
        );
    }

    #[test]
    fn resolve_command_program_skips_custom_commands() {
        let mut command = vec!["uv".to_string(), "run".to_string(), "feed".to_string()];
        resolve_command_program("claude", true, &mut command).expect("custom command is a no-op");
        assert_eq!(command[0], "uv");
    }

    // T9 (N4): payload-builderen for "card-spawn-failed" testes isoleret fra
    // AppHandle'en — den er en ren transformation (name/number/error -> json).
    #[test]
    fn spawn_failed_payload_carries_name_number_and_error_verbatim() {
        let payload = spawn_failed_payload(
            "card-3",
            3,
            "codex.exe blev ikke fundet paa PATH (er codex installeret?)",
        );
        assert_eq!(payload["name"], "card-3");
        assert_eq!(payload["number"], 3);
        assert_eq!(
            payload["error"], "codex.exe blev ikke fundet paa PATH (er codex installeret?)",
            "Rust-fejlteksten fra resolve_spawn_program ER brugerteksten — uaendret igennem",
        );
    }

    // M1: kompensationen i card_pair's spawn-vej. Testes paa den udtrukne
    // funktion, fordi den rigtige vej kraever en AppHandle og et PTY-spawn.

    #[test]
    fn spawn_or_close_lukker_det_foraeldreloese_kort_naar_spawnet_fejler() {
        let closed = std::cell::RefCell::new(Vec::<String>::new());
        let err = spawn_or_close(
            "card-7".to_string(),
            |_: &str| Err("codex.exe blev ikke fundet paa PATH".to_string()),
            |name: &str| closed.borrow_mut().push(name.to_string()),
        )
        .expect_err("spawn-fejlen skal propagere uaendret");

        assert_eq!(err, "codex.exe blev ikke fundet paa PATH");
        assert_eq!(
            closed.into_inner(),
            vec!["card-7".to_string()],
            "et oprettet kort uden PTY maa ikke blive liggende paa canvaset"
        );
    }

    #[test]
    fn spawn_or_close_lukker_ikke_ved_succes() {
        let closed = std::cell::RefCell::new(Vec::<String>::new());
        let name = spawn_or_close(
            "card-7".to_string(),
            |_: &str| Ok(()),
            |name: &str| closed.borrow_mut().push(name.to_string()),
        )
        .expect("succes-vejen returnerer navnet");

        assert_eq!(name, "card-7");
        assert!(
            closed.into_inner().is_empty(),
            "et levende kort maa aldrig lukkes af kompensationen"
        );
    }

    // M11's Failed-gren: dialogen kan ikke unit-testes, men dens TEKST kan.
    // Kontrakten er at operatoeren kan handle paa beskeden alene — uden konsol,
    // uden exit-kode, uden at vide hvor state-dir'en ligger.
    #[test]
    fn startup_lock_failure_message_baerer_slug_sti_og_aarsag() {
        let message = startup_lock_failure_message(
            "demo",
            std::path::Path::new(r"C:\Users\x\AppData\Local\Talminal\projects\demo\.lock"),
            "Access is denied. (os error 5)",
        );
        // Citationstegnene, ikke bare navnet: stien indeholder ogsaa slug'en,
        // saa et raat `contains` ville bestaa uden at slug'en var naevnt.
        assert!(message.contains("'demo'"), "slug'en mangler: {message}");
        assert!(
            message.contains(r"projects\demo\.lock"),
            "laasefilens sti mangler: {message}"
        );
        assert!(
            message.contains("Access is denied. (os error 5)"),
            "OS-aarsagen mangler: {message}"
        );
        // Skabelonen selv (indsaettelserne er ASCII i denne test) maa ikke
        // indfoere danske tegn: teksten gaar ogsaa til stderr, hvor en konsol
        // paa en legacy-kodeside ville mangle dem.
        assert!(message.is_ascii(), "skabelonen er ikke ASCII: {message}");
    }
}
