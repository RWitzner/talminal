//! Workspace-koordinering på tværs af processer. Se
//! docs/superpowers/specs/2026-07-26-workspace-sidebar-design.md rev 1.1.

pub mod attention;
pub mod close;
pub mod commands;
pub mod control;
pub mod geometry;
pub mod listing;
pub mod protocol;
pub mod status;
pub mod surface;
pub mod wake;

use crate::atomic;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// Sammensat request-identitet. `seq` alene er IKKE sikker: udstedelsen er
/// læs-plus-én uden fælles lås, så to processer kan nå frem til samme tal.
/// `issuer` er udstederens `instance_id` (unikt pr. livsforløb), så parret er
/// entydigt — og rollback sammenligner hele parret, hvilket lukker ABA.
///
/// `#[serde(default)]`: filen skrives til disk, og Global Constraint kræver at
/// persistens-formater er bagudkompatible. Et manglende felt i en fil fra en
/// ældre build skal give defaultværdien, ikke en parse-fejl der (via
/// `read_active`s `.ok()?`) ville se ud som "ingen request findes".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RequestId {
    pub issuer: String,
    pub seq: u64,
}

/// Hvem SKAL være synlig. `#[serde(default)]` af samme grund som `RequestId`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActiveRequest {
    pub id: RequestId,
    pub slug: String,
    /// ISO-Z. Efter dette tidspunkt må kilden tage requesten tilbage.
    pub launch_deadline: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionKind {
    #[default]
    None,
    DoneUnread,
    NeedsYou,
}

impl AttentionKind {
    pub fn is_attention(self) -> bool {
        self != Self::None
    }
}

/// Pr. workspace. `acked_request` + `instance_id` gør kvitteringen korreleret,
/// så et stale `visible: true` fra en crashet proces aldrig accepteres.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceStatus {
    pub instance_id: String,
    pub pid: u32,
    pub acked_request: Option<RequestId>,
    pub visible: bool,
    pub cards: u32,
    pub running_cards: u32,
    pub attention: bool,
    pub attention_kind: AttentionKind,
    pub updated_at: String,
}

impl WorkspaceStatus {
    /// En status skrevet af en ældre build har kun `attention: true`.
    /// Kombinationen kan ikke produceres af den nye skriver og behandles
    /// derfor konservativt som den eskalerede tilstand.
    pub fn effective_attention_kind(&self) -> AttentionKind {
        if self.attention && self.attention_kind == AttentionKind::None {
            AttentionKind::NeedsYou
        } else {
            self.attention_kind
        }
    }
}

impl Default for WorkspaceStatus {
    fn default() -> Self {
        Self {
            instance_id: String::new(),
            pid: 0,
            acked_request: None,
            // Nye vinduer starter USYNLIGE (spec §3.2) — ellers ville en frisk
            // proces' defaultværdi kunne læses som en kvittering.
            visible: false,
            cards: 0,
            running_cards: 0,
            attention: false,
            attention_kind: AttentionKind::None,
            updated_at: String::new(),
        }
    }
}

pub fn active_path(global_base: &Path) -> PathBuf {
    global_base.join("active_workspace.json")
}

pub fn status_path(global_base: &Path, slug: &str) -> PathBuf {
    global_base.join("projects").join(slug).join("status.json")
}

