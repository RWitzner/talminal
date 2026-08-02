//! Muterbart kort-registry (Task 5): create/close/list + runtime-ejerskab.
//!
//! Registryet EJER kortenes runtime-state (foer: main.rs' AppState.cards) som
//! global singleton i lib-cratet (Testbarhed-reglen: tests/ linker lib'en;
//! main.rs-wrapperne er tynde). Socket-signaturerne fra Task 1 Del B er
//! kontrakt og staar uaendret.
//!
//! Browser-kort (browser-cards plan Task 2): registryet baerer nu TO backend-
//! former i samme nummer-/navnerum — `CardBackend::Terminal` (PTY-kortet som
//! hidtil) og `CardBackend::Browser` (embedded webview-kort; selve webviewen
//! ejes af main.rs-laget/browser_host — registryet roerer ALDRIG AppHandle).
//! Fejlkontrakt (bindende): alle PTY-veje paa et browser-kort ⇒
//! `Err("card is a browser: {name}")`.
//!
//! Invarianter (plan Task 5; nummer-genbrug er ejer-beslutning 2026-07-20):
//! - `number` er det LAVESTE ledige nummer >= 1 ved oprettelsen: et lukket
//!   korts nummer genbruges af det naeste kort, der oprettes ("luk 3-6,
//!   aabn 2 nye" -> 3 og 4 — ikke 7 og 8). Levende kort deler aldrig
//!   nummer, og et navn `card-N` er aldrig i brug ved allokeringen.
//!   Browser-kort deler allokatoren (samme nummerserie, spec §3).
//! - create_card-byggede kort: `name == format!("card-{number}")` — navnet er
//!   den stabile noegle ALLE eksisterende kommandoer fortsat bruger
//!   (spawn_card, write_pty, resize_pty, kill_card, get_card_state, …).
//! - toml-seedede kort beholder deres navn; supervisionens `master` bruger
//!   reserveret nummer 0 og påvirker aldrig generated-taelleren.
//! - Resume-binding (spec §6-gate-krav): profil-kort resumer med profilens
//!   `resume_command`; kort med custom `command` resumer med samme `command`
//!   (ingen `--continue`-semantik for custom-kort).
//! - Fejlflader: ugyldig cwd -> `Err("cwd not found: …")`; ukendt kort ved
//!   close -> `Err("no such card: …")`; navne-kollision ved seed (korrupt
//!   workspace-load-fladen) -> `Err("duplicate card name: …")`.
//!
//! LAAS-HIERARKI (fix F2, normativt): registry-map-laasen holdes KUN for
//! opslag/insert/remove — den SLIPPES foer en kort-laas tages, og holdes
//! ALDRIG hen over PtyHost::spawn, host.write eller kill_and_teardown.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
#[cfg(feature = "perf-trace")]
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::cards::CardConfig;
use crate::control::EpochGate;
use crate::profiles;
use crate::prompt_readiness::PromptReadiness;
use crate::pty::PtyHost;
use crate::workspaces::attention::CardAttention;

// `cards::MASTER_NAME` er supervision-gated, mens registryets generated-
// allocator kompilerer i begge feature-states.
const NON_GENERATED_MASTER_NAME: &str = "master";

