//! Lukkeprotokollen: én funnel for ALLE luk-veje, og en tilstandsmaskine der
//! ikke blokerer event-loopet.
//!
//! **Hvorfor en tilstandsmaskine og ikke bare en handler.**
//! `WindowEvent::CloseRequested` kører på event-loopet. Ventede handleren
//! synkront på en successors kvittering, ville hele appen fryse — inklusive den
//! overdragelse den venter på, for successoren tager forgrunden gennem netop
//! det loop. Forløbet er derfor delt i to: handleren sætter kun retning og
//! returnerer straks, og selve overdragelsen kører i sin egen tråd, hvor
//! blokering er i orden.
//!
//! **Hvorfor maskinen bor her og ikke i `main.rs`.** Global Constraint (plan)
//! siger ordret at close-logikken bor i lib'en, og `tests/` kan ikke nå
//! binary-craten. Kendelse CORRECTIONS.md C-T11 fryser snittet: `ClosePhase`,
//! `CloseAction` og den RENE `next_phase()` her — `main.rs`' handler kalder kun
//! den og udfører resultatet.
//!
//! **Faserne.**
//! ```text
//!   titel-✕ / Alt+F4 / taskbar
//!              |
//!   Idle --> Confirming --(ja)--> Quitting --> quit-all til alle processer
//!     ^          |
//!     +--(nej)---+
//!
//!   rail-✕ / peer-close --> HandingOff --> Approved --> luk ét workspace
//! ```
//! `Approved` er den ENE fase hvor eventet får lov at passere, og derfra kører
//! den normative `ExitRequested`-teardown i `main.rs` uændret.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::Manager;

/// Hvor i lukningen er vi? Bor i app-state; handleren læser og skriver den
/// gennem [`CloseState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClosePhase {
    /// Ingen lukning i gang.
    #[default]
    Idle,
    /// Brugeren er blevet spurgt; vi venter på `confirm_close(ok)`.
    Confirming,
    /// Overdragelsen kører i sin egen tråd.
    HandingOff,
    /// Brugeren har godkendt en global exit. Alle direkte vinduesluk holdes
    /// tilbage, indtil hver proces går gennem `AppHandle::exit`.
    Quitting,
    /// Forløbet er færdigt — næste `CloseRequested` skal passere.
    Approved,
}

/// Hvad `main.rs` skal GØRE. Handleren oversætter én-til-én; der er bevidst
/// ingen `Option`/`bool`-kombinationer at fortolke forkert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// Stop lukningen og spørg brugeren (`close-confirm-requested` med globalt snapshot).
    PreventAndConfirm { running: u32 },
    /// Stop lukningen; der sker ikke andet (forløbet er allerede i gang).
    Prevent,
    /// Broadcast global exit og afslut derefter denne proces.
    QuitAll,
    /// Lad lukningen passere til `ExitRequested`-teardown.
    Allow,
}

/// Den rene overgangsfunktion for et `CloseRequested`.
///
/// `running_cards` er det samlede antal kørende kort i alle levende workspaces.
/// Det bruges kun som payload til advarslen; der spørges ALTID, også ved nul.
/// Bemærk at tallet er uden betydning i `HandingOff`/`Quitting`/`Approved`.
///
/// **`Confirming` spørger IGEN i stedet for bare at afvise.** Spørgsmålet
/// stilles med et event, og et event har ingen leveringsgaranti: lander
/// `close-confirm-requested` før frontenden har nået at registrere sin lytter
/// (`App.tsx` registrerer den asynkront efter mount), er der ingen dialog, intet
/// `confirm_close` — og fasen står i `Confirming` for evigt, hvorefter ALLE
/// senere ✕/Alt+F4 blot afvises og vinduet ikke længere kan lukkes. Et gentaget
/// forsøg skal derfor kunne genstille spørgsmålet. Det giver ikke to dialoger:
/// frontenden køer højst én `application`-anmodning (`App.tsx`s
/// `close-confirm-requested`-lytter).
pub fn next_phase(current: ClosePhase, running_cards: u32) -> (ClosePhase, CloseAction) {
    match current {
        ClosePhase::Approved => (ClosePhase::Approved, CloseAction::Allow),
        ClosePhase::Confirming => (
            ClosePhase::Confirming,
            CloseAction::PreventAndConfirm {
                running: running_cards,
            },
        ),
        ClosePhase::HandingOff => (ClosePhase::HandingOff, CloseAction::Prevent),
        ClosePhase::Quitting => (ClosePhase::Quitting, CloseAction::Prevent),
        ClosePhase::Idle => (
            ClosePhase::Confirming,
            CloseAction::PreventAndConfirm {
                running: running_cards,
            },
        ),
    }
}