/// Fravær og korruption er begge `None`: en ulæselig fil må ikke fælde pollertråden.
pub fn read_active(global_base: &Path) -> Option<ActiveRequest> {
    let text = fs::read_to_string(active_path(global_base)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_active(global_base: &Path, request: &ActiveRequest) -> Result<(), String> {
    atomic::write_json_pretty(&active_path(global_base), request).map_err(|e| e.to_string())?;
    // Filen er autoritativ; eventet er kun en best-effort fast path. Det skal
    // derfor signaleres EFTER den atomiske skrivning, aldrig før.
    wake::notify_badge_refresh();
    let _ = wake::notify(global_base, &request.slug);
    Ok(())
}

pub fn read_status(global_base: &Path, slug: &str) -> Option<WorkspaceStatus> {
    let text = fs::read_to_string(status_path(global_base, slug)).ok()?;
    serde_json::from_str(&text).ok()
}

pub const POLL_MS: u64 = 200;
pub const LAUNCH_DEADLINE_SECS: i64 = 8;
/// Badge-listens kadence (bindende, plan Global Constraints). Langsommere end
/// synligheds-pollen med vilje: den her rører hele projektlisten på disken.
pub const BADGE_TICK_MS: u64 = 1_000;

/// Denne proces' udsteder-identitet. Sættes én gang i app-opstarten; commands
/// kan ikke tage den som parameter (frontenden må ikke kunne vælge identitet).
static MY_INSTANCE_ID: OnceLock<String> = OnceLock::new();

pub fn set_my_instance_id(id: String) {
    let _ = MY_INSTANCE_ID.set(id);
}

/// Fallback'en er pid-baseret og dermed stadig entydig pr. proces: en TOM
/// issuer ville bryde `next_request`s "parret er entydigt" og genåbne ABA'en.
pub fn my_instance_id() -> &'static str {
    MY_INSTANCE_ID.get_or_init(|| format!("{}-uinitialiseret", std::process::id()))
}

pub fn write_status(
    global_base: &Path,
    slug: &str,
    status: &WorkspaceStatus,
) -> Result<(), String> {
    atomic::write_json_pretty(&status_path(global_base, slug), status).map_err(|e| e.to_string())
}

/// Systemets ene ISO-8601-form: `YYYY-MM-DDTHH:MM:SS.mmmZ`.
///
/// Formatet er en KRYDS-PROCES-kontrakt, ikke kosmetik: `launch_deadline`
/// sammenlignes som **streng** (`decide`s deadline-gren nedenfor), saa enhver
/// skriver skal producere praecis denne bredde. Derfor staar literalen ét sted
/// — et `%.6f` sneget ind i en kopi ville braekke sammenligningen tavst.
fn fmt_iso_z(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub fn now_iso_z() -> String {
    fmt_iso_z(chrono::Utc::now())
}

pub fn deadline_from_now() -> String {
    fmt_iso_z(chrono::Utc::now() + chrono::Duration::seconds(LAUNCH_DEADLINE_SECS))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceState {
    Stopped,
    Starting,
    Running,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkspaceSummary {
    pub slug: String,
    pub name: String,
    pub path_hint: Option<String>,
    /// Fuld rod-sti. Rail'en lover den i `title` på trunkerede navne, så den
    /// skal med på wiren — `path_hint` er kun det korteste unikke suffiks.
    pub root: Option<String>,
    pub state: WorkspaceState,
    pub cards: u32,
    pub running_cards: u32,
    pub attention: bool,
    pub attention_kind: AttentionKind,
    pub is_active: bool,
    pub hidden: bool,
    pub defect: bool,
}

/// Row-state udledes af (liste, request, status, liveness) — der findes ingen
/// selvstændig tilstand at læse. `Starting` og `Failed` kan KUN produceres her,
/// og kræver derfor både requesten og et nu-tidspunkt.
pub fn summaries(
    global_base: &Path,
    active: Option<&ActiveRequest>,
    now: &str,
) -> Vec<WorkspaceSummary> {
    listing::list(global_base)
        .into_iter()
        .map(|e| {
            let status = read_status(global_base, &e.slug);
            let alive = instance_alive(global_base, &e.slug);
            let er_target = active.is_some_and(|a| a.slug == e.slug);
            let ackede_denne = match (
                active,
                status.as_ref().and_then(|s| s.acked_request.as_ref()),
            ) {
                (Some(a), Some(acked)) => acked == &a.id,
                _ => false,
            };

            let state = if alive && ackede_denne {
                WorkspaceState::Running
            } else if er_target && !ackede_denne {
                // Requesten er udstedt men ikke kvitteret: enten på vej frem,
                // eller løbet tør for tid uden nogensinde at melde sig.
                let udloebet = active.is_some_and(|a| now > a.launch_deadline.as_str());
                if udloebet {
                    WorkspaceState::Failed
                } else {
                    WorkspaceState::Starting
                }
            } else if alive {
                WorkspaceState::Running
            } else {
                WorkspaceState::Stopped
            };

            // SESSIONS-felterne gates på liveness, præcis som `state` er det.
            // `status.json` overlever processen, og `attention` har PRÆCIS én
            // skriver — `run_badge_tick` — som stoppes af det delte stop-flag
            // uden nogensinde at skrive et afsluttende `false`. Uden gaten
            // beholdt et lukket workspace derfor sin ravgule "venter på
            // dig"-prik permanent (og frontendens `dotStyle`/`statusText`
            // læser `attention` FØR `state`, så prikken undertrykte også "ikke
            // åben"). Samme argument gælder `running_cards`: der kører ingen
            // sessioner i en proces der ikke findes, og tallet gater
            // luk-bekræftelsen i rail'en.
            //
            // `cards` er derimod projektets kort på disken — de kommer tilbage
            // når workspacet startes igen — og forbliver ugatet.
            let sessionsstatus = if alive { status.as_ref() } else { None };

            WorkspaceSummary {
                state,
                cards: status.as_ref().map(|s| s.cards).unwrap_or(0),
                running_cards: sessionsstatus.map(|s| s.running_cards).unwrap_or(0),
                attention: sessionsstatus.is_some_and(|s| s.attention),
                attention_kind: sessionsstatus
                    .map(WorkspaceStatus::effective_attention_kind)
                    .unwrap_or(AttentionKind::None),
                // AKTIV betyder "er fremme nu" — ikke "der er sendt en request".
                // Et stoppet workspace der lige er blevet klikket, er `starting`,
                // ikke aktivt; ellers ville rail'en fremhæve to poster på én gang.
                is_active: ackede_denne && status.as_ref().is_some_and(|s| s.visible),
                root: e.root.as_ref().map(|r| r.display().to_string()),
                slug: e.slug,
                name: e.name,
                path_hint: e.path_hint,
                hidden: e.hidden,
                defect: e.defect,
            }
        })
        .collect()
}

/// Delt spawn-vej for CLI og app. DETACHED_PROCESS + null-stdio, så hverken
/// launcheren eller en capturing shell holdes i live af arvede pipe-handles.
pub fn spawn_workspace(exe: &Path, state_dir: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    std::process::Command::new(exe)
        .env("TALMINAL_HOME", state_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(DETACHED_PROCESS)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("spawn {} failed: {e}", exe.display()))
}

/// Hvad er den højeste attention-tilstand blandt processens terminal-kort?
/// Hele listen scannes: `done_unread` må ikke short-circuite et senere
/// `needs_you`.
pub fn attention_kind_now() -> AttentionKind {
    let mut result = AttentionKind::None;
    for handle in crate::registry::all_handles() {
        let card = handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let kind = card
            .terminal()
            .and_then(|terminal| terminal.attention.as_ref())
            .map(|machine| machine.poll_kind(attention::now_ms()))
            .unwrap_or(AttentionKind::None);
        match kind {
            AttentionKind::NeedsYou => result = AttentionKind::NeedsYou,
            AttentionKind::DoneUnread if result == AttentionKind::None => {
                result = AttentionKind::DoneUnread;
            }
            _ => {}
        }
    }
    result
}

/// Sidst udsendte synligheds-kant. 0 = pollertråden har aldrig talt (CLI,
/// tests, og vinduet før første reveal), 1 = skjult, 2 = synlig.
///
/// Findes for de maskiner der IKKE var i registryet da kanten faldt: et kort
/// der spawner samtidig med en reveal installeres først når PTY'en er oppe, og
/// `set_attention_visibility` er kanttrigget. `spawn_into` læser derfor
/// stemplet efter installationen (`attention::sync_initial_visibility`).
static ATTENTION_VISIBILITY: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// `None` = der er endnu ikke faldet nogen kant.
pub fn attention_visibility_now() -> Option<bool> {
    match ATTENTION_VISIBILITY.load(Ordering::SeqCst) {
        1 => Some(false),
        2 => Some(true),
        _ => None,
    }
}

/// Fortæller ALLE kortets attention-maskiner at workspacet blev skjult/vist.
/// Kanttrigget med vilje: `on_conceal` rydder prikken, så et kald pr. tick
/// ville slukke den for evigt, mens workspacet var skjult.
pub fn set_attention_visibility(visible: bool) {
    // STEMPLET FØR ITERATIONEN — bindende. Det er den rækkefølge der gør
    // `sync_initial_visibility` korrekt: en maskine der ikke nås af løkken
    // herunder (fordi den installeres et øjeblik senere) læser stemplet selv,
    // og en maskine der læste stemplet før det blev sat, er pr. konstruktion
    // allerede i registryet og nås af løkken.
    ATTENTION_VISIBILITY.store(if visible { 2 } else { 1 }, Ordering::SeqCst);
    for handle in crate::registry::all_handles() {
        let card = handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(machine) = card
            .terminal()
            .and_then(|terminal| terminal.attention.as_ref())
        {
            if visible {
                machine.on_reveal();
            } else {
                machine.on_conceal();
            }
        }
    }
}

/// Badge-løkken (1 s): skriver denne proces' attention-flag og udsender listen
/// **kun ved diff**. Diffen er hele pointen — uden den ville rail'en få et event
/// i sekundet, og et projekt tilføjet fra CLI'en ville stadig dukke op live.
///
/// Løkken bor her og ikke i `main.rs` (Global Constraints): beslutningen "er
/// noget ændret" er logik, og logik skal kunne nås fra `tests/`.
pub fn run_badge_tick(
    global_base: PathBuf,
    status_writer: std::sync::Arc<status::StatusWriter>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    emit: impl Fn(&[WorkspaceSummary]),
) {
    let mut sidst: Option<Vec<WorkspaceSummary>> = None;
    let mut badge_wake = wake::BadgeWake::new();
    while !stop.load(Ordering::SeqCst) {
        // LÅSEORDEN (status.rs' modul-doc): `attention_kind_now()` låser hvert
        // `CardRuntime`, så den må ikke kaldes inde i `update`-closuren —
        // det ville give StatusWriter → CardRuntime, mens `spawn_into` går
        // CardRuntime → StatusWriter. Værdien beregnes derfor FØR låsen.
        let kind = attention_kind_now();
        status_writer.update(|status| {
            status.attention = kind.is_attention();
            status.attention_kind = kind;
        });
        let active = read_active(&global_base);
        let next = summaries(&global_base, active.as_ref(), &now_iso_z());
        if sidst.as_ref() != Some(&next) {
            emit(&next);
            sidst = Some(next);
        }
        badge_wake.wait(std::time::Duration::from_millis(BADGE_TICK_MS));
    }
}

pub struct Poller {
    pub stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub handle: std::thread::JoinHandle<()>,
}

impl Poller {
    pub fn shutdown(self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = self.handle.join();
    }
}

pub fn run_poller(
    global_base: PathBuf,
    me: String,
    instance_id: String,
    status_writer: std::sync::Arc<status::StatusWriter>,
    surface: std::sync::Arc<dyn surface::WindowSurface>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;

    // Første tick køres straks. Derefter venter vi enten på targetets named
    // event eller på den eksisterende 200 ms timeout som crash-sikkert fallback.
    let poll_wake = wake::PollWake::new(&global_base, &me);
    while !stop.load(Ordering::SeqCst) {
        // Control-kanalen (T11): en anden proces kan bede MIG lukke. `take_pending`
        // FJERNER anmodningen, så tre klik i rail'en giver én lukning. Seamen er
        // en no-op indtil `close::install_peer_close` er kaldt i app-opstarten —
        // CLI'en og testene har intet vindue at lukke.
        if let Some(request) = control::take_pending(&global_base, &me) {
            match request.action.as_str() {
                control::CLOSE => close::fire_peer_close(),
                control::QUIT_ALL => {
                    close::fire_peer_quit();
                    // Global exit må ikke fortsætte ned i samme ticks
                    // reveal/hide-protokol og risikere at vise vinduet igen,
                    // efter quit-hooken netop har skjult det.
                    break;
                }
                ukendt => {
                    eprintln!("[canvas] ukendt workspace-control handling for {me}: {ukendt}")
                }
            }
        }

        let active = read_active(&global_base);
        // Samme låseorden-regel: `surface.is_visible()` går ud i Tauri/WebView2
        // og må ikke kaldes med writeren i hånden. (Skrivningen selv er nu
        // gratis når intet ændrer sig — `StatusWriter::update` springer disken
        // over ved uændret krop, så pollerens 5 Hz ikke fsync'er i tomgang.)
        let synlig = surface.is_visible();
        status_writer.update(|status| status.visible = synlig);
        let my_status = status_writer.snapshot();

        let other = active.as_ref().filter(|request| request.slug != me);
        let target_alive = other.is_some_and(|request| instance_alive(&global_base, &request.slug));
        let live_slugs = live_slugs(&global_base);
        // ÉT opslag pr. slug pr. tick. Targetets `status.json` blev tidligere
        // aabnet og parset TO gange i samme tick — én gang som `target_status`
        // og én gang inde i `any_visible`-loekken. Ved 5 Hz er det en fil-read
        // + en serde-parse i sekundet pr. aabent workspace, uden at
        // state-maskinen ser noget andet.
        let live_statuses: Vec<Option<WorkspaceStatus>> = live_slugs
            .iter()
            .map(|slug| {
                if slug == &me {
                    Some(my_status.clone())
                } else {
                    read_status(&global_base, slug)
                }
            })
            .collect();
        let any_visible = live_statuses
            .iter()
            .any(|status| status.as_ref().is_some_and(|status| status.visible));
        // Targetet behoever ikke vaere i `live_slugs` — en doed proces har
        // stadig en fil — saa fald tilbage til en direkte laesning.
        let target_status = other.and_then(|request| {
            match live_slugs.iter().position(|slug| slug == &request.slug) {
                Some(index) => live_statuses[index].clone(),
                None => read_status(&global_base, &request.slug),
            }
        });
        let now = now_iso_z();

        let action = protocol::decide(&protocol::Inputs {
            me: &me,
            my_status: &my_status,
            active: active.as_ref(),
            target_status: target_status.as_ref(),
            now: &now,
            target_alive,
            live_slugs: &live_slugs,
            any_visible,
        });

        match action {
            protocol::Action::Reveal { request } => match surface.reveal() {
                Ok(()) => {
                    // Ejeren kigger igen: maskinerne må ikke tælle output som
                    // "venter på dig" — men en allerede tændt prik bliver
                    // stående, indtil der faktisk svares (T7).
                    set_attention_visibility(true);
                    if read_active(&global_base).map(|active| active.id) == Some(request.id.clone())
                    {
                        status_writer.update(|status| {
                            status.acked_request = Some(request.id);
                            status.visible = true;
                        });
                        // Hintet skal følge det workspace der FAKTISK blev
                        // synligt — ikke kun det der stod i setup. No-op for
                        // "default" (se funktionens doc).
                        if let Err(error) =
                            crate::project::write_last_project_for_active(&global_base, &me)
                        {
                            eprintln!("[canvas] last_project-hint for {me} fejlede: {error}");
                        }
                        // Ack'et er nu på disk. Væk den synlige kilde med det
                        // samme, så den kan conceal'e uden endnu et poll-vindue.
                        // Ekstra levende peers må gerne få et billigt no-op-tick.
                        for slug in live_slugs.iter().filter(|slug| *slug != &me) {
                            let _ = wake::notify(&global_base, slug);
                        }
                        // Targetets egen React-rail skal markere den nye aktive
                        // række nu, ikke først ved badge-trådens næste 1 s tick.
                        wake::notify_badge_refresh();
                    }
                }
                Err(error) => {
                    eprintln!("[canvas] reveal af {me} fejlede: {error}");
                }
            },
            protocol::Action::Conceal => {
                status_writer.update(|status| status.visible = false);
                surface.conceal();
                // Herfra tæller output som noget ejeren ikke ser.
                set_attention_visibility(false);
            }
            protocol::Action::Rollback {
                failed_slug,
                observed,
            } => {
                if read_active(&global_base).map(|active| active.id) == Some(observed) {
                    let mine = next_request(&global_base, &me, &instance_id, deadline_from_now());
                    let _ = write_active(&global_base, &mine);
                    eprintln!("[canvas] workspace {failed_slug} svarede ikke — bliver i {me}");
                }
            }
            protocol::Action::TakeOver => {
                let mine = next_request(&global_base, &me, &instance_id, deadline_from_now());
                let _ = write_active(&global_base, &mine);
            }
            protocol::Action::Nothing => {}
        }

        let _ = poll_wake.wait(std::time::Duration::from_millis(POLL_MS));
    }
}

pub fn live_slugs(global_base: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(global_base.join("projects")) else {
        return Vec::new();
    };
    let mut slugs: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|slug| instance_alive(global_base, slug))
        .collect();
    slugs.sort();
    slugs
}

/// LEVENDE = den navngivne mutex `Talminal-<slug>` findes.
///
/// Signalet er mutexen fra `acquire_instance_lock`s `CreateMutexW`-lag: den skabes
/// af den kørende proces og forsvinder når processen dør — også ved crash, fordi
/// kernen lukker handles. `global_base` indgår ikke: duplikat-værnet er
/// maskinbredt pr. slug, så en probe der kiggede i én rod ville kunne melde
/// "død" om en proces der lever i en anden.
///
/// **Kendelse CORRECTIONS.md C-T3.** Planens variant åbnede
/// `projects/<slug>/.lock` eksklusivt (`share_mode(0)`) og læste `Err` som
/// "levende". Den holdt dermed låsen i et mikrosekund pr. probe, og polleren
/// prober hvert `POLL_MS` pr. workspace: en proces der netop er inde i
/// `acquire_instance_lock`s `.lock`-aabning kunne tabe kapløbet om SIN EGEN
/// `.lock` og afslutte som "kører allerede". `OpenMutexW` er read-only og har
/// ingen sådan race. Handlet lukkes straks — ellers ville proben selv holde
/// mutexen i live efter at ejeren døde.
#[cfg(windows)]
pub fn instance_alive(_global_base: &Path, slug: &str) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};

    let name = crate::instance::to_wide(&format!("Talminal-{slug}"));
    let handle = unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) };
    if handle.is_null() {
        return false;
    }
    unsafe {
        CloseHandle(handle);
    }
    true
}