fn default_kind() -> String {
    "terminal".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardInfo {
    pub number: u32,
    pub name: String,
    pub cwd: String,
    pub profile: String,
    pub running: bool,
    pub exited: Option<u32>,
    /// Task 11 (restore-badgen): "resume" | "fresh_shared_cwd" | "fresh".
    /// Saettes KUN ved app-start (main.rs' restore-plan via
    /// set_restore_action); kort oprettet i sessionens loeb har None.
    /// Bevidst bare `Option` — ingen `#[serde(skip_serializing_if)]` (plan).
    pub restore_action: Option<String>,
    /// Browser-kort-wire (plan Task 2, TS-spejl i Task 7): "terminal" |
    /// "browser". Serde-default saa gamle CardInfo-serialiseringer loader.
    #[serde(default = "default_kind")]
    pub kind: String,
    /// Browser-kort: navnet paa ejer-terminalen (None = canvas-aabnet).
    /// Terminal-kort: altid None.
    #[serde(default)]
    pub opened_by: Option<String>,
    /// Browser-kort: aktuel URL. Terminal-kort: None.
    #[serde(default)]
    pub url: Option<String>,
    /// Browser-kort: sidste kendte dokument-titel. Terminal-kort: None.
    #[serde(default)]
    pub title: Option<String>,
    /// Chat-kort: traadens id. Terminal-/browser-kort: None.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Chat-kort: traadens formaal = kortets titel. Ellers None.
    #[serde(default)]
    pub purpose: Option<String>,
}

/// Terminal-grenen af et kort — PTY-runtime-felterne som hidtil (flyttet
/// uaendret fra den flade CardRuntime ved browser-cards-splittet).
pub struct TerminalRuntime {
    pub config: CardConfig,
    /// Agent-profil-id (CC-only i MVP: "claude") — spawn-stien (main.rs)
    /// slaar deny-listerne op pr. kort her.
    pub profile: String,
    /// Arc (fix F2): write_pty kloner hosten og SLIPPER kort-laasen foer den
    /// blokerende write — en haengende writer kan aldrig spaerre kill/teardown
    /// (kill_and_teardown tager &self og gaar via child-Mutex'en).
    pub pty: Option<Arc<PtyHost>>,
    /// Per-run Claude startup gate. `None` for custom commands and while no
    /// profile PTY is running. The reader owns a clone and signals it from raw
    /// PTY output; submitters may wait without holding any card/registry lock.
    pub submit_readiness: Option<Arc<PromptReadiness>>,
    /// Opmaerksomheds-maskinen for dette kort ("agenten venter paa dig") —
    /// lever praecis saa laenge PTY'en goer, og fodres af samme reader som
    /// `submit_readiness`. `None` for kort uden koerende PTY: et kort der er
    /// vaek, maa ikke kunne holde en prik taendt. Browser-/chat-kort har
    /// ingen — de har intet PTY.
    pub attention: Option<Arc<CardAttention>>,
    /// Supervision: epoch-gate pr. kort. Default-state: control-facadens
    /// ZST-stub — owner altid "persona", epoch altid 0 (frossen IPC-kontrakt).
    pub gate: EpochGate,
    /// Task 8: synligheds-flag (output-gating). `false` ⇒ reader-emitten
    /// dropper kortets chunks (alt-screen har ingen historik at miste,
    /// FUND 2) — pty'en LAESES stadig (backpressure maa aldrig ramme
    /// child'en; gaten sidder i gated_emit, ALDRIG i pty.rs' read-loekke).
    /// Arc: main.rs' reader-closure holder en klon uden kort-laasen.
    /// Default true — kort foedes synlige.
    pub visible: Arc<AtomicBool>,
    /// Task 11: restore-plan-resultatet ("resume" | "fresh_shared_cwd" |
    /// "fresh") — saettes KUN ved app-start; None for kort oprettet senere.
    pub restore_action: Option<String>,
    /// true ⇔ kortet blev oprettet med custom `command` (browser-cards plan
    /// Task 6: kun profil-spawnede CC-workers faar MCP-injektion).
    pub custom_command: bool,
    /// Hvem der maa skrive til dette kort gennem traad-kanalen. Politikken
    /// ligger paa runtime, saa et genbrugt kortnummer aldrig arver samtykke.
    pub accepts_from: AcceptsFrom,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AcceptsFrom {
    Nobody,
    #[default]
    HumanOnly,
    List(Vec<String>),
    Any,
}

/// Browser-grenen: registry-siden af et webview-kort. Webview-objektet ejes
/// af main.rs-laget (browser_host) — her bor kun navne-/tilstands-data.
pub struct BrowserRuntime {
    pub name: String,
    /// Ejer-terminalens navn (kaskade-luk, spec §3); None = canvas-aabnet.
    pub opened_by: Option<String>,
    /// Scope-noeglen ("canvas" / "agent-{name}") — main.rs-laget bruger den
    /// til webview-/CDP-teardown ved close (BrowserClosed-resultatet).
    pub scope_key: String,
    pub url: String,
    pub title: String,
    /// false ⇒ browserprocessen er doed (reconciliation-polleren, spec §5);
    /// wire-formen er `running = alive`.
    pub alive: bool,
    /// Playwright/CDP-target-id for kortets tab (Task 5-lease-udvidelse).
    /// Tom indtil browser_host's CDP-ready-loop har fundet targetet efter
    /// webview-oprettelsen; reconciliation-polleren og MCP-`list` laeser den.
    pub target_id: String,
}

/// Chat-grenen: kortet ER traaden. Ingen PTY, intet webview - kun en peger ind
/// i `threads`-modulet plus titlen ejeren ser. `name` ligger paa runtime som hos
/// `BrowserRuntime`, fordi `CardRuntime::name()` laeser navnet fra backenden.
pub struct ChatRuntime {
    pub name: String,
    pub thread_id: String,
    pub purpose: String,
}

pub enum CardBackend {
    Terminal(TerminalRuntime),
    Browser(BrowserRuntime),
    Chat(ChatRuntime),
}

/// Et korts runtime-state (flyttet fra main.rs ved registry-ombygningen;
/// backend-splittet ved browser-cards Task 2).
pub struct CardRuntime {
    /// Kortets nummer — laveste ledige ved oprettelsen (se invarianterne
    /// oeverst).
    pub number: u32,
    /// `false` saa snart kortet er detached fra registryet. Et spawn, der
    /// allerede naaede at klone runtime-handlet foer close, skal kontrollere
    /// flaget under kort-laasen og maa ikke starte en orphan PTY bagefter.
    pub active: bool,
    /// Sidste exit-status (fix F10) — sat af exit-watcher/kill, nulstillet
    /// ved spawn; baerer reload-hydreringen via get_card_state.
    pub exited: Option<u32>,
    pub backend: CardBackend,
}

impl CardRuntime {
    pub fn name(&self) -> &str {
        match &self.backend {
            CardBackend::Terminal(t) => &t.config.name,
            CardBackend::Browser(b) => &b.name,
            CardBackend::Chat(c) => &c.name,
        }
    }

    pub fn terminal(&self) -> Option<&TerminalRuntime> {
        match &self.backend {
            CardBackend::Terminal(t) => Some(t),
            CardBackend::Browser(_) => None,
            CardBackend::Chat(_) => None,
        }
    }

    pub fn terminal_mut(&mut self) -> Option<&mut TerminalRuntime> {
        match &mut self.backend {
            CardBackend::Terminal(t) => Some(t),
            CardBackend::Browser(_) => None,
            CardBackend::Chat(_) => None,
        }
    }

    pub fn browser_mut(&mut self) -> Option<&mut BrowserRuntime> {
        match &mut self.backend {
            CardBackend::Terminal(_) => None,
            CardBackend::Browser(b) => Some(b),
            CardBackend::Chat(_) => None,
        }
    }

    pub fn kind_str(&self) -> &'static str {
        match &self.backend {
            CardBackend::Terminal(_) => "terminal",
            CardBackend::Browser(_) => "browser",
            CardBackend::Chat(_) => "chat",
        }
    }
}

struct Inner {
    cards: HashMap<String, Arc<Mutex<CardRuntime>>>,
    /// Kun navne skabt af `create_card`/`create_browser_card`; seedede kort
    /// (isaer supervisionens permanente `master`) taeller ikke som
    /// genererede i `sequence_reset`-rapporteringen.
    generated_names: HashSet<String>,
    /// Levende korts nummer pr. navn (inkl. seedede; master staar med 0).
    /// Vedligeholdes ved PRAECIS de fire map-mutationssteder (create x2,
    /// seed, close-remove) — kortnumre bor bag kort-laasene og maa ikke
    /// laeses under map-laasen (laas-hierarkiet, fix F2), saa allokatoren
    /// scanner dette spejl i stedet.
    numbers: HashMap<String, u32>,
}

/// Den globale registry-singleton. Interior mutability efter det
/// eksisterende Mutex-moenster (fix F2: laasen holdes kun for map-ops).
static REGISTRY: LazyLock<Mutex<Inner>> = LazyLock::new(|| {
    Mutex::new(Inner {
        cards: HashMap::new(),
        generated_names: HashSet::new(),
        numbers: HashMap::new(),
    })
});

fn with_registry<T>(f: impl FnOnce(&mut Inner) -> T) -> Result<T, String> {
    let mut inner = REGISTRY.lock().map_err(|e| e.to_string())?;
    Ok(f(&mut inner))
}

fn sequence_reset_state(inner: &Inner) -> bool {
    let only_non_numbered_master_remains = inner
        .cards
        .keys()
        .all(|name| name.eq_ignore_ascii_case(NON_GENERATED_MASTER_NAME));
    inner.generated_names.is_empty() && only_non_numbered_master_remains
}

/// Aktuel reset-tilstand, genlæst efter workspace-persist. En close kan have
/// observeret registryet før en samtidig close detached sit sidste kort; den
/// oprindelige bool må derfor ikke bruges som et stale AND-led bagefter.
pub(crate) fn sequence_reset_ready() -> Result<bool, String> {
    with_registry(|inner| sequence_reset_state(inner))
}

/// Nummer-/navne-allokator for genererede `card-N`-kort — delt af terminal-
/// og browser-create (samme nummerserie, spec §3). Laveste ledige nummer
/// genbruges (ejer-beslutning 2026-07-20): et nummer er optaget, hvis et
/// levende kort baerer det (uanset navn), ELLER hvis navnet `card-N` er
/// taget (seedet kort, der tilfaeldigvis hedder `card-N`).
fn allocate_generated(inner: &mut Inner) -> (u32, String) {
    let used: HashSet<u32> = inner.numbers.values().copied().collect();
    let mut number = 1u32;
    loop {
        let name = format!("card-{number}");
        if !used.contains(&number) && !inner.cards.contains_key(&name) {
            return (number, name);
        }
        number += 1;
    }
}

/// Laveste ledige NUMMER alene (seedede kort beholder deres toml-navn og
/// reserverer derfor intet `card-N`-navn — kun nummeret).
fn lowest_free_number(inner: &Inner) -> u32 {
    let used: HashSet<u32> = inner.numbers.values().copied().collect();
    (1u32..)
        .find(|n| !used.contains(n))
        .expect("u32-rummet er aldrig fuldt")
}

/// Snapshot af et korts CardInfo — kaldes med KORT-laasen (aldrig map-laasen).
fn card_info(card: &CardRuntime) -> CardInfo {
    match &card.backend {
        CardBackend::Terminal(t) => CardInfo {
            number: card.number,
            name: t.config.name.clone(),
            cwd: t.config.cwd.display().to_string(),
            profile: t.profile.clone(),
            running: t.pty.is_some(),
            exited: card.exited,
            restore_action: t.restore_action.clone(),
            kind: "terminal".into(),
            opened_by: None,
            url: None,
            title: None,
            thread_id: None,
            purpose: None,
        },
        CardBackend::Browser(b) => CardInfo {
            number: card.number,
            name: b.name.clone(),
            cwd: String::new(),
            profile: String::new(),
            running: b.alive,
            exited: card.exited,
            restore_action: None,
            kind: "browser".into(),
            opened_by: b.opened_by.clone(),
            url: Some(b.url.clone()),
            title: Some(b.title.clone()),
            thread_id: None,
            purpose: None,
        },
        CardBackend::Chat(c) => CardInfo {
            number: card.number,
            name: c.name.clone(),
            cwd: String::new(),
            profile: String::new(),
            running: true,
            exited: card.exited,
            restore_action: None,
            kind: "chat".into(),
            opened_by: None,
            url: None,
            title: None,
            thread_id: Some(c.thread_id.clone()),
            purpose: Some(c.purpose.clone()),
        },
    }
}

pub fn create_card(
    cwd: String,
    profile: String,
    command: Option<String>,
) -> Result<CardInfo, String> {
    // Beskrivende fejlflader FOER der roeres taeller/map (invarianterne).
    if !Path::new(&cwd).is_dir() {
        return Err(format!("cwd not found: {cwd}"));
    }
    // CC-only (laast ejer-beslutning): kun "claude"-profilen findes.
    let prof = profiles::profile(&profile).ok_or_else(|| format!("unknown profile: {profile}"))?;
    // Browser-cards Task 6-flag: custom commands faar aldrig MCP-injektion.
    let custom_command = command.is_some();
    // Resume-binding (bindende): profil-kort resumer med profilens
    // resume_command; custom-kort resumer med SAMME command (ingen
    // --continue-semantik). Custom-kommandoen whitespace-splittes til
    // CardConfig's Vec<String>-form (cards.toml-formens argv — MVP-valg,
    // ingen quoting; journalfoert).
    let (command, resume_command): (Vec<String>, Vec<String>) = match command {
        Some(raw) => {
            let argv: Vec<String> = raw.split_whitespace().map(str::to_string).collect();
            if argv.is_empty() {
                return Err("command must not be empty".to_string());
            }
            (argv.clone(), argv)
        }
        None => (
            prof.spawn_command.iter().map(|s| s.to_string()).collect(),
            prof.resume_command.iter().map(|s| s.to_string()).collect(),
        ),
    };
    with_registry(move |inner| {
        let (number, name) = allocate_generated(inner);
        let runtime = CardRuntime {
            number,
            active: true,
            exited: None,
            backend: CardBackend::Terminal(TerminalRuntime {
                config: CardConfig {
                    name: name.clone(),
                    cwd: PathBuf::from(&cwd),
                    command,
                    resume_command,
                },
                profile: profile.clone(),
                pty: None,
                submit_readiness: None,
                attention: None,
                gate: EpochGate::new(),
                visible: Arc::new(AtomicBool::new(true)),
                restore_action: None,
                custom_command,
                accepts_from: AcceptsFrom::default(),
            }),
        };
        let info = card_info(&runtime);
        inner.generated_names.insert(name.clone());
        inner.numbers.insert(name.clone(), number);
        inner.cards.insert(name, Arc::new(Mutex::new(runtime)));
        info
    })
}

/// Browser-kort (plan Task 2): samme nummer-/navne-allokator som terminal-
/// kort. Foedes alive med tom titel; webview-oprettelsen er main.rs-lagets
/// ansvar (Task 5) — fejler den, kalder laget close_card for at rydde op.
pub fn create_browser_card(
    opened_by: Option<String>,
    scope_key: String,
    url: String,
    target_id: String,
) -> Result<CardInfo, String> {
    with_registry(move |inner| {
        let (number, name) = allocate_generated(inner);
        let runtime = CardRuntime {
            number,
            active: true,
            exited: None,
            backend: CardBackend::Browser(BrowserRuntime {
                name: name.clone(),
                opened_by,
                scope_key,
                url,
                title: String::new(),
                alive: true,
                target_id,
            }),
        };
        let info = card_info(&runtime);
        inner.generated_names.insert(name.clone());
        inner.numbers.insert(name.clone(), number);
        inner.cards.insert(name, Arc::new(Mutex::new(runtime)));
        info
    })
}

/// Chat-kort (spec §6): samme nummer-/navne-allokator som terminal- og
/// browser-kort. Foedes `running: true` - kortet lever saa laenge traaden
/// findes, og traadens egen `state` baerer om udvekslingen er lukket.
pub fn create_chat_card(thread_id: &str, purpose: &str) -> Result<CardInfo, String> {
    let thread_id = thread_id.to_string();
    let purpose = purpose.to_string();
    with_registry(move |inner| {
        let (number, name) = allocate_generated(inner);
        let runtime = CardRuntime {
            number,
            active: true,
            exited: None,
            backend: CardBackend::Chat(ChatRuntime {
                name: name.clone(),
                thread_id,
                purpose,
            }),
        };
        let info = card_info(&runtime);
        inner.generated_names.insert(name.clone());
        inner.numbers.insert(name.clone(), number);
        inner.cards.insert(name, Arc::new(Mutex::new(runtime)));
        info
    })
}

/// Laeser thread_id fra en Chat-backend; None for terminal- og browser-kort.
pub fn chat_thread_id(name: &str) -> Option<String> {
    let handle = card_handle(name).ok()?;
    let guard = handle.lock().unwrap_or_else(|p| p.into_inner());
    match &guard.backend {
        CardBackend::Chat(c) => Some(c.thread_id.clone()),
        CardBackend::Terminal(_) => None,
        CardBackend::Browser(_) => None,
    }
}

/// Signal om at BACKENDEN selv har ændret kortlisten.
///
/// Fladen **poller ikke**: den henter kortlisten ved mount og derefter kun paa
/// `browser-card-updated`/`browser-card-dead`. Kort som en AGENT skaber —
/// `card_pair`s partner og chat-kort — havde derfor ingen vej ind og laa
/// usynlige indtil ejeren tilfaeldigvis selv oprettede et kort. Det er denne
/// seam's eneste formaal; `main.rs` leverer emitten, som med de oevrige seams.
type CardsChanged = Box<dyn Fn() + Send + Sync + 'static>;

static CARDS_CHANGED: LazyLock<Mutex<Option<CardsChanged>>> = LazyLock::new(|| Mutex::new(None));

pub fn set_cards_changed(sink: CardsChanged) {
    *CARDS_CHANGED.lock().unwrap_or_else(|p| p.into_inner()) = Some(sink);
}

/// Kaldes ALDRIG med kort- eller registry-laase holdt (laaseorden §5.6).
pub fn notify_cards_changed() {
    let sink = CARDS_CHANGED.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(sink) = sink.as_ref() {
        sink();
    }
}

/// Arvet cwd til `card_pair` (spec §3.5, trin 2). None for ikke-terminale kort.
pub fn card_cwd(card: &str) -> Option<String> {
    let handle = card_handle(card).ok()?;
    let guard = handle.lock().unwrap_or_else(|p| p.into_inner());
    guard
        .terminal()
        .map(|terminal| terminal.config.cwd.display().to_string())
}

/// Opdaterer browser-kortets spejl-tilstand (navigations-/titel-events og
/// reconciliation-polleren, Task 5). None-felter roeres ikke. `target_id`
/// saettes af browser_host's CDP-ready-loop lige efter webview-oprettelsen.
pub fn update_browser_card(
    name: &str,
    url: Option<String>,
    title: Option<String>,
    alive: Option<bool>,
    target_id: Option<String>,
) -> Result<(), String> {
    let handle = card_handle(name)?;
    let mut card = handle.lock().map_err(|e| e.to_string())?;
    let name_owned = card.name().to_string();
    let Some(browser) = card.browser_mut() else {
        return Err(format!("card is not a browser: {name_owned}"));
    };
    if let Some(url) = url {
        browser.url = url;
    }
    if let Some(title) = title {
        browser.title = title;
    }
    if let Some(alive) = alive {
        browser.alive = alive;
    }
    if let Some(target_id) = target_id {
        browser.target_id = target_id;
    }
    Ok(())
}

/// Kaskade-ekspansion (spec §3): input-navnene + alle browser-kort hvis
/// `opened_by` matcher et input-navn. Ren navnelogik — deterministisk:
/// dedupet input-raekkefoelge foerst, kaskade-tilfoejelser sorteret pr.
/// nummer. Kalderen (main.rs' close-veje) sender resultatet til
/// close_cards_persisted.
pub fn expand_close_targets(names: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut result: Vec<String> = Vec::new();
    for name in names {
        if seen.insert(name.clone()) {
            result.push(name);
        }
    }
    let input_len = result.len();
    let mut cascade: Vec<(u32, String)> = Vec::new();
    for handle in all_handles() {
        let Ok(card) = handle.lock() else { continue };
        if let CardBackend::Browser(b) = &card.backend {
            let Some(owner) = &b.opened_by else { continue };
            if result[..input_len].iter().any(|name| name == owner) && !seen.contains(&b.name) {
                cascade.push((card.number, b.name.clone()));
            }
        }
    }
    cascade.sort_by_key(|(number, _)| *number);
    for (_, name) in cascade {
        if seen.insert(name.clone()) {
            result.push(name);
        }
    }
    result
}

/// Read-only snapshot used by browser_host's two-phase close path. A browser
/// webview must be closed successfully before `close_cards` detaches the same
/// card from the registry; otherwise the native child can become an orphan
/// that the orchestrator can no longer address.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserCloseTarget {
    pub name: String,
    pub scope_key: String,
    pub alive: bool,
}