/// Frontendens svar på `close-confirm-requested`.
///
/// Svar der kommer i en anden fase end `Confirming` er UBEDTE og ignoreres.
/// Det er ikke pedanteri: en peer-lukning kan have overhalet brugerens dialog,
/// og et forsinket "nej" må ikke kunne sætte fasen tilbage til `Idle` midt i en
/// overdragelse eller global exit.
pub fn on_confirm(current: ClosePhase, ok: bool) -> (ClosePhase, CloseAction) {
    match (current, ok) {
        (ClosePhase::Confirming, true) => (ClosePhase::Quitting, CloseAction::QuitAll),
        (ClosePhase::Confirming, false) => (ClosePhase::Idle, CloseAction::Prevent),
        (andet, _) => (andet, CloseAction::Prevent),
    }
}

/// Hvem skal overtage skærmen når jeg lukker?
///
/// `None` betyder "spring overdragelsen over — luk bare":
/// - **Jeg er ikke synlig.** Et skjult workspace der lukkes fjernstyret må
///   ALDRIG udstede en request; det ville rive fladen væk fra den proces
///   brugeren faktisk kigger på.
/// - **Der er ingen andre levende.** Så er der ingen skærm at give videre, og
///   appen lukker helt.
///
/// Ellers: næste slug i den (sorterede) liste, ellers den forrige. `live_slugs`
/// kommer sorteret fra [`crate::workspaces::live_slugs`], så "næste/forrige" er
/// den samme rækkefølge brugeren ser i rail'en.
pub fn handoff_target(i_am_visible: bool, live_slugs: &[String], me: &str) -> Option<String> {
    if !i_am_visible {
        return None;
    }
    let andre: Vec<&String> = live_slugs.iter().filter(|s| s.as_str() != me).collect();
    andre
        .iter()
        .find(|s| s.as_str() > me)
        .or_else(|| andre.last())
        .map(|s| (*s).clone())
}

/// Hvor længe overdragelsen venter på successorens kvittering, før den lukker
/// alligevel. Kortere end `LAUNCH_DEADLINE_SECS` (8 s) med vilje: pollerens
/// rollback-gren må ikke nå at fyre og skrive requesten tilbage til mig selv
/// mens jeg er på vej ud.
const HANDOFF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const HANDOFF_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// Frisk backend-snapshot til appens globale exit-advarsel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct QuitSummary {
    pub workspaces: u32,
    pub running_cards: u32,
}

pub fn quit_summary(global_base: &Path) -> QuitSummary {
    let slugs = super::live_slugs(global_base);
    QuitSummary {
        workspaces: slugs.len().try_into().unwrap_or(u32::MAX),
        running_cards: slugs
            .iter()
            .filter_map(|slug| super::read_status(global_base, slug))
            .map(|status| status.running_cards)
            .fold(0_u32, u32::saturating_add),
    }
}

/// Fasen plus det overdragelsen har brug for at vide om MIG.
pub struct CloseState {
    global_base: PathBuf,
    me: String,
    phase: Mutex<ClosePhase>,
    /// `AppHandle::exit` må startes højst én gang pr. proces. Det er adskilt fra
    /// fasen, fordi initiatoren står i `Quitting` allerede FØR peer-broadcasten
    /// er færdig, mens en modtager går direkte fra enhver fase til exit.
    quit_started: AtomicBool,
}

impl CloseState {
    pub fn new(global_base: PathBuf, me: String) -> Self {
        Self {
            global_base,
            me,
            phase: Mutex::new(ClosePhase::Idle),
            quit_started: AtomicBool::new(false),
        }
    }