#[cfg(not(windows))]
pub fn instance_alive(global_base: &Path, slug: &str) -> bool {
    global_base
        .join("projects")
        .join(slug)
        .join(".lock")
        .exists()
}

/// Næste request: højeste af (det der står på disken, det denne proces selv
/// senest udstedte) plus én, stemplet med MIN identitet.
/// To processer kan nå frem til samme `seq` — men aldrig samme `issuer`, så
/// parret er entydigt. Last-writer-wins på filen er korrekt semantik: den
/// seneste brugerhandling skal vinde.
///
/// VANDMÆRKET er ikke overflødigt. `read_active` mapper BÅDE fravær og
/// korruption til `None` (bevidst — en ulæselig fil må ikke fælde pollertråden),
/// så uden det ville en enkelt tabt eller halvskrevet fil sende `seq` tilbage
/// til 1. Samme proces kunne dermed genudstede et `(issuer, seq)` den allerede
/// har brugt, hvilket modsiger modul-doc'ens "parret er entydigt" og genåbner præcis
/// det ABA-hul rollbackens parvise sammenligning er bygget for at lukke.
pub fn next_request(
    global_base: &Path,
    slug: &str,
    issuer: &str,
    deadline: String,
) -> ActiveRequest {
    /// Proces-lokalt højvandsmærke over alle `seq` denne proces har udstedt.
    static HIGH_WATER: AtomicU64 = AtomicU64::new(0);

    let on_disk = read_active(global_base).map(|a| a.id.seq).unwrap_or(0);
    // CAS-løkke frem for load+store: to tråde i samme proces må ikke kunne
    // læse samme vandmærke og udstede samme seq under samme issuer.
    let mut set = HIGH_WATER.load(Ordering::Relaxed);
    let seq = loop {
        let next = set.max(on_disk) + 1;
        match HIGH_WATER.compare_exchange_weak(set, next, Ordering::SeqCst, Ordering::Relaxed) {
            Ok(_) => break next,
            Err(faktisk) => set = faktisk,
        }
    };
    ActiveRequest {
        id: RequestId {
            issuer: issuer.to_string(),
            seq,
        },
        slug: slug.to_string(),
        launch_deadline: deadline,
    }
}