/// Resolve only the live registry entries that are browser cards, preserving
/// the first occurrence of each requested name. Unknown and terminal names are
/// deliberately skipped here: the subsequent `close_cards` call remains the
/// single source of their established error/PTY semantics.
///
/// This function never detaches or deactivates a card. That read-only property
/// is the first half of browser_host's close transaction.
pub fn browser_close_targets(names: &[String]) -> Result<Vec<BrowserCloseTarget>, String> {
    let mut seen = HashSet::new();
    let handles = with_registry(|inner| {
        names
            .iter()
            .filter(|name| seen.insert((*name).clone()))
            .filter_map(|name| {
                inner
                    .cards
                    .get(name)
                    .cloned()
                    .map(|handle| (name.clone(), handle))
            })
            .collect::<Vec<_>>()
    })?;

    let mut targets = Vec::new();
    for (_name, handle) in handles {
        // Poison-recovery (read-only snapshot): en panic i en anden traad maa
        // ikke goere hele close-batchen umulig — close_cards recoverer selv
        // poisonede kort-mutexer laengere nede ad samme grund.
        let card = match handle.lock() {
            Ok(card) => card,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !card.active {
            continue;
        }
        if let CardBackend::Browser(browser) = &card.backend {
            targets.push(BrowserCloseTarget {
                name: browser.name.clone(),
                scope_key: browser.scope_key.clone(),
                alive: browser.alive,
            });
        }
    }
    Ok(targets)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CloseCardsError {
    pub name: String,
    pub message: String,
}

/// Browser-kort detached i close-batchens registry-fase. browser_host har
/// allerede accepteret native webview-close i preclose-fasen; metadataet
/// bruges bagefter til UI-event/scope-bookkeeping. `serde(skip)` bevarer
/// CloseCardsResults eksisterende wire-form.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BrowserClosed {
    pub name: String,
    pub scope_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CloseCardsResult {
    /// Kort der blev detached fra registryet. Et kort bliver staaende her,
    /// selv hvis dets efterfoelgende PTY-teardown rapporterer en fejl.
    pub closed: Vec<String>,
    pub errors: Vec<CloseCardsError>,
    /// Sand naar HELE batchens teardowns efterlod canvas uden genererede
    /// kort. Seedede kort (fx `master`) taeller ikke. Rent informativ siden
    /// laveste-ledige-allokatoren (numre genbruges ogsaa uden reset);
    /// wire-formen er bevaret.
    pub sequence_reset: bool,
    /// Browser-lifecycle er browser_host-lagets ansvar — registry roerer
    /// aldrig AppHandle (plan Task 2-kontrakten).
    #[serde(skip)]
    pub browser_closed: Vec<BrowserClosed>,
}

/// Batch-close i normativ lock-raekkefoelge:
/// 1) dedupe + remove under EN kort map-laas,
/// 2) slip map-laasen, markér runtimes inactive og tag deres PTY'er,
/// 3) teardown PTY'erne parallelt uden registry-/kort-laase,
/// 4) rapportér `sequence_reset` (canvas tom for genererede kort) FOERST
///    naar alle teardown-traade er joined.
///
/// Browser-kort har ingen PTY-teardown: de markeres inactive/doede og
/// rapporteres i `browser_closed` til browser_host-lagets post-close-fase.
///
/// Ukendte navne og teardown-fejl rapporteres struktureret; alle oevrige
/// targets fortsaetter. Resultatets raekkefoelge foelger inputtets foerste
/// forekomst deterministisk.
pub fn close_cards(names: Vec<String>) -> Result<CloseCardsResult, String> {
    #[cfg(feature = "perf-trace")]
    let total_started = Instant::now();
    #[cfg(feature = "perf-trace")]
    let trace_context = crate::perf_trace::capture_context();
    let mut seen = HashSet::new();
    let unique: Vec<String> = names
        .into_iter()
        .filter(|name| seen.insert(name.clone()))
        .collect();

    #[cfg(feature = "perf-trace")]
    let detach_started = Instant::now();
    let (removed, mut errors) = with_registry(|inner| {
        let mut removed = Vec::new();
        let mut errors = Vec::new();
        for name in &unique {
            match inner.cards.remove(name) {
                Some(handle) => {
                    inner.generated_names.remove(name);
                    inner.numbers.remove(name);
                    removed.push((name.clone(), handle));
                }
                None => errors.push(CloseCardsError {
                    name: name.clone(),
                    message: format!("no such card: {name}"),
                }),
            }
        }
        (removed, errors)
    })?;
    crate::perf_mark!(
        "close.registry_detach.end",
        serde_json::json!({
            "duration_ms": detach_started.elapsed().as_secs_f64() * 1_000.0,
            "requested_count": unique.len(),
            "removed_count": removed.len(),
            "initial_error_count": errors.len(),
        }),
    );

    let mut closed = Vec::with_capacity(removed.len());
    let mut browser_closed = Vec::new();
    let mut chat_threads = HashMap::new();
    let mut teardown_jobs = Vec::new();
    for (name, handle) in removed {
        closed.push(name.clone());
        let (mut card, poison_message) = match handle.lock() {
            Ok(card) => (card, None),
            // Selv ved en tidligere panic skal active/visible slukkes og
            // PTY'en tages; ellers kunne et nyt kort genbruge navnet, mens
            // den forgiftede runtime stadig emitter.
            Err(poisoned) => (
                poisoned.into_inner(),
                Some("card state lock was poisoned".to_string()),
            ),
        };
        if let Some(message) = poison_message {
            errors.push(CloseCardsError {
                name: name.clone(),
                message,
            });
        }
        card.active = false;
        match &mut card.backend {
            CardBackend::Terminal(terminal) => {
                // Readeren maa gerne draene, men dens gamle kortnavn maa ikke
                // emitte under/efter et sequence-reset.
                terminal.visible.store(false, Ordering::Relaxed);
                if let Some(readiness) = terminal.submit_readiness.take() {
                    readiness.cancel();
                }
                // Fjernes praecis samme sted som readiness: et lukket kort maa
                // ikke kunne holde workspacets opmaerksomheds-prik taendt.
                terminal.attention = None;
                if let Some(host) = terminal.pty.take() {
                    let job_name = name.clone();
                    #[cfg(feature = "perf-trace")]
                    let job_context = trace_context.clone();
                    teardown_jobs.push((
                        name,
                        std::thread::spawn(move || {
                            crate::perf_with_context!(job_context, {
                                #[cfg(feature = "perf-trace")]
                                let started = Instant::now();
                                crate::perf_mark!(
                                    "close.pty_teardown.begin",
                                    serde_json::json!({ "name": job_name }),
                                );
                                let result =
                                    host.kill_and_teardown().map_err(|e| CloseCardsError {
                                        name: job_name.clone(),
                                        message: e.to_string(),
                                    });
                                crate::perf_mark!(
                                    "close.pty_teardown.end",
                                    serde_json::json!({
                                        "name": job_name,
                                        "duration_ms": started.elapsed().as_secs_f64() * 1_000.0,
                                        "ok": result.is_ok(),
                                    }),
                                );
                                result
                            })
                        }),
                    ));
                }
            }
            CardBackend::Browser(browser) => {
                browser.alive = false;
                browser_closed.push(BrowserClosed {
                    name,
                    scope_key: browser.scope_key.clone(),
                });
            }
            CardBackend::Chat(chat) => {
                // Intet at rive ned: hverken PTY eller webview. Traad-id'et
                // opsamles her, fordi kortet er fjernet fra registryet naar
                // livscyklus-hooket tilfoejes i T11.
                chat_threads.insert(name.clone(), chat.thread_id.clone());
            }
        }
    }

    // Join i input-raekkefoelge giver deterministiske fejl, mens selve
    // teardown-arbejdet stadig koerer parallelt.
    #[cfg(feature = "perf-trace")]
    let teardown_count = teardown_jobs.len();
    #[cfg(feature = "perf-trace")]
    let join_started = Instant::now();
    for (name, job) in teardown_jobs {
        match job.join() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => errors.push(error),
            Err(_) => errors.push(CloseCardsError {
                name,
                message: "PTY teardown thread panicked".to_string(),
            }),
        }
    }
    crate::perf_mark!(
        "close.pty_batch_join.end",
        serde_json::json!({
            "duration_ms": join_started.elapsed().as_secs_f64() * 1_000.0,
            "teardown_count": teardown_count,
        }),
    );

    // Ingen taeller at nulstille laengere (laveste-ledige-allokatoren) —
    // feltet rapporterer stadig "canvas blev tom for genererede kort" paa
    // den eksisterende wire-form.
    let sequence_reset = sequence_reset_ready()?;

    let result = CloseCardsResult {
        closed,
        errors,
        sequence_reset,
        browser_closed,
    };

    // Traad-hooket til sidst: kortet er detacheret, PTY'en revet ned og
    // kort-laasen sluppet. Kald aldrig threads::* med en kort-laas (spec §5.6).
    // Kun `closed` itereres — et ukendt navn i `errors` har ingen traade.
    for name in &result.closed {
        let chat_thread = chat_threads.get(name.as_str()).cloned();
        crate::threads::on_card_gone(name, chat_thread.as_deref());
    }

    crate::perf_mark!(
        "close.registry_close.end",
        serde_json::json!({
            "duration_ms": total_started.elapsed().as_secs_f64() * 1_000.0,
            "closed_count": result.closed.len(),
            "error_count": result.errors.len(),
            "browser_count": result.browser_closed.len(),
            "pty_count": teardown_count,
        }),
    );
    Ok(result)
}

/// Bagudkompatibel single-close; bruger batch-corens locking, teardown og
/// reset-semantik, men bevarer den gamle `Result<(), String>`-fejlflade.
pub fn close_card(name: String) -> Result<(), String> {
    let result = close_cards(vec![name.clone()])?;
    if let Some(error) = result.errors.into_iter().find(|error| error.name == name) {
        return Err(error.message);
    }
    Ok(())
}

/// Sorteret pr. nummer (deterministisk). Map-laasen slippes FOER
/// kort-laasene tages (fix F2).
pub fn list_cards() -> Vec<CardInfo> {
    let handles = all_handles();
    let mut infos: Vec<CardInfo> = handles
        .iter()
        .filter_map(|h| h.lock().ok().map(|c| card_info(&c)))
        .collect();
    infos.sort_by_key(|c| c.number);
    infos
}

/// Slaar et korts runtime-handle op ved navn — map-laasen SLIPPES igen foer
/// retur (fix F2); kalderen laaser derefter selv kortet. Fejlteksten
/// ("unknown card") er de eksisterende kommandoers uaendrede fejlflade.
pub fn card_handle(name: &str) -> Result<Arc<Mutex<CardRuntime>>, String> {
    with_registry(|inner| inner.cards.get(name).cloned())?
        .ok_or_else(|| format!("unknown card: {name}"))
}

pub fn accepts_from(card: &str) -> Option<AcceptsFrom> {
    let handle = card_handle(card).ok()?;
    let guard = handle.lock().unwrap_or_else(|p| p.into_inner());
    guard.terminal().map(|t| t.accepts_from.clone())
}

/// Traad-lagets politikport. Der findes bevidst intet agent-tool der muterer
/// politikken; kun parring og terminalisering maa aendre den.
pub struct RegistryPolicyPort;

impl crate::threads::policy::Port for RegistryPolicyPort {
    fn read(&self, card: &str) -> crate::threads::policy::AcceptsFromView {
        use crate::threads::policy::AcceptsFromView as V;
        match accepts_from(card) {
            None => V::Nobody,
            Some(AcceptsFrom::Nobody) => V::Nobody,
            Some(AcceptsFrom::HumanOnly) => V::HumanOnly,
            Some(AcceptsFrom::Any) => V::Any,
            Some(AcceptsFrom::List(list)) => V::List(list),
        }
    }

    fn pair(&self, a: &str, b: &str) -> Result<(), String> {
        for (target, peer) in [(a, b), (b, a)] {
            mutate_policy(target, |policy| match policy {
                AcceptsFrom::List(list) => {
                    if !list.iter().any(|card| card == peer) {
                        list.push(peer.to_string());
                    }
                }
                other => *other = AcceptsFrom::List(vec![peer.to_string()]),
            })?;
        }
        Ok(())
    }

    fn revoke(&self, a: &str, b: &str) {
        for (target, peer) in [(a, b), (b, a)] {
            let _ = mutate_policy(target, |policy| {
                if let AcceptsFrom::List(list) = policy {
                    list.retain(|card| card != peer);
                    if list.is_empty() {
                        *policy = AcceptsFrom::default();
                    }
                }
            });
        }
    }
}

fn mutate_policy(card: &str, f: impl FnOnce(&mut AcceptsFrom)) -> Result<(), String> {
    let handle = card_handle(card)?;
    let mut guard = handle.lock().unwrap_or_else(|p| p.into_inner());
    let terminal = guard
        .terminal_mut()
        .ok_or_else(|| format!("card is not a terminal: {card}"))?;
    f(&mut terminal.accepts_from);
    Ok(())
}

/// Alle korts runtime-handles (map-laasen slippes foer retur, fix F2) —
/// get_cards og app-exit-teardownen i main.rs itererer denne.
pub fn all_handles() -> Vec<Arc<Mutex<CardRuntime>>> {
    with_registry(|inner| inner.cards.values().cloned().collect()).unwrap_or_default()
}

/// Seed af et eksisterende cards.toml-kort. Navnet BEHOLDES (grid-kompat);
/// supervisionens `master` lever uden for generated-nummersekvensen.
/// Navne-kollision -> `Err("duplicate card name: …")` —
/// case-insensitivt som cards.rs' validering (Windows-fs).
pub fn seed_card(config: CardConfig, profile: &str) -> Result<CardInfo, String> {
    let profile = profile.to_string();
    with_registry(move |inner| {
        if inner
            .cards
            .keys()
            .any(|k| k.eq_ignore_ascii_case(&config.name))
        {
            return Err(format!("duplicate card name: {}", config.name));
        }
        // Supervisionens master lever uden for workspace/generationerne og
        // maa aldrig optage card-1's nummer.
        let number = if config.name.eq_ignore_ascii_case(NON_GENERATED_MASTER_NAME) {
            0
        } else {
            lowest_free_number(inner)
        };
        let name = config.name.clone();
        let runtime = CardRuntime {
            number,
            active: true,
            exited: None,
            backend: CardBackend::Terminal(TerminalRuntime {
                config,
                profile,
                pty: None,
                submit_readiness: None,
                attention: None,
                gate: EpochGate::new(),
                visible: Arc::new(AtomicBool::new(true)),
                restore_action: None,
                custom_command: false,
                accepts_from: AcceptsFrom::default(),
            }),
        };
        let info = card_info(&runtime);
        inner.numbers.insert(name.clone(), number);
        inner.cards.insert(name, Arc::new(Mutex::new(runtime)));
        Ok(info)
    })?
}

/// Task 11: saetter restore-badgen ("resume" | "fresh_shared_cwd" | "fresh").
/// Kaldes KUN fra main.rs' app-start-sti (restore-planen) — kort oprettet i
/// sessionens loeb beholder None. Fejlflade som resten af registryet
/// ("unknown card") for kort der ikke blev seedet (fx defekt workspace-linje).
pub fn set_restore_action(name: &str, action: &str) -> Result<(), String> {
    let handle = card_handle(name)?;
    let mut card = handle.lock().map_err(|e| e.to_string())?;
    let name_owned = card.name().to_string();
    let Some(terminal) = card.terminal_mut() else {
        return Err(format!("card is a browser: {name_owned}"));
    };
    terminal.restore_action = Some(action.to_string());
    Ok(())
}

// ---------------------------------------------------------------------------
// Task 8: Rust-side output-gating pr. kort.
// ---------------------------------------------------------------------------

/// Saetter kortets synligheds-flag (socket-kontrakten fra Task 1 Del B —
/// main.rs-wrapperen er tynd). Virker uafhaengigt af running-state: LOD-laget
/// (Task 10) kalder den ogsaa for u-spawnede kort; flaget bor paa
/// TerminalRuntime, ikke paa pty'en. Reader-closuren ser flippet ved naeste
/// chunk (Relaxed er nok: rent flag, ingen data-afhaengig ordering).
/// Browser-kort har ingen PTY-emit at gate — webview-synlighed styres af
/// main.rs-lagets occlusion-/fuldskaerms-komposition (Task 5/7).
pub fn set_card_visible(name: String, visible: bool) -> Result<(), String> {
    let handle = card_handle(&name)?;
    let card = handle.lock().map_err(|e| e.to_string())?;
    let Some(terminal) = card.terminal() else {
        return Err(format!("card is a browser: {name}"));
    };
    terminal.visible.store(visible, Ordering::Relaxed);
    Ok(())
}

/// Den DELTE suppressions-beslutning (Testbarhed-reglen): wrapper en
/// emit-closure saa den kun kaldes naar kortet er synligt. main.rs'
/// reader-emit OG tests/gating.rs bruger PRAECIS denne funktion — chunks
/// droppes ved visible=false (FUND 2: alt-screen har ingen historik at
/// miste), mens pty.rs' read-loekke draener uaendret (backpressure maa
/// aldrig ramme child'en — derfor sidder gaten i emitten, ikke i laesningen).
pub fn gated_emit(
    visible: Arc<AtomicBool>,
    emit: impl Fn(&[u8]) + Send + 'static,
) -> impl Fn(&[u8]) + Send + 'static {
    move |bytes: &[u8]| {
        if visible.load(Ordering::Relaxed) {
            emit(bytes);
        }
    }
}