    pub fn phase(&self) -> ClosePhase {
        *self.phase.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// `CloseRequested` fra event-loopet. Låsen holdes kun over overgangen —
    /// intet I/O, intet vindues-kald.
    pub fn on_close_requested(&self, running_cards: u32) -> CloseAction {
        let mut guard = self.phase.lock().unwrap_or_else(|p| p.into_inner());
        let (næste, action) = next_phase(*guard, running_cards);
        *guard = næste;
        action
    }

    /// `confirm_close(ok)` fra frontenden.
    pub fn on_confirm(&self, ok: bool) -> CloseAction {
        let mut guard = self.phase.lock().unwrap_or_else(|p| p.into_inner());
        let (næste, action) = on_confirm(*guard, ok);
        *guard = næste;
        action
    }

    /// En ANDEN proces har bedt mig lukke (control-kanalen). Bekræftelsen
    /// springes over: rail'en i den synlige proces har allerede spurgt
    /// brugeren, og et spørgsmål stillet i et skjult vindue kan ingen svare på
    /// — lukningen ville hænge i `Confirming` for evigt.
    ///
    /// Returnerer `true` når KALDEREN skal starte overdragelsen. En anmodning
    /// der lander mens en overdragelse allerede kører, giver `false`: tre klik
    /// skal give én lukning.
    pub fn begin_peer_close(&self) -> bool {
        let mut guard = self.phase.lock().unwrap_or_else(|p| p.into_inner());
        match *guard {
            ClosePhase::Idle | ClosePhase::Confirming => {
                *guard = ClosePhase::HandingOff;
                true
            }
            ClosePhase::HandingOff | ClosePhase::Quitting | ClosePhase::Approved => false,
        }
    }

    /// Global exit vinder over både en åben dialog og en igangværende handoff.
    /// `true` betyder at kalderen ejer det ENE `AppHandle::exit`.
    pub fn begin_peer_quit(&self) -> bool {
        if self.quit_started.swap(true, Ordering::SeqCst) {
            return false;
        }
        *self.phase.lock().unwrap_or_else(|p| p.into_inner()) = ClosePhase::Quitting;
        true
    }

    /// Forløbet er færdigt; næste `CloseRequested` skal passere. En samtidig
    /// global exit må aldrig overskrives af en sen handoff-kvittering.
    pub fn approve(&self) -> bool {
        let mut guard = self.phase.lock().unwrap_or_else(|p| p.into_inner());
        if *guard == ClosePhase::Quitting {
            return false;
        }
        *guard = ClosePhase::Approved;
        true
    }

    /// Overdragelsen mislykkedes — lukningen er AFLYST, ikke godkendt.
    ///
    /// Uden den ville fasen stå i `HandingOff`, hvor hvert nyt ✕ svarer
    /// `Prevent`: vinduet blev stående (korrekt) og kunne aldrig lukkes igen
    /// (forkert). Brugeren skal kunne prøve igen.
    pub fn abort(&self) {
        let mut guard = self.phase.lock().unwrap_or_else(|p| p.into_inner());
        if *guard == ClosePhase::HandingOff {
            *guard = ClosePhase::Idle;
        }
    }

    /// Broadcasten fejlede før den lokale exit blev startet. Gør et nyt forsøg
    /// muligt; når `quit_started` først er sand, er nedlukningen irreversibel.
    pub fn abort_quit(&self) {
        if self.quit_started.load(Ordering::SeqCst) {
            return;
        }
        let mut guard = self.phase.lock().unwrap_or_else(|p| p.into_inner());
        if *guard == ClosePhase::Quitting {
            *guard = ClosePhase::Idle;
        }
    }

    pub fn global_base(&self) -> &Path {
        &self.global_base
    }

    pub fn me(&self) -> &str {
        &self.me
    }
}

// --- Control-kanalens seam ------------------------------------------------

/// Pollertråden kalder [`fire_peer_close`] når `control::take_pending` fandt en
/// anmodning for MIT slug. Hvad der så sker, kræver en `AppHandle`, og
/// `run_poller`s signatur er frossen (T3's tests kalder den med seks
/// argumenter). En proces-global seam er derfor både det mindste indgreb og
/// det eneste der ikke brækker en anden tasks lease.
static PEER_CLOSE_HOOK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
static PEER_QUIT_HOOK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

pub fn set_peer_close_hook(hook: Box<dyn Fn() + Send + Sync>) {
    let _ = PEER_CLOSE_HOOK.set(hook);
}

pub fn set_peer_quit_hook(hook: Box<dyn Fn() + Send + Sync>) {
    let _ = PEER_QUIT_HOOK.set(hook);
}

/// Kaldes af pollertråden. Er der ingen hook (tests, CLI), sker der intet.
pub fn fire_peer_close() {
    if let Some(hook) = PEER_CLOSE_HOOK.get() {
        hook();
    }
}

/// Global exit er en anden intention end workspace-close: ingen successor må
/// afsløres, for alle processer er på vej ned.
pub fn fire_peer_quit() {
    if let Some(hook) = PEER_QUIT_HOOK.get() {
        hook();
    }
}

// --- Tauri-siden ----------------------------------------------------------

/// Kobler control-kanalen til lukkevejen. `main.rs` kalder den én gang i setup.
pub fn install_peer_close(app: tauri::AppHandle) {
    let close_app = app.clone();
    set_peer_close_hook(Box::new(move || {
        // Uden tilstanden findes der ingen fase at rykke, og en lukning her ville
        // være ustyret. Hellere tabe anmodningen end at lukke ukontrolleret.
        let Some(state) = close_app.try_state::<std::sync::Arc<CloseState>>() else {
            eprintln!("[canvas] lukkeanmodning ignoreret: CloseState mangler");
            return;
        };
        if !state.begin_peer_close() {
            return;
        }
        spawn_handoff_then_close(close_app.clone());
    }));

    set_peer_quit_hook(Box::new(move || start_process_quit(app.clone())));
}

/// Afslut alle andre levende workspace-processer og derefter initiatoren.
///
/// Peer-filerne skrives først. Fejler én, bliver initiatoren stående og kan vise
/// fejlen/prøve igen; den lokale exit startes først, når hele broadcasten er
/// accepteret. Stoppede workspaces berøres aldrig, så der efterlades ingen
/// `control.json`-landmine til næste opstart.
pub fn quit_all(app: tauri::AppHandle) -> Result<(), String> {
    let state = app
        .try_state::<std::sync::Arc<CloseState>>()
        .ok_or("lukketilstanden er ikke initialiseret")?;
    let peers = super::live_slugs(state.global_base())
        .into_iter()
        .filter(|slug| slug != state.me())
        .collect::<Vec<_>>();

    let mut failures = Vec::new();
    for slug in peers {
        if let Err(error) = super::control::request_quit_all(state.global_base(), &slug) {
            failures.push(format!("{slug}: {error}"));
        }
    }
    if !failures.is_empty() {
        state.abort_quit();
        return Err(format!(
            "Talminal kunne ikke afslutte alle workspaces ({})",
            failures.join("; ")
        ));
    }

    start_process_quit(app);
    Ok(())
}

/// Proceslokal halvdel af global exit. `AppHandle::exit` er afgørende: den
/// udløser `RunEvent::ExitRequested`, som persisterer og dræner alle PTY-hosts.
/// Vinduet skjules straks efter brugerens godkendelse, mens den sikre teardown
/// får lov at køre færdig i baggrunden.
pub fn start_process_quit(app: tauri::AppHandle) {
    let Some(state) = app.try_state::<std::sync::Arc<CloseState>>() else {
        eprintln!("[canvas] global exit ignoreret: CloseState mangler");
        return;
    };
    if !state.begin_peer_quit() {
        return;
    }
    if let Some(poller) = app.try_state::<super::Poller>() {
        poller.stop.store(true, Ordering::SeqCst);
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    app.exit(0);
}

/// Overdragelsen. **Kaldes ALDRIG fra event-loopet** — den blokerer op til
/// [`HANDOFF_TIMEOUT`] mens den venter på successorens kvittering.
///
/// Rækkefølgen er bindende:
/// 1. find successoren (ingen ⇒ spring over),
/// 2. overdrag forgrundsretten til dens pid (Windows nægter ellers cross-process
///    aktivering),
/// 3. skriv requesten,
/// 4. vent på et KORRELERET ack (`acked_request == min request` + `visible`),
/// 5. `Approved`, stop pollerne, kald `window.close()` igen.
///
/// **Uden ack lukkes der IKKE** — se [`hand_off`].
pub fn spawn_handoff_then_close(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        if let Some(state) = app.try_state::<std::sync::Arc<CloseState>>() {
            match hand_off(state.inner().as_ref()) {
                Handoff::Klar => {
                    if !state.approve() {
                        return;
                    }
                }
                Handoff::Afbrudt { target } => {
                    // Lukningen er AFLYST. Fasen tilbage til `Idle`, så et nyt ✕
                    // kan prøve igen, og brugeren får at vide hvorfor vinduet
                    // blev stående — ellers ser det ud som en død knap.
                    state.abort();
                    let _ = tauri::Emitter::emit(&app, "workspace-handoff-failed", target);
                    return;
                }
            }
        }
        // Pollerne skal STOPPE før vinduet lukker: ellers kan et sidste tick nå
        // at kalde `reveal()` på et vindue der er ved at forsvinde.
        if let Some(poller) = app.try_state::<super::Poller>() {
            poller.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        if let Some(window) = tauri::Manager::get_webview_window(&app, "main") {
            let _ = window.close();
        } else {
            // Intet vindue at lukke (skulle ikke kunne ske) — luk appen, så en
            // fjernstyret lukning ikke efterlader en zombie-proces.
            tauri::Manager::app_handle(&app).exit(0);
        }
    });
}

/// Udfaldet af en overdragelse. `Afbrudt` bærer successorens slug, så brugeren
/// kan få at vide hvem der ikke svarede.
pub enum Handoff {
    Klar,
    Afbrudt { target: String },
}

/// **Ingen kvittering ⇒ ingen lukning.**
///
/// Spec §6.4 siger ordret at det aktive workspace overdrager synligheden "inkl.
/// ack" og lukker DEREFTER, og udpeger rev 1.0's "skriv successor og luk uden at
/// vente" som defekten. En timeout der lukker alligevel er den samme defekt med
/// et loft på: successoren kan være levende og alligevel ikke nå frem (WebView2
/// throttling af et skjult vindue, en hængende `SetForegroundWindow`, en maskine
/// under belastning), og så er der ingen tilbage til at vise noget. Værnene
/// nedenunder fanger det ikke: rollback-grenen kræver en SYNLIG kilde, og
/// takeover-grenen kræver at target er dødt — en levende, tavs successor
/// opfylder ingen af delene, så skærmen ville blive sort indtil brugeren dræbte
/// processer i hånden.
///
/// Ved timeout tages requesten derfor tilbage (samme bevægelse som protokollens
/// `Rollback`), og kalderen aflyser lukningen. Jeg har aldrig skjult mig
/// undervejs — min egen poller står i `Nothing`, fordi successoren ikke har
/// kvitteret — så invarianten "mindst ét synligt vindue" holder hele vejen.
fn hand_off(state: &CloseState) -> Handoff {
    let base = state.global_base();
    let me = state.me();
    let synlig = super::read_status(base, me).is_some_and(|s| s.visible);
    let Some(target) = handoff_target(synlig, &super::live_slugs(base), me) else {
        // Ingen successor: enten er jeg ikke synlig (og river ikke fladen fra
        // nogen ved at lukke), eller jeg er den sidste. Begge dele er en lovlig
        // lukning.
        return Handoff::Klar;
    };

    if let Some(status) = super::read_status(base, &target) {
        super::surface::allow_foreground_for(status.pid);
    }
    let request = super::next_request(
        base,
        &target,
        super::my_instance_id(),
        super::deadline_from_now(),
    );
    if let Err(error) = super::write_active(base, &request) {
        eprintln!("[canvas] overdragelse til {target} kunne ikke skrives: {error}");
        // Requesten nåede aldrig disken, så der er intet at tage tilbage — men
        // der er heller ingen der overtager skærmen. Lukningen aflyses.
        return Handoff::Afbrudt { target };
    }

    let frist = std::time::Instant::now() + HANDOFF_TIMEOUT;
    while std::time::Instant::now() < frist {
        if let Some(status) = super::read_status(base, &target) {
            if status.acked_request.as_ref() == Some(&request.id) && status.visible {
                return Handoff::Klar;
            }
        }
        std::thread::sleep(HANDOFF_POLL);
    }

    // Tag requesten tilbage, så successoren ikke rejser sig et sekund senere og
    // efterlader to synlige vinduer. Kun hvis den stadig er MIN: har brugeren
    // klikket videre imens, ejer den nye overdragelse forløbet (spec §3.3).
    if super::read_active(base).map(|active| active.id) == Some(request.id) {
        let mine = super::next_request(
            base,
            me,
            super::my_instance_id(),
            super::deadline_from_now(),
        );
        let _ = super::write_active(base, &mine);
    }
    eprintln!("[canvas] {target} kvitterede ikke inden for fristen — lukningen er aflyst");
    Handoff::Afbrudt { target }
}
