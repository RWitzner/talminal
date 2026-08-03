//! Webview-livscyklus for browser-kort (spec §4/§5/§8a). AppHandle-laget:
//! registry.rs + browser.rs er REN logik — ALT der roerer tauri-webviews bor
//! her.
//!
//! Traad-disciplin (spike S1, normativ): webview-OPRETTELSE (`add_child`)
//! deadlocker Windows hvis den koeres synkront paa main-traaden. Derfor
//! oprettes webviews ALTID via `run_on_main_thread` + kanal ([`on_main`]) fra
//! en OFF-main-traad (kommandoerne koerer paa `run_blocking`-poolen; MCP-
//! serveren + polleren paa egne std-traade). [`on_main`] maa ALDRIG kaldes fra
//! selve main-traaden (den ville poste et callback bag sin egen recv og
//! deadlocke). De lette praesentations-kommandoer (`set_browser_*`,
//! `focus_browser_card`) er derimod SYNKRONE tauri-kommandoer — de koerer
//! netop PAA main og roerer eksisterende webviews DIREKTE (ingen `add_child`,
//! ingen reentrancy), saa de springer kanal-hoppet over.
//!
//! HTTP mod CDP (`/json`, `/json/close/{id}`) er en minimal synkron std-net-
//! klient: reqwest i cratet er async-only, og kalderne her (`run_blocking`-
//! poolen, MCP-traaden, poller-traaden) er synkrone uden en tokio-runtime at
//! blocke paa (tokio-featuren mangler `rt`). En localhost-CDP-GET er
//! deterministisk nok til at haandrulle.
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::Duration;
#[cfg(feature = "perf-trace")]
use std::time::Instant;

use serde::Serialize;
use serde_json::Value;
use tauri::webview::WebviewBuilder;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl};

use crate::{browser, mcp, registry};

/// Global praesentationstilstand (spec §8a): occlusion-set'et ejes af
/// frontenden (én bool over IPC); fuldskaerm ét kort ad gangen; bounds
/// rapporteres pr. kort af frontenden.
struct Presentation {
    occluded: bool,
    fullscreen: Option<String>,
    /// Sidste kendte bounds pr. kort-navn (gendannelse efter fuldskaerm/occlusion).
    bounds: HashMap<String, (f64, f64, f64, f64)>,
}
static PRESENTATION: LazyLock<Mutex<Presentation>> = LazyLock::new(|| {
    Mutex::new(Presentation {
        occluded: false,
        fullscreen: None,
        bounds: HashMap::new(),
    })
});

/// Serialiserer kort-oprettelsen saa CDP-target-diff'en (before/after mod
/// samme scope-port) aldrig kan forveksle to samtidigt oprettede kort.
/// `into_inner` ved poison: en tidligere panic maa ikke laase al oprettelse.
static CREATE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Browser-close er en tofaset transaktion (native webview foerst, registry
/// bagefter). Kun OVERLAPPENDE target-saet maa serialiseres: uafhaengige kort
/// kan lukkes parallelt, mens samme kort stadig er laast fra native preclose
/// til registry-detach/finish. Weak-vaerdier undgaar en voksende navne-cache;
/// baade map- og target-poison recoveres, saa et panic ikke lammer senere close.
static CLOSE_TARGET_LOCKS: LazyLock<Mutex<HashMap<String, Weak<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Returnerer target-laase i kanonisk raekkefoelge. Sortering + dedupe er
/// deadlock-vaernet for overlappende batches med omvendt input-raekkefoelge.
fn close_target_locks(names: &[String]) -> Vec<Arc<Mutex<()>>> {
    let mut keys = names.to_vec();
    keys.sort();
    keys.dedup();

    let mut lock_map = CLOSE_TARGET_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    lock_map.retain(|_, lock| lock.strong_count() > 0);
    keys.into_iter()
        .map(|key| {
            if let Some(lock) = lock_map.get(&key).and_then(Weak::upgrade) {
                return lock;
            }
            let lock = Arc::new(Mutex::new(()));
            lock_map.insert(key, Arc::downgrade(&lock));
            lock
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Event-payloads (frontend-wire, bindende for Task 7/9)
// ---------------------------------------------------------------------------

#[derive(Clone, Serialize)]
struct BrowserCardUpdated {
    name: String,
    url: String,
    title: String,
}

#[derive(Clone, Serialize)]
struct BrowserCardDead {
    names: Vec<String>,
}

// ---------------------------------------------------------------------------
// Rene funktioner (unit-testet i tests/browser_lifecycle.rs)
// ---------------------------------------------------------------------------

/// Origins der tilhoerer APPEN selv, som `host` eller `host:port`.
///
/// **Gate 2.** Browser-kort er child-webviews inde i vinduet `main`
/// (`add_child` nedenfor), og Tauri injicerer `__TAURI_INTERNALS__` UBETINGET
/// i hver child-webview — der er ingen External-gate i injektionen. Det der
/// redder os er Tauris egen remote-origin-check: en fremmed side faar
/// `acl = None` og afvises ved kommando-dispatch.
///
/// Men capability'en er VINDUE-targetet (`capabilities/default.json`:
/// `"windows": ["main"]`, ingen `webviews`-noegle), saa hele graensen hviler
/// paa én ting: at et browser-kort aldrig staar paa appens egen origin. Sker
/// det, er kortet `is_local` og faar hele `generate_handler!`-fladen — en
/// eskalering FORBI PTY-graensen, som modsiger SECURITY.md's loefte om at et
/// kort giver samme adgang som en terminal.
///
/// Dev-originen er med med vilje: `is_local_url` i tauri matcher ogsaa alt
/// relativt til `get_app_url()`, som i dev er `build.devUrl` — og dev-bygget
/// er praecis det CONTRIBUTING beder enhver bidragyder koere.
/// `dev_origin_constant_matches_tauri_conf` haandhaever at listen ikke drifter.
pub const APP_ORIGINS: &[&str] = &[
    "tauri.localhost",
    "localhost:1420",
    "127.0.0.1:1420",
    "[::1]:1420",
];

/// Sammenlignings-noegle for en URL: `host` eller `host:port`, lowercased og
/// uden trailing dot.
///
/// Normaliseringen er ikke pedanteri — `TAURI.LOCALHOST` og `tauri.localhost.`
/// rammer samme vaert i en browser, saa en ren streng-sammenligning ville
/// vaere en aaben doer.
fn origin_key(u: &url::Url) -> String {
    let host = u
        .host_str()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    match u.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    }
}

/// Er URL'en appens egen origin? Se `APP_ORIGINS`.
pub fn is_app_origin(u: &url::Url) -> bool {
    let key = origin_key(u);
    APP_ORIGINS.iter().any(|o| *o == key)
}

/// Navigations-politikken som en ren funktion, saa den kan testes.
///
/// Den var foer en anonym closure inde i `create_card_webview`, altsaa kun
/// naabar gennem `WebviewBuilder` — og dermed utestbar. Politikken og
/// sideeffekten (registry-opdatering + event) er nu adskilt: closuren
/// beslutter intet selv.
pub fn navigation_allowed(u: &url::Url) -> bool {
    match u.scheme() {
        "http" | "https" => !is_app_origin(u),
        // about:blank er den interne blank-mekanisme; alt andet
        // (file:/javascript:/data: …) afvises (spec §4/§7).
        _ => u.as_str() == "about:blank",
    }
}

/// URL-politik (spec §4/§7): `None` ⇒ `about:blank` (intern blank-mekanisme);
/// ellers kun `http`/`https`, og aldrig appens egen origin (gate 2).
pub fn validate_card_url(url: Option<&str>) -> Result<String, String> {
    match url {
        None => Ok("about:blank".to_string()),
        Some(raw) => {
            let parsed: url::Url = raw
                .parse()
                .map_err(|_| format!("unsupported url scheme: {raw}"))?;
            match parsed.scheme() {
                "http" | "https" => {
                    if is_app_origin(&parsed) {
                        return Err(format!(
                            "appens egen origin kan ikke vaere et kort-maal: {raw}"
                        ));
                    }
                    Ok(raw.to_string())
                }
                other => Err(format!("unsupported url scheme: {other}")),
            }
        }
    }
}

/// Det foerste target i `after`, hvis id ikke fandtes i `before` — CDP-ready-
/// loopets kerne (S3b): kortets nye page-target er praecis diffen mod
/// snapshottet taget FOER `add_child`.
pub fn new_target_id(before: &[(String, String)], after: &[(String, String)]) -> Option<String> {
    after
        .iter()
        .find(|(id, _)| !before.iter().any(|(b, _)| b == id))
        .map(|(id, _)| id.clone())
}

/// Guard mod en TOM before-snapshot (final-review Finding 2). Er `before` tom,
/// findes intet paalideligt grundlag for [`new_target_id`]-diffen: den ville da
/// returnere det FOERSTE target i `after` (typisk keeperens `about:blank`) som
/// "det nye kort". Kortet blev saa tagget med keeperens target, mens dets
/// rigtige side saa "ukendt, ikke-blank" ud for polleren — der /json/close'r
/// den efter naadesvinduet og draeber siden agenten styrer. En tom
/// before-snapshot er derfor en hard fejl, ikke et diff-grundlag.
pub fn before_snapshot_ok(before: &[(String, String)]) -> Result<(), String> {
    if before.is_empty() {
        Err("cdp before-snapshot unavailable".to_string())
    } else {
        Ok(())
    }
}

/// Ejer-beslutning 2026-07-20 (amender enkelt-fejl-doedsvejen ovenfor):
/// scope-doed kraever 2 PAA HINANDEN FOELGENDE poll-fejl — én langsom
/// `/json`-laesning maa ikke masse-draebe et sundt scope. `failures` er
/// pollerens fejl-streak pr. scope-key (fjernes helt ved succes, saa en ny
/// fejl altid starter forfra som streak 1). Returnerer om scopet SKAL
/// behandles som doedt efter dette poll.
///
/// "Poll-fejl" er siden M2 (2026-07-29) bredere end en Err fra CDP: ogsaa et
/// SUCCESFULDT sample uden keeperens eget target taeller som fejl-tick, fordi
/// det modsiger sig selv ([`trusted_sample`]). Kalderen afgoer det ved at
/// sende `failed = trusted_sample(...).is_none()` — samme streak, samme
/// debounce, ét kald pr. tick.
///
/// `has_keeper` (P1-lazy-keeper-fund 2026-07-21): et scope uden registreret
/// keeper-target — lazy-vinduet mellem worker-spawn og foerste
/// `browser_card_open`, samt ready-vinduet FOER keeper-id-registreringen —
/// har et CDP-endpoint der FORVENTELIGT er nede; Err-ticks dér er ingen
/// fejl. Streaken RYDDES (ikke blot fryses), saa keeper-up altid starter
/// paa streak 0 og 2-fejls-debouncen reelt gaelder for scopets foerste
/// kort. Uden gaten akkumulerede lazy-vinduet ubegraenset (eneste clear
/// var et Ok-tick, som strukturelt aldrig kom), og EEN langsom
/// `/json`-laesning efter foerste open doedsmarkerede det friske kort.
pub fn track_scope_failure(
    failures: &mut HashMap<String, u32>,
    scope_key: &str,
    failed: bool,
    has_keeper: bool,
) -> bool {
    if !has_keeper || !failed {
        failures.remove(scope_key);
        return false;
    }
    let count = failures.entry(scope_key.to_string()).or_insert(0);
    *count += 1;
    *count >= 2
}

/// Identificerer keeperens target i et CDP-page-snapshot. Sentinel-match
/// (fragmentet fra [`browser::keeper_sentinel`]) vinder; fallback (hvis
/// `/json` stripper fragmentet) er praecis-én-side-reglen. Flere sider uden
/// sentinel er en HARD fejl — aldrig "gaet den foerste" (samme fejlklasse som
/// final-review Finding 2's mis-tagging). Kaldes altid under CREATE_LOCK, saa
/// ingen kort-oprettelse kan blande sig i snapshottet.
pub fn identify_keeper(pages: &[(String, String)], scope_key: &str) -> Result<String, String> {
    // Sentinel-match kraever about:blank-praefikset — en side som
    // `https://x/#keeper-{scope}` maa ikke kunne spoofe sig til
    // keeper-immunitet (review F3).
    let marker = format!("#keeper-{scope_key}");
    if let Some((id, _)) = pages
        .iter()
        .find(|(_, url)| url.starts_with("about:blank") && url.contains(&marker))
    {
        return Ok(id.clone());
    }
    match pages {
        [(only, _)] => Ok(only.clone()),
        [] => Err("cdp keeper snapshot empty".to_string()),
        _ => Err(format!(
            "cdp keeper ambiguous: {} pages and no sentinel match",
            pages.len()
        )),
    }
}

/// Modsiger et SUCCESFULDT `/json`-sample sig selv? Keeper-webviewen er selv
/// et page-target paa scopets port ([`identify_keeper`]), saa et svar der
/// mangler keeperens target — det TOMME sample inklusive — paastaar paa én
/// gang at browserprocessen svarer, OG at dens egen livline er vaek. Den slags
/// sample fortjener mistaenkeliggoerelse, ikke tillid.
///
/// Fundet (M2, 2026-07-29): vaernene laa kun i pollerens Err-arm
/// ([`track_scope_failure`]s 2-fejls-debounce + [`scope_dead_names`]s
/// naadesvindue), mens Ok-armen draebte paa ÉT sample — og oven i koebet
/// ryddede fejl-streaken foerst (`failed=false`). Ét tomt sample kunne derfor
/// masse-draebe et scope uden nogen debounce overhovedet. Keeper-loese samples
/// foeres nu ad den debouncede fejl-vej i stedet.
///
/// PARTIELLE samples (keeperen ER der, men ét korts target mangler) forbliver
/// AUTORITATIVE efter én observation — bevidst valg: samplet modsiger ikke sig
/// selv, keeperens tilstedevaerelse beviser at target-listen reelt blev
/// afleveret, og `/json` er ét atomart svar hvor et page-target enten findes
/// eller ikke findes. En ekstra observation ville koste 2 s ekstra foer "tab
/// lukket udefra" bliver til et doedt kort — og det er den ENESTE
/// doeds-detektor vi har (spike S7: ingen automatisk death-detection).
/// Mid-creation-kort er paa DEN vej allerede skaanet af tomt-target-reglen i
/// [`reconcile_actions`] (for et mistroet sample, se [`trusted_sample`]).
///
/// Et scope UDEN registreret keeper (lazy-vinduet, samme gate som
/// `has_keeper` i [`track_scope_failure`]) kan ikke modsige sig selv: uden id
/// findes intet at savne, og armen opfoerer sig praecis som foer.
pub fn sample_contradicts_keeper(pages: &[(String, String)], keeper_tid: Option<&str>) -> bool {
    let Some(keeper) = keeper_tid else {
        return false;
    };
    !pages.iter().any(|(id, _)| id.as_str() == keeper)
}

/// Pollerens ARM-VALG for ét scope-tick, som ÉN ren funktion. Det er denne
/// `reconcile_scope` faktisk kalder, saa regressionstests kan maale
/// produktionens klassifikation i stedet for en kopi af den (review-fund:
/// et testlokalt match-udtryk beviser kun at kopien er rigtig).
///
/// `Some(pages)` ⇒ samplet er autoritativt (Ok-armen). `None` ⇒ ticket
/// behandles som en poll-fejl med [`track_scope_failure`]s 2-fejls-debounce
/// — baade naar CDP svarede med fejl, OG naar et succesfuldt svar modsiger
/// sig selv ved at mangle keeperens eget target
/// ([`sample_contradicts_keeper`]).
///
/// ADFAERDSAENDRING (M2, 2026-07-29) der er vaerd at kende: et keeper-loest
/// sample gik FOER ad Ok-armen, hvor [`reconcile_actions`]' dead_names-filter
/// (`!tid.is_empty()`) skaanede mid-creation-kort UBETINGET; nu gaar det ad
/// fejl-armen, hvor de kun har dennes BUNDNE naade (`EMPTY_TID_GRACE_TICKS`
/// i [`scope_dead_names`]) — accepteret, fordi et evigt target-loest kort
/// ellers ville leve videre i praecis det scope vi har mistanke til.
pub fn trusted_sample(
    sample: Result<Vec<(String, String)>, String>,
    keeper_tid: Option<&str>,
) -> Option<Vec<(String, String)>> {
    match sample {
        Ok(pages) if !sample_contradicts_keeper(&pages, keeper_tid) => Some(pages),
        _ => None,
    }
}

/// Pollerens beslutninger for ét scope-tick (Ok-armen) — ren funktion, saa
/// keeper-immuniteten og naadesvinduerne kan regressionstestes uden CDP.
#[derive(Debug, Clone, PartialEq)]
pub struct ReconcileActions {
    /// Ukendte, ikke-blanke targets der skal `/json/close`s (2 sightings).
    pub close_targets: Vec<String>,
    /// Keeperen staar paa en ikke-blank URL (kapret af en agent der sprang
    /// `browser_card_open` over) — naviger den tilbage til sentinelen i
    /// stedet for at draebe den: et drab tager hele scopets browserproces.
    pub renavigate_keeper: bool,
    /// Kort hvis target er forsvundet (tab lukket eksternt) ⇒ doede. Kun
    /// meningsfuld for et sample der er sluppet gennem [`trusted_sample`] —
    /// kalderen maa aldrig regne dead_names ud af et sample uden keeperen.
    pub dead_names: Vec<String>,
}

/// Reglerne i raekkefoelge pr. page-target: (1) keeperens id er ALTID immunt
/// mod close — non-blank URL betyder renavigation, aldrig drab; (2) URLer der
/// begynder med `about:blank` er fritaget (playwrights friske blank-tabs +
/// keeper-sentinelen foer id-registrering); (3) oevrige ukendte lukkes efter
/// 2 paa hinanden foelgende sightings (`pending`-naadesvinduet).
pub fn reconcile_actions(
    cards: &[(String, String, bool)],
    pages: &[(String, String)],
    keeper_tid: Option<&str>,
    pending: &mut HashMap<String, u32>,
    seen_unknown: &mut HashSet<String>,
) -> ReconcileActions {
    let known: HashSet<&str> = cards
        .iter()
        .filter(|(_, tid, _)| !tid.is_empty())
        .map(|(_, tid, _)| tid.as_str())
        .collect();
    let page_ids: HashSet<&str> = pages.iter().map(|(id, _)| id.as_str()).collect();

    let mut close_targets = Vec::new();
    let mut renavigate_keeper = false;
    for (id, url) in pages {
        if keeper_tid == Some(id.as_str()) {
            if !url.starts_with("about:blank") {
                renavigate_keeper = true;
            }
            continue;
        }
        if url.starts_with("about:blank") || known.contains(id.as_str()) {
            continue;
        }
        seen_unknown.insert(id.clone());
        let count = pending.entry(id.clone()).or_insert(0);
        *count += 1;
        if *count >= 2 {
            close_targets.push(id.clone());
        }
    }

    let dead_names = cards
        .iter()
        .filter(|(_, tid, alive)| *alive && !tid.is_empty() && !page_ids.contains(tid.as_str()))
        .map(|(name, _, _)| name.clone())
        .collect();

    ReconcileActions {
        close_targets,
        renavigate_keeper,
        dead_names,
    }
}

/// Scope-doeds-armen (CDP svarer ikke — eller svarer succesfuldt UDEN
/// keeperens target, hvilket [`trusted_sample`] behandler ens — og
/// 2-fejls-debouncen er passeret): kort MED
/// target doer straks; kort UDEN target (mid-creation) faar et bounded
/// naadesvindue paa `max_grace_ticks` doeds-kvalificerede ticks, foer de
/// ogsaa doer — ubetinget skip ville lade et kort, hvis open-traad doede
/// mellem registry-create og target-opdatering, leve evigt.
pub fn scope_dead_names(
    cards: &[(String, String, bool)],
    grace: &mut HashMap<String, u32>,
    max_grace_ticks: u32,
) -> Vec<String> {
    let mut dead = Vec::new();
    for (name, tid, alive) in cards {
        if !alive {
            continue;
        }
        if !tid.is_empty() {
            dead.push(name.clone());
            continue;
        }
        let ticks = grace.entry(name.clone()).or_insert(0);
        *ticks += 1;
        if *ticks > max_grace_ticks {
            dead.push(name.clone());
        }
    }
    dead
}

/// Fast-path-probe + heal som testbar soem (WebView2-frie closures). Sund
/// probe (inden for retry-vinduet) ⇒ `Ok(false)` og INTET stadie roeres.
/// Doed probe ⇒ teardown → vent-paa-frigivelse → recreate; frigives zombie-
/// webview/port aldrig, er det en HARD fejl med runbook-anvisning — aldrig
/// stille degradering (praecedens: silent-CDP-timeout-fixet).
pub fn fastpath_heal_with(
    mut probe: impl FnMut() -> bool,
    probe_attempts: u32,
    probe_delay: Duration,
    teardown: impl FnOnce() -> Result<(), String>,
    wait_released: impl FnOnce() -> bool,
    recreate: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    for attempt in 0..probe_attempts {
        if attempt > 0 {
            std::thread::sleep(probe_delay);
        }
        if probe() {
            return Ok(false);
        }
    }
    teardown()?;
    if !wait_released() {
        return Err(
            "zombie browser webview/port was not released - close the scope's cards and respawn the worker"
                .to_string(),
        );
    }
    recreate()?;
    Ok(true)
}

/// Oprydning efter et fejlet `open_browser_card` — fjern registry-kortet og
/// emit DEREFTER. Ghost-kort-regressionen 2026-07-20: polleren kan have naaet
/// at doedsmarkere + emitte det halvfaerdige kort, saa frontenden HAR renderet
/// det; uden et afsluttende event efter fjernelsen staar dead-UI'et tilbage
/// for evigt. Emitten fyrer OGSAA ved close-fejl (kortet kan allerede vaere
/// vaek) — et event for meget er en no-op-refresh, et for lidt er et ghost.
pub fn cleanup_failed_open(name: &str, emit_dead: impl FnOnce(Vec<String>)) {
    cleanup_failed_open_with(|| registry::close_card(name.to_string()), emit_dead, name);
}

/// Testbar soem for [`cleanup_failed_open`]: close injiceret, saa ordningen
/// "fjernet FOER emit" kan regressionstestes uden registry (kort-nummer-
/// genbruget goer navnebaserede registry-asserts racy under parallel test).
pub fn cleanup_failed_open_with(
    close: impl FnOnce() -> Result<(), String>,
    emit_dead: impl FnOnce(Vec<String>),
    name: &str,
) {
    let _ = close();
    emit_dead(vec![name.to_string()]);
}

// ---------------------------------------------------------------------------
// Main-traad-bro
// ---------------------------------------------------------------------------

/// Koer `f` paa main-traaden og vent paa resultatet. MAA kun kaldes fra en
/// OFF-main-traad (se modul-doc). Blocker den kaldende traad paa kanalen,
/// mens main koerer callbacket — main forbliver fri.
fn on_main<T: Send + 'static>(
    app: &AppHandle,
    f: impl FnOnce(&AppHandle) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = mpsc::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let _ = tx.send(f(&handle));
    })
    .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// CDP-HTTP (synkron std-net)
// ---------------------------------------------------------------------------

fn http_get(port: u16, path: &str) -> Result<String, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(1000))
        .map_err(|e| format!("cdp connect: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(2000)))
        .ok();
    stream
        .set_write_timeout(Some(Duration::from_millis(2000)))
        .ok();
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("cdp write: {e}"))?;
    let mut raw = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut body_start = None;
    let mut expected_end = None;
    loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|e| format!("cdp read: {e}"))?;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);

        if body_start.is_none() {
            if let Some(header_end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                let start = header_end + 4;
                let headers = String::from_utf8_lossy(&raw[..header_end]);
                let content_length = headers.lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| value.trim())
                });
                if let Some(value) = content_length {
                    let length = value
                        .parse::<usize>()
                        .map_err(|e| format!("cdp: invalid content-length: {e}"))?;
                    expected_end = Some(
                        start
                            .checked_add(length)
                            .ok_or_else(|| "cdp: content-length overflow".to_string())?,
                    );
                }
                body_start = Some(start);
            }
        }

        // WebView2 svarer med Content-Length, men holder HTTP/1.1-socketen
        // aaben. Vent derfor kun paa den deklarerede body — aldrig paa EOF.
        if expected_end.is_some_and(|end| raw.len() >= end) {
            break;
        }
    }

    let start = body_start.ok_or_else(|| "cdp: malformed http response".to_string())?;
    let end = expected_end.unwrap_or(raw.len());
    if raw.len() < end {
        return Err("cdp: truncated http response".to_string());
    }
    Ok(String::from_utf8_lossy(&raw[start..end]).to_string())
}

/// `(target_id, url)` for alle `type=="page"`-targets paa CDP-porten. `Err`
/// betyder at porten ikke svarer (processen er sandsynligvis doed — polleren
/// tolker det saadan).
pub fn cdp_list_pages(port: u16) -> Result<Vec<(String, String)>, String> {
    let body = http_get(port, "/json")?;
    let arr: Vec<Value> = serde_json::from_str(&body).map_err(|e| format!("cdp parse: {e}"))?;
    Ok(arr
        .iter()
        .filter(|t| t.get("type").and_then(Value::as_str) == Some("page"))
        .filter_map(|t| {
            let id = t.get("id").and_then(Value::as_str)?.to_string();
            let url = t
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some((id, url))
        })
        .collect())
}

fn json_close(port: u16, target_id: &str) {
    let _ = http_get(port, &format!("/json/close/{target_id}"));
}

// ---------------------------------------------------------------------------
// Scope + keeper
// ---------------------------------------------------------------------------

/// Idempotent: opretter scopet (browser.rs) og dets KEEPER-webview
/// (`keeper-{scope-key}`, sentinel-URL, 0/0 1×1, straks skjult) hvis scopet
/// er nyt. Keeperen holder browser-processen + CDP-endpointet i live selv
/// naar intet kort er synligt (spec §5, S0-verificeret). Venter til keeperens
/// CDP-target er registreret, saa senere kort-diffs er entydige.
///
/// SCOPE-serialisering (review fund 1/3/12): HELE vejen — label-check,
/// sundhedsprobe, heal og foerstegangs-oprettelse inkl. keeper-tid-
/// registrering — koerer under CREATE_LOCK. Uden den kan en samtidig
/// `create_card_webview` naa at `add_child`'e et KORT midt i scope-
/// oprettelsens ready-vindue, saa keeper-identifikationen binder kortets
/// target (og renavigations-vaernet ville derefter ramme et aegte kort).
/// Laasen holdes over on_main-rundture + bounded ventetider — det er sikkert,
/// PRAECIS fordi main-traaden ALDRIG tager CREATE_LOCK (de synkrone
/// praesentations-kommandoer roerer den ikke); nye kaldere paa main er forbudt.
///
/// Fast-path'en prober CDP-sundheden i stedet for blindt at genbruge labelen
/// (2026-07-20-dogfood-bug: doedt scope + zombie-keeper-label ⇒ alle senere
/// opens fejlede "cdp before-snapshot unavailable" til app-genstart). Et
/// usundt scope HEALES paa SAMME port/profil — aldrig via remove_scope/ny
/// port: workerens allerede indlaeste `--mcp-config` peger paa porten
/// (spec §5 endpoint-uforanderlighed).
pub fn ensure_scope_ready(
    app: &AppHandle,
    opened_by: Option<&str>,
) -> Result<browser::ScopeInfo, String> {
    #[cfg(feature = "perf-trace")]
    let total_started = Instant::now();
    #[cfg(feature = "perf-trace")]
    let ensure_started = Instant::now();
    let scope = browser::ensure_scope(opened_by)?;
    crate::perf_mark!(
        "create.scope.ensure_scope",
        serde_json::json!({
            "scope": scope.key,
            "duration_ms": ensure_started.elapsed().as_secs_f64() * 1_000.0,
        }),
    );
    #[cfg(feature = "perf-trace")]
    let lock_started = Instant::now();
    let _create_guard = CREATE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    #[cfg(feature = "perf-trace")]
    let lock_wait_ms = lock_started.elapsed().as_secs_f64() * 1_000.0;
    crate::perf_mark!(
        "create.scope.create_lock_acquired",
        serde_json::json!({ "scope": scope.key, "wait_ms": lock_wait_ms }),
    );
    let keeper_label = format!("keeper-{}", scope.key);
    if app.get_webview(&keeper_label).is_some() {
        #[cfg(feature = "perf-trace")]
        let fastpath_started = Instant::now();
        #[cfg(feature = "perf-trace")]
        let mut probe_count = 0_u32;
        #[cfg(feature = "perf-trace")]
        let mut probe_http_ms = 0.0_f64;
        let healed = fastpath_heal_with(
            || {
                crate::perf_only!({
                    probe_count += 1;
                });
                #[cfg(feature = "perf-trace")]
                let probe_started = Instant::now();
                let healthy = cdp_list_pages(scope.port)
                    .map(|p| !p.is_empty())
                    .unwrap_or(false);
                crate::perf_only!({
                    probe_http_ms += probe_started.elapsed().as_secs_f64() * 1_000.0;
                });
                healthy
            },
            5,
            Duration::from_millis(100),
            || teardown_scope_webviews(app, &scope, &keeper_label),
            || wait_scope_released(app, &scope, &keeper_label),
            || create_keeper_webview(app, &scope, &keeper_label),
        )
        .map_err(|e| format!("browser scope '{}': {e}", scope.key))?;
        crate::perf_mark!(
            "create.scope.fastpath_probe.end",
            serde_json::json!({
                "scope": scope.key,
                "duration_ms": fastpath_started.elapsed().as_secs_f64() * 1_000.0,
                "probe_count": probe_count,
                "probe_http_ms": probe_http_ms,
                "healed": healed,
            }),
        );
        if healed {
            eprintln!(
                "[canvas] browser scope '{}' healed (dead cdp endpoint, keeper recreated)",
                scope.key
            );
        }
        // Tid-reparation (review F1): en tidligere create_keeper_webview kan
        // vaere fejlet EFTER add_child (CDP-ready-timeout/ambiguity) og have
        // efterladt keeperen staaende med uregistreret target-id. Uden id er
        // keeperen kun blank-fritaget — en kapring ville draebe den som foer
        // fixet. Reparér under samme CREATE_LOCK (intet kort kan blande sig).
        if browser::keeper_target(&scope.key).is_none() {
            let pages = cdp_list_pages(scope.port).map_err(|e| {
                format!("browser scope '{}': keeper repair snapshot: {e}", scope.key)
            })?;
            let keeper_tid = identify_keeper(&pages, &scope.key)
                .map_err(|e| format!("browser scope '{}': keeper repair: {e}", scope.key))?;
            browser::set_keeper_target(&scope.key, Some(keeper_tid));
        }
        crate::perf_mark!(
            "create.scope.end",
            serde_json::json!({
                "scope": scope.key,
                "path": if healed { "healed" } else { "healthy_fastpath" },
                "create_lock_wait_ms": lock_wait_ms,
                "duration_ms": total_started.elapsed().as_secs_f64() * 1_000.0,
            }),
        );
        return Ok(scope);
    }
    create_keeper_webview(app, &scope, &keeper_label)?;
    crate::perf_mark!(
        "create.scope.end",
        serde_json::json!({
            "scope": scope.key,
            "path": "new",
            "create_lock_wait_ms": lock_wait_ms,
            "duration_ms": total_started.elapsed().as_secs_f64() * 1_000.0,
        }),
    );
    Ok(scope)
}

/// Lazy-scope TOCTOU-værn: bevis at scopets allerede publicerede CDP-port
/// stadig er ledig, og hold den indtil main-callbacket er klar til add_child.
/// Listeneren droppes umiddelbart før WebView2 binder samme port. Fejl må
/// aldrig udløse en ny port — endpointet er bagt ind i workerens MCP-config.
pub fn reserve_exact_cdp_port(port: u16) -> Result<TcpListener, String> {
    TcpListener::bind(("127.0.0.1", port)).map_err(|error| {
        format!(
            "CDP-port {port} er ikke længere ledig ({error}); luk ejer-kortet og opret det igen"
        )
    })
}

/// Opretter keeper-webviewen paa sentinel-URL'en, venter paa CDP + keeperens
/// target og registrerer target-id'et paa scopet. Kommer CDP ALDRIG op inden
/// for vinduet, er scopet ubrugeligt: en tom before-snapshot ville lade
/// create_card_webview forveksle keeperens target med kortets (final-review
/// Finding 2). Hard-fejl i stedet for at returnere Ok — kalderne rydder op:
/// open_browser_card's `?` returnerer FOER registry-kortet oprettes, og
/// worker-spawn-injektionen er best-effort (spawner uden browser-tools).
/// Keeperen bliver staaende og rives ned med scopet ved ejer-terminalens
/// close (teardown_scope). MAA kun kaldes under CREATE_LOCK.
fn create_keeper_webview(
    app: &AppHandle,
    scope: &browser::ScopeInfo,
    keeper_label: &str,
) -> Result<(), String> {
    #[cfg(feature = "perf-trace")]
    let total_started = Instant::now();
    // Lazy keeper betyder, at porten har været pick-then-release siden
    // worker-spawn. Bind samme port eksklusivt nu: et fremmed target på en
    // stjålet port må aldrig kunne passere identify_keeper-fallbacken.
    let port_guard = reserve_exact_cdp_port(scope.port)?;
    std::fs::create_dir_all(&scope.profile_dir).map_err(|e| e.to_string())?;
    let args = browser::additional_browser_args(scope.port);
    let dir = scope.profile_dir.clone();
    let label = keeper_label.to_string();
    let sentinel = browser::keeper_sentinel(&scope.key);
    #[cfg(feature = "perf-trace")]
    let on_main_started = Instant::now();
    on_main(app, move |a| {
        // WebView2 kan først binde porten efter reservationen er sluppet.
        // Vinduet er kun selve add_child-kaldet; porten skiftes aldrig.
        drop(port_guard);
        let window = a.get_window("main").ok_or("no main window")?;
        let url: url::Url = sentinel
            .parse()
            .map_err(|e| format!("keeper sentinel url: {e}"))?;
        let builder = WebviewBuilder::new(&label, WebviewUrl::External(url))
            .data_directory(dir)
            .additional_browser_args(&args);
        let wv = window
            .add_child(
                builder,
                LogicalPosition::new(0.0, 0.0),
                LogicalSize::new(1.0, 1.0),
            )
            .map_err(|e| format!("keeper add_child: {e}"))?;
        wv.hide().map_err(|e| e.to_string())?;
        Ok(())
    })?;
    crate::perf_mark!(
        "create.keeper.on_main.end",
        serde_json::json!({
            "scope": scope.key,
            "duration_ms": on_main_started.elapsed().as_secs_f64() * 1_000.0,
        }),
    );
    let mut ready_pages: Option<Vec<(String, String)>> = None;
    #[cfg(feature = "perf-trace")]
    let ready_started = Instant::now();
    #[cfg(feature = "perf-trace")]
    let mut attempts = 0_u32;
    #[cfg(feature = "perf-trace")]
    let mut http_ms = 0.0_f64;
    for _ in 0..50 {
        crate::perf_only!({
            attempts += 1;
        });
        #[cfg(feature = "perf-trace")]
        let http_started = Instant::now();
        if let Ok(pages) = cdp_list_pages(scope.port) {
            crate::perf_only!({
                http_ms += http_started.elapsed().as_secs_f64() * 1_000.0;
            });
            if !pages.is_empty() {
                ready_pages = Some(pages);
                break;
            }
        } else {
            crate::perf_only!({
                http_ms += http_started.elapsed().as_secs_f64() * 1_000.0;
            });
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let Some(pages) = ready_pages else {
        return Err(format!(
            "cdp endpoint did not come up for scope {}",
            scope.key
        ));
    };
    let keeper_tid = identify_keeper(&pages, &scope.key)?;
    browser::set_keeper_target(&scope.key, Some(keeper_tid));
    crate::perf_mark!(
        "create.keeper.cdp_ready.end",
        serde_json::json!({
            "scope": scope.key,
            "attempts": attempts,
            "http_ms": http_ms,
            "duration_ms": ready_started.elapsed().as_secs_f64() * 1_000.0,
            "total_ms": total_started.elapsed().as_secs_f64() * 1_000.0,
        }),
    );
    Ok(())
}

/// Heal-fase 1: doedsmarkér + luk ALLE scopets webviews (keeper + kort).
/// Kortenes zombie-webviews SKAL med — deres WebView2-controllere holder
/// ellers den halvdoede browserproces i live, og en ny keeper paa samme
/// user-data-dir ville JOINE den defekte proces i stedet for at starte en
/// frisk (WebView2's procesmodel). Registry-kortene bliver staaende som
/// doede/lukbare; CDP'en er allerede uopnaaelig, saa de VAR de facto doede.
fn teardown_scope_webviews(
    app: &AppHandle,
    scope: &browser::ScopeInfo,
    keeper_label: &str,
) -> Result<(), String> {
    let names: Vec<String> = browser_cards_in_scope(&scope.key)
        .into_iter()
        .map(|(name, _, _)| name)
        .collect();
    for name in &names {
        let _ = registry::update_browser_card(name, None, None, Some(false), None);
    }
    let card_labels: Vec<String> = names.iter().map(|n| format!("browser-{n}")).collect();
    let keeper = keeper_label.to_string();
    on_main(app, move |a| {
        for label in card_labels.iter().chain(std::iter::once(&keeper)) {
            if let Some(wv) = a.get_webview(label) {
                if let Err(e) = wv.close() {
                    eprintln!("[canvas] heal: close of zombie webview {label} failed: {e}");
                }
            }
        }
        Ok(())
    })?;
    browser::set_keeper_target(&scope.key, None);
    finalize_dead(app, names);
    Ok(())
}

/// Heal-fase 2: vent bounded paa at Tauri frigiver keeper-labelen (close er
/// asynkron — umiddelbar add_child med samme label ville fejle) OG paa at
/// CDP-porten er fri (den gamle browserproces helt doed — ellers kan den nye
/// keeper ikke genbinde porten).
fn wait_scope_released(app: &AppHandle, scope: &browser::ScopeInfo, keeper_label: &str) -> bool {
    let mut label_free = false;
    for _ in 0..20 {
        if app.get_webview(keeper_label).is_none() {
            label_free = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !label_free {
        return false;
    }
    let addr = SocketAddr::from(([127, 0, 0, 1], scope.port));
    for _ in 0..50 {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

// ---------------------------------------------------------------------------
// Kort-webview
// ---------------------------------------------------------------------------

/// Opretter kortets webview (`browser-{name}`), venter paa dets CDP-target og
/// returnerer target-id'et. Fejler CDP-ready-loopet (spec §9: ingen halve
/// kort), lukkes webviewen igen og der returneres `Err` — kalderen fjerner
/// registry-kortet.
fn create_card_webview(
    app: &AppHandle,
    name: &str,
    scope: &browser::ScopeInfo,
    url: &str,
) -> Result<String, String> {
    let label = format!("browser-{name}");
    let _create_guard = CREATE_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    // before-snapshot: vent kort paa at CDP svarer med mindst keeperens target,
    // saa diffen mod after aldrig kan ramme keeperen. En TOM before-snapshot
    // (CDP nede ELLER endpointet endnu ikke lister keeperen) er farlig:
    // new_target_id ville da returnere det FOERSTE target i after (typisk
    // keeperen) som "det nye kort". Vi accepterer derfor kun et IKKE-tomt
    // snapshot; er det stadig tomt efter vinduet, hard-fejler vi FOER add_child
    // (ingen webview at rydde; open_browser_card's Err-arm fjerner kortet).
    let mut before = Vec::new();
    for _ in 0..20 {
        match cdp_list_pages(scope.port) {
            Ok(pages) if !pages.is_empty() => {
                before = pages;
                break;
            }
            _ => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    before_snapshot_ok(&before)?;

    let known_bounds = {
        let p = PRESENTATION.lock().map_err(|e| e.to_string())?;
        p.bounds.get(name).copied()
    };
    let args = browser::additional_browser_args(scope.port);
    let dir = scope.profile_dir.clone();
    let parsed: url::Url = url.parse().map_err(|e| format!("bad url: {e}"))?;

    let app_nav = app.clone();
    let name_nav = name.to_string();
    let app_title = app.clone();
    let name_title = name.to_string();
    let label_add = label.clone();

    on_main(app, move |a| {
        let window = a.get_window("main").ok_or("no main window")?;
        let builder = WebviewBuilder::new(&label_add, WebviewUrl::External(parsed))
            .data_directory(dir)
            .additional_browser_args(&args)
            .on_navigation(move |u: &url::Url| {
                // Politikken bor i `navigation_allowed` — closuren beslutter
                // intet selv. Den daekker BEGGE veje ind: `validate_card_url`
                // ved oprettelse, og denne ved enhver senere navigation
                // (inkl. redirects, som er den vej en fremmed side ellers
                // kunne foere kortet hen paa appens origin).
                if !navigation_allowed(u) {
                    return false;
                }
                if matches!(u.scheme(), "http" | "https") {
                    let _ = registry::update_browser_card(
                        &name_nav,
                        Some(u.to_string()),
                        None,
                        None,
                        None,
                    );
                    emit_card_update(&app_nav, &name_nav);
                }
                true
            })
            .on_document_title_changed(move |_wv, title| {
                let _ = registry::update_browser_card(&name_title, None, Some(title), None, None);
                emit_card_update(&app_title, &name_title);
            })
            .on_download(|_wv, _event| false);
        // Offscreen-prewarm (bug 4): et nyt kort har endnu ingen kendte
        // bounds, og et 1x1-barn faar Chromium til at udskyde layout/paint
        // til foerste set_browser_bounds (synligt som "URL i feltet, blank
        // side" i op til flere sekunder). Giv i stedet en reel viewport i
        // vinduets stoerrelse, placeret helt til venstre for klientfladen,
        // saa siden renderer FOER frontenden maaler body-rekten ind.
        let (x, y, w, h) = known_bounds.unwrap_or_else(|| {
            let logical = window.inner_size().ok().map(|size| {
                let scale = window.scale_factor().unwrap_or(1.0);
                (
                    (f64::from(size.width) / scale).max(1.0),
                    (f64::from(size.height) / scale).max(1.0),
                )
            });
            match logical {
                Some((lw, lh)) => (-lw - 8.0, 0.0, lw, lh),
                None => (0.0, 0.0, 1.0, 1.0),
            }
        });
        window
            .add_child(builder, LogicalPosition::new(x, y), LogicalSize::new(w, h))
            .map_err(|e| format!("browser card add_child: {e}"))?;
        Ok(())
    })?;

    // CDP-ready-loop (S3b): op til 50 × 100 ms.
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(after) = cdp_list_pages(scope.port) {
            if let Some(id) = new_target_id(&before, &after) {
                return Ok(id);
            }
        }
    }

    // Timeout: luk webviewen (ingen halve kort, spec §9). En close-fejl her
    // efterlader en orphan-webview uden registry-kort — logges saa den kan
    // korreleres, hvis en usynlig webview dukker op i en fejlrapport.
    let label_close = label.clone();
    let close_result = on_main(app, move |a| {
        if let Some(wv) = a.get_webview(&label_close) {
            wv.close().map_err(|e| e.to_string())?;
        }
        Ok(())
    });
    if let Err(e) = close_result {
        eprintln!("[canvas] timeout-cleanup close of {label} failed: {e}");
    }
    Err("cdp target did not appear".to_string())
}

/// Resultatet af en fuld kort-aabning (Task 5/6: MCP-`open` bruger alle tre
/// felter; kommandoen kun `info`).
pub struct OpenedCard {
    pub info: registry::CardInfo,
    pub target_id: String,
    pub cdp_endpoint: String,
}

/// Fuld aaben-vej: URL-politik → scope/keeper → registry-kort → webview →
/// CDP-target. Fejler webviewen, fjernes registry-kortet (ingen halve kort).
/// Kaldes af `create_browser_card`-kommandoen OG af MCP-`open` (samme sti).
pub fn open_browser_card(
    app: &AppHandle,
    url: Option<&str>,
    opened_by: Option<&str>,
) -> Result<OpenedCard, String> {
    let resolved = validate_card_url(url)?;
    let scope = ensure_scope_ready(app, opened_by)?;
    let info = registry::create_browser_card(
        opened_by.map(str::to_string),
        scope.key.clone(),
        resolved.clone(),
        String::new(),
    )?;
    match create_card_webview(app, &info.name, &scope, &resolved) {
        Ok(target_id) => {
            // alive=Some(true), ikke None (review F2): en samtidig scope-heal
            // kan have doedsmarkeret det tid-loese kort mid-creation; et
            // succesfuldt CDP-target BEVISER at webviewen lever i retur-
            // oejeblikket, saa Ok-kontrakten ("aabnet kort er levende")
            // genoprettes her — doer det reelt igen, re-detekterer polleren.
            let _ = registry::update_browser_card(
                &info.name,
                None,
                None,
                Some(true),
                Some(target_id.clone()),
            );
            // Respekter aktuel occlusion/fullscreen for det nye kort.
            let _ = apply_presentation(app);
            // Ubetinget create-emit (final-review minor): on_navigation-emitten
            // fyrer KUN for http/https, saa et about:blank-kort (MCP open uden
            // url) ville ellers vaere usynligt i gridden indtil en urelateret
            // refresh. Emit her, saa gridden renderer kortet straks.
            emit_card_update(app, &info.name);
            Ok(OpenedCard {
                info,
                target_id,
                cdp_endpoint: format!("http://127.0.0.1:{}", scope.port),
            })
        }
        Err(e) => {
            // Webviewen er allerede lukket i create_card_webview; fjern kortet
            // og emit DEREFTER — polleren kan have naaet at doedsmarkere +
            // emitte det halvfaerdige kort (scope-doeds-armen fyrer hvert tick
            // ved streak ≥2), saa frontenden HAR muligvis renderet det. Uden
            // det afsluttende event blev dead-UI'et staaende som ghost-kort
            // (2026-07-20-dogfood-bug).
            cleanup_failed_open(&info.name, |names| finalize_dead(app, names));
            Err(e)
        }
    }
}

/// Naviger et eksisterende kort. URL-politik (http/https). Registry-URL +
/// `browser-card-updated` opdateres af webviewens navigations-callback.
pub fn navigate_card(app: &AppHandle, name: &str, url: &str) -> Result<(), String> {
    let resolved = validate_card_url(Some(url))?;
    let parsed: url::Url = resolved.parse().map_err(|e| format!("bad url: {e}"))?;
    let label = format!("browser-{name}");
    let name_owned = name.to_string();
    on_main(app, move |a| {
        let wv = a
            .get_webview(&label)
            .ok_or_else(|| format!("no webview for card: {name_owned}"))?;
        wv.navigate(parsed).map_err(|e| e.to_string())
    })
}

// ---------------------------------------------------------------------------
// Praesentation (occlusion / fullscreen / bounds / focus)
// ---------------------------------------------------------------------------

/// DIREKTE webview-ops for alle browser-kort — MAA koeres paa main-traaden
/// (kaldes af de synkrone `set_browser_*`-kommandoer og via [`on_main`] fra
/// off-main-veje). Fuldskaerm/occlusion er ren SYNLIGHED her; layoutet (og
/// fuldskaerms-bounds) ejes af frontenden, der rapporterer via
/// `set_browser_bounds`.
fn present_all(app: &AppHandle) {
    let (occluded, fullscreen, bounds) = {
        let Ok(p) = PRESENTATION.lock() else { return };
        (p.occluded, p.fullscreen.clone(), p.bounds.clone())
    };
    for (name, alive) in browser_cards_alive() {
        let label = format!("browser-{name}");
        let Some(wv) = app.get_webview(&label) else {
            continue;
        };
        if browser::webview_should_show(occluded, fullscreen.as_deref(), &name, alive) {
            if let Some(&(x, y, w, h)) = bounds.get(&name) {
                let _ = wv.set_position(LogicalPosition::new(x, y));
                let _ = wv.set_size(LogicalSize::new(w, h));
            }
            let _ = wv.show();
        } else {
            let _ = wv.hide();
        }
    }
}

/// DIREKTE single-webview op — MAA koeres paa main. Bruges af
/// `set_browser_bounds`-kommandoen (hoej frekvens under drag/resize; undgaar
/// det fulde sweep).
fn present_one(app: &AppHandle, name: &str, x: f64, y: f64, w: f64, h: f64) {
    let (occluded, fullscreen) = {
        let Ok(p) = PRESENTATION.lock() else { return };
        (p.occluded, p.fullscreen.clone())
    };
    let label = format!("browser-{name}");
    let Some(wv) = app.get_webview(&label) else {
        return;
    };
    if browser::webview_should_show(
        occluded,
        fullscreen.as_deref(),
        name,
        browser_card_alive(name),
    ) {
        let _ = wv.set_position(LogicalPosition::new(x, y));
        let _ = wv.set_size(LogicalSize::new(w, h));
        let _ = wv.show();
    } else {
        let _ = wv.hide();
    }
}

/// OFF-main wrapper om [`present_all`] (open-vejen + polleren).
pub fn apply_presentation(app: &AppHandle) -> Result<(), String> {
    on_main(app, |a| {
        present_all(a);
        Ok(())
    })
}

/// `set_browser_bounds` (synkron kommando, paa main): opdater bounds-map +
/// DIREKTE single-webview op.
pub fn set_bounds(
    app: &AppHandle,
    name: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Result<(), String> {
    if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) {
        return Err("bounds must be finite".to_string());
    }
    {
        let mut p = PRESENTATION.lock().map_err(|e| e.to_string())?;
        p.bounds.insert(name.to_string(), (x, y, w, h));
    }
    present_one(app, name, x, y, w, h);
    Ok(())
}

/// `set_browser_occlusion` (synkron kommando, paa main): muter + sweep.
pub fn set_occlusion(app: &AppHandle, occluded: bool) -> Result<(), String> {
    {
        let mut p = PRESENTATION.lock().map_err(|e| e.to_string())?;
        p.occluded = occluded;
    }
    present_all(app);
    Ok(())
}

/// `set_browser_fullscreen` (synkron kommando, paa main): muter + sweep.
pub fn set_fullscreen(app: &AppHandle, name: Option<String>) -> Result<(), String> {
    {
        let mut p = PRESENTATION.lock().map_err(|e| e.to_string())?;
        p.fullscreen = name;
    }
    present_all(app);
    Ok(())
}

/// Rydder fuldskaerm hvis `name` var fuldskaerms-kortet. Returnerer true naar
/// noget aendredes (polleren re-applyer da praesentationen). Kalderen sikrer
/// selv sweep'et (main-traad-kontekst).
pub fn clear_fullscreen_if(name: &str) -> bool {
    let Ok(mut p) = PRESENTATION.lock() else {
        return false;
    };
    if p.fullscreen.as_deref() == Some(name) {
        p.fullscreen = None;
        true
    } else {
        false
    }
}

/// DIREKTE focus — MAA koeres paa main. `focus_browser_card`-kommandoen kalder
/// den direkte; MCP-`focus` via [`on_main`].
fn focus_now(app: &AppHandle, name: &str) -> Result<(), String> {
    let wv = app
        .get_webview(&format!("browser-{name}"))
        .ok_or_else(|| format!("no webview for card: {name}"))?;
    wv.set_focus().map_err(|e| e.to_string())
}

/// `focus_browser_card` (synkron kommando, paa main).
pub fn focus_card(app: &AppHandle, name: &str) -> Result<(), String> {
    focus_now(app, name)
}

// ---------------------------------------------------------------------------
// Teardown (kaskade-luk)
// ---------------------------------------------------------------------------

/// Resultatet fra den testbare preclose-soem. `closed` er native webviews hvis
/// close blev accepteret og hvis registry-spejlet derfor er markeret doedt;
/// selve registry-entryen er stadig til stede, indtil fase 2 kalder
/// `registry::close_cards`.
#[derive(Debug, Clone, PartialEq)]
pub struct BrowserPrecloseResult {
    pub closed: Vec<String>,
    pub errors: Vec<registry::CloseCardsError>,
}

/// Fase 1 af browser-close, med den native operation injiceret som closure saa
/// failure-semantikken kan regressionstestes uden en rigtig WebView2-runtime.
///
/// Invarianten er vigtig: en native close-fejl fjerner ALDRIG registry-entryen.
/// Succes markerer kortet `alive=false`, men det forbliver adresserbart, indtil
/// kalderen eksplicit udfoerer registry-fase 2. Hvis fase 2 mod forventning
/// fejler (fx en poisoned registry-lock), bliver der derfor et synligt, doedt,
/// retrybart kort frem for en usynlig native orphan.
pub fn preclose_browser_cards_with(
    names: &[String],
    mut close_webview: impl FnMut(&registry::BrowserCloseTarget) -> Result<(), String>,
) -> Result<BrowserPrecloseResult, String> {
    let targets = registry::browser_close_targets(names)?;
    let mut closed = Vec::new();
    let mut errors = Vec::new();

    for target in targets {
        match close_webview(&target) {
            Ok(()) => {
                closed.push(target.name.clone());
                if let Err(message) =
                    registry::update_browser_card(&target.name, None, None, Some(false), None)
                {
                    errors.push(registry::CloseCardsError {
                        name: target.name,
                        message: format!(
                            "native webview closed but registry update failed: {message}"
                        ),
                    });
                }
            }
            Err(message) => errors.push(registry::CloseCardsError {
                name: target.name,
                message,
            }),
        }
    }

    Ok(BrowserPrecloseResult { closed, errors })
}

/// Produktionsbindingen for fase 1. `Webview::close`-fejl og main-thread-
/// roundtrip-fejl propageres. Et succesfuldt `close()` er commit-graensen for
/// den native fase; vi binder ikke lifecycle-koden til Tauri-runtime-intern
/// timing efter det offentlige Result.
fn preclose_browser_webviews(
    app: &AppHandle,
    names: &[String],
) -> Result<BrowserPrecloseResult, String> {
    preclose_browser_cards_with(names, |target| {
        let label = format!("browser-{}", target.name);
        let name = target.name.clone();
        let was_alive = target.alive;
        #[cfg(feature = "perf-trace")]
        let on_main_started = Instant::now();
        let result = on_main(app, move |a| {
            let Some(webview) = a.get_webview(&label) else {
                // En tidligere accepteret preclose kan efterlade et doedt,
                // retrybart registry-kort, hvis registry-/workspace-fase 2
                // fejlede. Kun den eksplicit doede tilstand maa behandle en
                // manglende native handle som idempotent succes.
                return if was_alive {
                    Err(format!("no native webview for live browser card: {name}"))
                } else {
                    Ok(())
                };
            };
            webview
                .close()
                .map_err(|e| format!("native webview close failed: {e}"))?;
            Ok(())
        });
        crate::perf_mark!(
            "close.browser_native.target.end",
            serde_json::json!({
                "name": target.name,
                "was_alive": was_alive,
                "duration_ms": on_main_started.elapsed().as_secs_f64() * 1_000.0,
                "ok": result.is_ok(),
            }),
        );
        result
    })
}

/// Fjern preclose-fejlede kort fra registry-batchen, saa RESTEN af batchen
/// (terminaler + succesfuldt preclosede browser-kort) stadig lukkes. En
/// wedged browser-webview maa aldrig holde sin ejer-terminal aaben. Ren
/// funktion — testbar uden AppHandle. Returnerer (to_close, skipped).
pub fn split_close_batch(
    names: Vec<String>,
    errors: &[registry::CloseCardsError],
) -> (Vec<String>, Vec<String>) {
    let failed: HashSet<&str> = errors.iter().map(|e| e.name.as_str()).collect();
    names
        .into_iter()
        .partition(|name| !failed.contains(name.as_str()))
}

/// River et scopes keeper ned + fjerner scopet (browser.rs). Kaldes ved
/// ejer-terminalens close — browser-processen doer med keeperen. No-op hvis
/// scopet ikke findes (fx en terminal der aldrig aabnede browser-kort).
pub fn teardown_scope(app: &AppHandle, scope_key: &str) {
    if !browser::all_scopes().iter().any(|s| s.key == scope_key) {
        return;
    }
    let keeper_label = format!("keeper-{scope_key}");
    let _ = on_main(app, move |a| {
        if let Some(wv) = a.get_webview(&keeper_label) {
            let _ = wv.close();
        }
        Ok(())
    });
    browser::remove_scope(scope_key);
}

/// Efterbehandling af en gennemfoert registry-close: riv agent-scopes ned for
/// lukkede terminal-ejere og emit igen EFTER detach, saa MCP-/agent-close altid
/// invaliderer frontendens kort-snapshot. `canvas`-scopet (scope_key(None), det
/// app-ejede) lever app-processen ud.
fn finish_close(app: &AppHandle, result: &registry::CloseCardsResult) {
    let browser_names: HashSet<&str> = result
        .browser_closed
        .iter()
        .map(|b| b.name.as_str())
        .collect();
    for name in &result.closed {
        if !browser_names.contains(name.as_str()) {
            teardown_scope(app, &browser::scope_key(Some(name)));
        }
    }
    finalize_dead(
        app,
        result
            .browser_closed
            .iter()
            .map(|browser| browser.name.clone())
            .collect(),
    );
}

/// Den samlede browser-/registry-lifecycle, delt af Tauri-close-kommandoerne
/// og MCP. Den native fase ligger bevidst FOER den injicerede registry-/
/// workspace-fase; target-laasene spaender over begge, saa samme kort ikke kan
/// blive dobbelt-preclosed mellem faserne. Uafhaengige target-saet koerer
/// parallelt.
///
/// Partial-close-semantik: preclose-fejlede browser-kort fjernes fra
/// registry-batchen og rapporteres som strukturerede entries i
/// `result.errors`, mens RESTEN af batchen (terminaler + oevrige browser-
/// kort) lukkes faerdigt — en wedged webview maa aldrig holde sin ejer-
/// terminal aaben. De fejlede kort forbliver live og retrybare.
///
/// Fejler registry-closuren FOER detach, bliver preclosede browser-kort
/// staaende doede og retrybare. Workspace-lagets eksisterende fejl EFTER
/// detach er fortsat en struktureret entry med navnet `workspace` i
/// `result.errors` (ikke Err): native+registry forbliver enige om "lukket",
/// og workspace beholder sit dirty snapshot til senere flush; en lukket
/// native webview kan ikke rulles sikkert tilbage.
pub fn close_cards_lifecycle(
    app: &AppHandle,
    names: Vec<String>,
    close_registry: impl FnOnce(Vec<String>) -> Result<registry::CloseCardsResult, String>,
) -> Result<registry::CloseCardsResult, String> {
    #[cfg(feature = "perf-trace")]
    let lifecycle_started = Instant::now();
    #[cfg(feature = "perf-trace")]
    let requested_count = names.len();
    // Poison-recovery pr. target: en tidligere panic i ét close maa ikke gøre
    // senere closes af samme eller andre kort umulige. Helperens sorterede,
    // dedupede rækkefølge er samtidig AB/BA-deadlock-værnet for batches.
    #[cfg(feature = "perf-trace")]
    let lock_started = Instant::now();
    let close_locks = close_target_locks(&names);
    let _close_guards: Vec<_> = close_locks
        .iter()
        .map(|lock| lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
        .collect();
    #[cfg(feature = "perf-trace")]
    let lock_wait_ms = lock_started.elapsed().as_secs_f64() * 1_000.0;
    crate::perf_mark!(
        "close.close_lock_acquired",
        serde_json::json!({
            "wait_ms": lock_wait_ms,
            "requested_count": requested_count,
            "target_lock_count": close_locks.len(),
        }),
    );
    #[cfg(feature = "perf-trace")]
    let browser_started = Instant::now();
    let preclosed = preclose_browser_webviews(app, &names)?;
    crate::perf_mark!(
        "close.browser_native.end",
        serde_json::json!({
            "duration_ms": browser_started.elapsed().as_secs_f64() * 1_000.0,
            "closed_count": preclosed.closed.len(),
            "error_count": preclosed.errors.len(),
        }),
    );
    let (to_close, skipped) = split_close_batch(names, &preclosed.errors);

    if to_close.is_empty() {
        // Hele batchen fejlede preclose (typisk ét enkelt wedged browser-
        // kort). Strukturerede fejl pr. target; kortene er live og retrybare.
        finalize_dead(app, preclosed.closed);
        crate::perf_mark!(
            "close.lifecycle.end",
            serde_json::json!({
                "duration_ms": lifecycle_started.elapsed().as_secs_f64() * 1_000.0,
                "close_lock_wait_ms": lock_wait_ms,
                "closed_count": 0,
                "all_preclose_failed": true,
            }),
        );
        return Ok(registry::CloseCardsResult {
            closed: Vec::new(),
            errors: preclosed.errors,
            sequence_reset: false,
            browser_closed: Vec::new(),
        });
    }

    #[cfg(feature = "perf-trace")]
    let registry_started = Instant::now();
    match close_registry(to_close) {
        Ok(mut result) => {
            #[cfg(feature = "perf-trace")]
            let registry_ms = registry_started.elapsed().as_secs_f64() * 1_000.0;
            crate::perf_mark!(
                "close.registry_workspace.end",
                serde_json::json!({
                    "duration_ms": registry_ms,
                    "closed_count": result.closed.len(),
                    "error_count": result.errors.len(),
                }),
            );
            #[cfg(feature = "perf-trace")]
            let finish_started = Instant::now();
            // Ét success-event EFTER detach undgaar konkurrerende refresh-
            // snapshots, hvor et preclose-event ellers kunne genindsaette en
            // stale dead-card efter den afsluttende refresh.
            finish_close(app, &result);
            // Sjaeldent hjoerne: native close accepteret men registry-mark
            // fejlede — kortet er skipped OG natively lukket; hold UI'et i
            // sync med dets doede tilstand alligevel.
            let skipped_closed: Vec<String> = preclosed
                .closed
                .iter()
                .filter(|name| skipped.contains(name))
                .cloned()
                .collect();
            finalize_dead(app, skipped_closed);
            result.errors.extend(preclosed.errors);
            crate::perf_mark!(
                "close.finish.end",
                serde_json::json!({
                    "duration_ms": finish_started.elapsed().as_secs_f64() * 1_000.0,
                }),
            );
            crate::perf_mark!(
                "close.lifecycle.end",
                serde_json::json!({
                    "duration_ms": lifecycle_started.elapsed().as_secs_f64() * 1_000.0,
                    "close_lock_wait_ms": lock_wait_ms,
                    "closed_count": result.closed.len(),
                    "error_count": result.errors.len(),
                }),
            );
            Ok(result)
        }
        Err(error) => {
            // Browser-webviewen er allerede lukket, men registry-fasen naaede
            // ikke at detach'e. Vis det retrybare doede kort med det samme.
            finalize_dead(app, preclosed.closed);
            crate::perf_mark!(
                "close.lifecycle.end",
                serde_json::json!({
                    "duration_ms": lifecycle_started.elapsed().as_secs_f64() * 1_000.0,
                    "close_lock_wait_ms": lock_wait_ms,
                    "ok": false,
                }),
            );
            Err(error)
        }
    }
}

// ---------------------------------------------------------------------------
// Registry-laesehjaelpere
// ---------------------------------------------------------------------------

fn read_browser_url_title(name: &str) -> Option<(String, String)> {
    let handle = registry::card_handle(name).ok()?;
    let card = handle.lock().ok()?;
    if let registry::CardBackend::Browser(b) = &card.backend {
        Some((b.url.clone(), b.title.clone()))
    } else {
        None
    }
}

fn emit_card_update(app: &AppHandle, name: &str) {
    if let Some((url, title)) = read_browser_url_title(name) {
        let _ = app.emit(
            "browser-card-updated",
            BrowserCardUpdated {
                name: name.to_string(),
                url,
                title,
            },
        );
    }
}

/// Én walk over registryet — og det ENE sted `CardBackend`s tre-arms-match
/// staar i denne fil. Opslagene nedenfor var tidligere fem kopier af den samme
/// laas-og-match-loekke, saa en fjerde backend-variant kostede fem
/// redigeringer her alene; nu koster den én.
fn map_browser_cards<T>(
    mut pick: impl FnMut(&registry::CardRuntime, &registry::BrowserRuntime) -> Option<T>,
) -> Vec<T> {
    registry::all_handles()
        .iter()
        .filter_map(|handle| {
            let card = handle.lock().ok()?;
            let registry::CardBackend::Browser(browser) = &card.backend else {
                return None;
            };
            pick(&card, browser)
        })
        .collect()
}

fn browser_cards_alive() -> Vec<(String, bool)> {
    map_browser_cards(|_, b| Some((b.name.clone(), b.alive)))
}

/// Alive-flaget for ét browser-kort — `false` for ukendte/fjernede kort, saa
/// praesentationen fail-safe skjuler i stedet for at vise en zombie-webview.
fn browser_card_alive(name: &str) -> bool {
    let Ok(handle) = registry::card_handle(name) else {
        return false;
    };
    let Ok(card) = handle.lock() else {
        return false;
    };
    match &card.backend {
        registry::CardBackend::Browser(b) => b.alive,
        registry::CardBackend::Terminal(_) | registry::CardBackend::Chat(_) => false,
    }
}

/// (name, target_id, alive) for hvert browser-kort i et scope — polleren.
fn browser_cards_in_scope(scope_key: &str) -> Vec<(String, String, bool)> {
    map_browser_cards(|_, b| {
        (b.scope_key == scope_key).then(|| (b.name.clone(), b.target_id.clone(), b.alive))
    })
}

/// (name, opened_by) for browser-kortet med et givent nummer. `None` hvis
/// nummeret ikke findes ELLER peger paa et terminal-kort.
fn browser_card_by_number(number: u32) -> Option<(String, Option<String>)> {
    map_browser_cards(|card, b| {
        (card.number == number).then(|| (b.name.clone(), b.opened_by.clone()))
    })
    .into_iter()
    .next()
}

fn browser_card_rows() -> Vec<mcp::BrowserCardRow> {
    let mut rows = map_browser_cards(|card, b| {
        Some(mcp::BrowserCardRow {
            number: card.number,
            opened_by: b.opened_by.clone(),
            url: b.url.clone(),
            title: b.title.clone(),
            target_id: b.target_id.clone(),
        })
    });
    rows.sort_by_key(|r| r.number);
    rows
}

// ---------------------------------------------------------------------------
// Reconciliation-/liveness-poller (spec §5)
// ---------------------------------------------------------------------------

/// Doeds-kvalificerede ticks et target-loest (mid-creation) kort overlever i
/// scope-doeds-armen: 4 ticks ≈ 8 s > worst-case-oprettelsen ~7 s (1 s
/// before-loop + 5 s ready-loop + slaek).
const EMPTY_TID_GRACE_TICKS: u32 = 4;

/// Den ENESTE doeds-detektor (spike S7: ingen automatisk death-detection).
/// Hvert 2 s pr. scope: doed CDP — eller et sample uden keeperens eget target,
/// som er lige saa utrovaerdigt ([`sample_contradicts_keeper`]) — ⇒ scopets
/// kort doer (2-fejls-debounce + tomt-target-naade, [`scope_dead_names`]);
/// ukendte page-targets (playwrights `browser_tabs new`, S5) lukkes via
/// `/json/close/{id}` efter 2 sightings; forsvundne kort-targets ⇒ doede
/// (straks, naar samplet indeholder keeperen); en KAPRET keeper (agent
/// navigerede den udenom kort-systemet) renavigeres til sentinelen og
/// meldes til HUD'en — aldrig draebes ([`reconcile_actions`]).
pub fn spawn_reconciliation_poller(app: AppHandle) {
    std::thread::spawn(move || {
        let mut pending: HashMap<String, u32> = HashMap::new();
        let mut scope_failures: HashMap<String, u32> = HashMap::new();
        let mut empty_tid_grace: HashMap<String, u32> = HashMap::new();
        let mut hijack_reported: HashSet<String> = HashSet::new();
        loop {
            std::thread::sleep(Duration::from_secs(2));
            #[cfg(feature = "perf-trace")]
            let tick_started = Instant::now();
            let mut seen_unknown: HashSet<String> = HashSet::new();
            let mut active_scopes: HashSet<String> = HashSet::new();
            let mut graceable: HashSet<String> = HashSet::new();
            for scope in browser::all_scopes() {
                active_scopes.insert(scope.key.clone());
                reconcile_scope(
                    &app,
                    &scope,
                    &mut pending,
                    &mut seen_unknown,
                    &mut scope_failures,
                    &mut empty_tid_grace,
                    &mut hijack_reported,
                    &mut graceable,
                );
            }
            // Glem targets der ikke laengere er ukendte (blev kort, forsvandt).
            pending.retain(|id, _| seen_unknown.contains(id));
            // Glem fejl-streaks for scopes der er revet ned (ingen leak).
            scope_failures.retain(|key, _| active_scopes.contains(key));
            // Glem naade-taellere for kort der fik target/doede/forsvandt.
            empty_tid_grace.retain(|name, _| graceable.contains(name));
            hijack_reported.retain(|key| active_scopes.contains(key));
            crate::perf_mark_background!(
                "background.reconcile_tick",
                serde_json::json!({
                    "duration_ms": tick_started.elapsed().as_secs_f64() * 1_000.0,
                    "scope_count": active_scopes.len(),
                    "unknown_target_count": seen_unknown.len(),
                    "scope_failure_count": scope_failures.len(),
                }),
            );
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn reconcile_scope(
    app: &AppHandle,
    scope: &browser::ScopeInfo,
    pending: &mut HashMap<String, u32>,
    seen_unknown: &mut HashSet<String>,
    scope_failures: &mut HashMap<String, u32>,
    empty_tid_grace: &mut HashMap<String, u32>,
    hijack_reported: &mut HashSet<String>,
    graceable: &mut HashSet<String>,
) {
    let cards = browser_cards_in_scope(&scope.key);
    for (name, tid, alive) in &cards {
        if *alive && tid.is_empty() {
            graceable.insert(name.clone());
        }
    }
    // P1-lazy-keeper-gate: keeper-eksistens laeses fra tickets all_scopes()-
    // snapshot. Uden keeper er endpointet forventeligt nede — Err-armen maa
    // hverken taelle streak eller draebe (og kort kan strukturelt ikke
    // findes i et keeper-loest scope: open_browser_card kraever
    // ensure_scope_ready-succes, og heal doedsmarkerer selv foer den rydder
    // keeper-id'et — eksisterende drabs-scenarier er uaendrede).
    let has_keeper = scope.keeper_target_id.is_some();
    // M2: Ok-armen er kun autoritativ hvis samplet er TROVAERDIGT. Et svar
    // uden keeperens eget target (inkl. det tomme) modsiger sig selv og
    // haandteres som en fejl — samme keeper-id som immuniteten nedenfor
    // bruger, saa de to beslutninger aldrig kan se forskellige keepere.
    // Klassifikationen bor i trusted_sample, saa testene rammer PRAECIS den
    // kode dette tick koerer.
    let sample = trusted_sample(
        cdp_list_pages(scope.port),
        scope.keeper_target_id.as_deref(),
    );
    // Fejl-streaken foeres af ÉT kald for begge arme (`failed =
    // sample.is_none()`), saa arm-valg og debounce ikke kan komme i utakt.
    // Et trovaerdigt sample rydder streaken og giver altid `false`.
    let scope_is_dead =
        track_scope_failure(scope_failures, &scope.key, sample.is_none(), has_keeper);
    match sample {
        // Porten svarer ikke — eller svarer selvmodsigende — ⇒ processen er
        // MULIGVIS doed. Owner-beslutning 2026-07-20: kun 2 paa hinanden
        // foelgende poll-fejl ⇒ scopets levende kort doer (én langsom
        // /json-laesning maa ikke masse-draebe).
        None => {
            if scope_is_dead {
                let dead = scope_dead_names(&cards, empty_tid_grace, EMPTY_TID_GRACE_TICKS);
                for name in &dead {
                    let _ = registry::update_browser_card(name, None, None, Some(false), None);
                }
                finalize_dead(app, dead);
            }
        }
        Some(pages) => {
            // Et sundt tick nulstiller mid-creation-naaden (review F5):
            // graden maa vaere PR. fejl-episode — et flappende scope maa ikke
            // akkumulere naadetraek paa tvaers af episoder, saa et frisk kort
            // doer efter faerre end EMPTY_TID_GRACE_TICKS i sidste episode.
            for (name, tid, alive) in &cards {
                if *alive && tid.is_empty() {
                    empty_tid_grace.remove(name);
                }
            }
            let actions = reconcile_actions(
                &cards,
                &pages,
                scope.keeper_target_id.as_deref(),
                pending,
                seen_unknown,
            );
            if actions.renavigate_keeper {
                renavigate_hijacked_keeper(app, scope, hijack_reported);
            }
            for id in &actions.close_targets {
                json_close(scope.port, id);
            }
            for name in &actions.dead_names {
                let _ = registry::update_browser_card(name, None, None, Some(false), None);
            }
            finalize_dead(app, actions.dead_names);
        }
    }
}

/// En agent har navigeret keeperen udenom kort-systemet (sprang
/// `browser_card_open` over). Naviger den tilbage til sentinelen — drab
/// ville tage hele scopets browserproces — og meld kapringen til HUD'en
/// (én gang pr. scope, saa tug-of-war ikke spammer fejlkanalen).
fn renavigate_hijacked_keeper(
    app: &AppHandle,
    scope: &browser::ScopeInfo,
    hijack_reported: &mut HashSet<String>,
) {
    let label = format!("keeper-{}", scope.key);
    let sentinel = browser::keeper_sentinel(&scope.key);
    let result = on_main(app, move |a| {
        let Some(wv) = a.get_webview(&label) else {
            return Ok(());
        };
        let url: url::Url = sentinel
            .parse()
            .map_err(|e| format!("keeper sentinel url: {e}"))?;
        wv.navigate(url).map_err(|e| e.to_string())
    });
    if let Err(e) = result {
        eprintln!(
            "[canvas] keeper renavigation failed for scope {}: {e}",
            scope.key
        );
    }
    if hijack_reported.insert(scope.key.clone()) {
        let owner = scope.owner.clone().unwrap_or_else(|| "canvas".to_string());
        let _ = app.emit(
            "browser-keeper-hijacked",
            serde_json::json!({ "scope": scope.key, "owner": owner }),
        );
    }
}

fn finalize_dead(app: &AppHandle, dead: Vec<String>) {
    if dead.is_empty() {
        return;
    }
    for name in &dead {
        clear_fullscreen_if(name);
    }
    let _ = app.emit("browser-card-dead", BrowserCardDead { names: dead });
    // UBETINGET sweep (ikke kun ved fullscreen-clear): doede korts zombie-
    // webviews skal skjules med det samme, ellers daekker de dead-chromen
    // ("luk kortet") i DOM'en — webview_should_show kender nu alive-flaget.
    let _ = apply_presentation(app);
}

// ---------------------------------------------------------------------------
// MCP-ops (Task 4's BrowserCardOps, injiceret i setup)
// ---------------------------------------------------------------------------

fn close_browser_card_by_name(app: &AppHandle, name: &str) -> Result<(), String> {
    // Browser-kort er ikke workspace-persisteret (create gik uden om
    // workspace) — brug stadig samme native-foerst lifecycle som UI/voice.
    // Scopet lever videre (ejeren lukkes ad terminal-vejen). Partial-close-
    // semantikken rapporterer preclose-fejl i result.errors — for MCP's
    // single-close skal det tilbage som en haard fejl, ikke et stille Ok.
    let result = close_cards_lifecycle(app, vec![name.to_string()], registry::close_cards)?;
    if let Some(error) = result.errors.iter().find(|error| error.name == name) {
        return Err(error.message.clone());
    }
    Ok(())
}

/// Registry-/webview-backed implementering af Task 4's MCP-flade.
pub struct HostOps {
    pub app: AppHandle,
}

impl mcp::BrowserCardOps for HostOps {
    fn open(&self, session: Option<&str>, url: Option<&str>) -> Result<mcp::OpenResult, String> {
        let opened = open_browser_card(&self.app, url, session)?;
        Ok(mcp::OpenResult {
            card_number: opened.info.number,
            target_id: opened.target_id,
            cdp_endpoint: opened.cdp_endpoint,
        })
    }

    fn close(&self, session: Option<&str>, card_number: u32) -> Result<(), String> {
        let (name, owner) = browser_card_by_number(card_number)
            .ok_or_else(|| format!("no such browser card: {card_number}"))?;
        // Bloed ejerskabs-check: kun aabneren maa lukke (spec §6).
        if owner.as_deref() != session {
            return Err("not your card".to_string());
        }
        close_browser_card_by_name(&self.app, &name)
    }

    fn list(&self) -> Result<Vec<mcp::BrowserCardRow>, String> {
        Ok(browser_card_rows())
    }

    fn focus(&self, card_number: u32) -> Result<(), String> {
        let (name, _owner) = browser_card_by_number(card_number)
            .ok_or_else(|| format!("no such browser card: {card_number}"))?;
        on_main(&self.app, move |a| focus_now(a, &name))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        close_target_locks, http_get, reconcile_actions, track_scope_failure, trusted_sample,
        CLOSE_TARGET_LOCKS,
    };
    use std::collections::{HashMap, HashSet};
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::{mpsc, Arc, Barrier};
    use std::thread;
    use std::time::{Duration, Instant};

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn pages(entries: &[(&str, &str)]) -> Vec<(String, String)> {
        entries
            .iter()
            .map(|(id, url)| ((*id).to_string(), (*url).to_string()))
            .collect()
    }

    fn cdp_down() -> Result<Vec<(String, String)>, String> {
        Err("cdp list: connection refused".to_string())
    }

    /// Ét pollertick af PRODUKTIONENS egne dele, i produktionens raekkefoelge:
    /// `trusted_sample` vaelger armen, og resultatets `is_none()` er praecis
    /// det `failed`-flag `reconcile_scope` giver `track_scope_failure`.
    /// Helperen indeholder INGEN kopi af armvalget (den forrige udgave gjorde,
    /// og maalte derfor kopien i stedet for produktionen). Selve KALDET inde i
    /// `reconcile_scope` kan foerst daekkes af en test naar den funktion kan
    /// koeres uden AppHandle + levende CDP-port — indtil da er nedenstaaende
    /// den taetteste ramme paa produktionsvejen.
    /// Returnerer om scope-doeds-vejen (`scope_dead_names` + dead-emit) fyrer
    /// paa dette tick.
    fn tick_fires_scope_death(
        failures: &mut HashMap<String, u32>,
        scope_key: &str,
        sample: Result<Vec<(String, String)>, String>,
        keeper_tid: Option<&str>,
    ) -> bool {
        let trusted = trusted_sample(sample, keeper_tid);
        track_scope_failure(failures, scope_key, trusted.is_none(), keeper_tid.is_some())
    }

    // M2 (2026-07-29): vaernene mod masse-drab laa KUN i pollerens Err-arm.
    // Et succesfuldt men TOMT /json-sample gik ad Ok-armen, ryddede fejl-
    // streaken (`failed=false`) og draebte derefter hvert kort hvis target
    // manglede — uden nogen debounce overhovedet. Keeperen er selv et
    // page-target paa porten, saa et sample uden den modsiger sig selv.

    #[test]
    fn trusted_sample_rejects_keeper_less_answers_but_keeps_partial_ones() {
        let keeper = Some("KEEPER");

        // Det tomme sample: keeperen kan ikke vaere vaek mens porten svarer.
        assert!(trusted_sample(Ok(Vec::new()), keeper).is_none());
        // Ikke-tomt, men keeperens target mangler — samme selvmodsigelse.
        assert!(trusted_sample(Ok(pages(&[("T-CARD", "https://example.com/")])), keeper).is_none());
        // En Err fra CDP er uaendret utrovaerdig.
        assert!(trusted_sample(cdp_down(), keeper).is_none());

        // PARTIELT sample (keeperen ER der, et korts target mangler) er
        // trovaerdigt — bevidst valg, se doc-kommentaren — og gaar UROERT
        // videre til reconcile_actions.
        let solo = pages(&[("KEEPER", "about:blank#keeper-agent-a")]);
        assert_eq!(trusted_sample(Ok(solo.clone()), keeper), Some(solo));
        let partial = pages(&[
            ("T-CARD", "https://example.com/"),
            ("KEEPER", "about:blank"),
        ]);
        assert_eq!(trusted_sample(Ok(partial.clone()), keeper), Some(partial));

        // Lazy-vinduet (ingen keeper registreret endnu) er uaendret: uden
        // id findes intet at savne, saa intet Ok-sample kan modsige sig selv.
        assert_eq!(trusted_sample(Ok(Vec::new()), None), Some(Vec::new()));
        let no_keeper = pages(&[("T-CARD", "https://example.com/")]);
        assert_eq!(trusted_sample(Ok(no_keeper.clone()), None), Some(no_keeper));
        assert!(trusted_sample(cdp_down(), None).is_none());
    }

    #[test]
    fn empty_sample_needs_two_consecutive_ticks_before_scope_death() {
        let mut failures: HashMap<String, u32> = HashMap::new();

        // Foerste tomme sample: INGEN doeds-vej (foer M2 draebte det straks).
        assert!(!tick_fires_scope_death(
            &mut failures,
            "agent-a",
            Ok(Vec::new()),
            Some("KEEPER")
        ));
        // Anden i traek: samme debouncede vej som en doed port.
        assert!(tick_fires_scope_death(
            &mut failures,
            "agent-a",
            Ok(Vec::new()),
            Some("KEEPER")
        ));
    }

    #[test]
    fn distrusted_sample_no_longer_clears_the_failure_streak() {
        let mut failures: HashMap<String, u32> = HashMap::new();

        // Fejl-streak 1 fra en doed port, saa et keeper-loest sample: det maa
        // ikke nulstille streaken (Ok-armens `failed=false` gjorde praecis
        // det og gav et scope uendeligt liv i den ene retning og et
        // udebounced drab i den anden).
        assert!(!tick_fires_scope_death(
            &mut failures,
            "agent-a",
            cdp_down(),
            Some("KEEPER")
        ));
        assert!(tick_fires_scope_death(
            &mut failures,
            "agent-a",
            Ok(pages(&[("T-CARD", "https://example.com/")])),
            Some("KEEPER")
        ));

        // Et TROVAERDIGT sample rydder derimod stadig streaken, saa en
        // enkelt selvmodsigelse bagefter kun er foerste tick af en ny episode.
        let mut failures: HashMap<String, u32> = HashMap::new();
        assert!(!tick_fires_scope_death(
            &mut failures,
            "agent-b",
            Ok(Vec::new()),
            Some("KEEPER")
        ));
        assert!(!tick_fires_scope_death(
            &mut failures,
            "agent-b",
            Ok(pages(&[("KEEPER", "about:blank")])),
            Some("KEEPER")
        ));
        assert!(!failures.contains_key("agent-b"));
        assert!(!tick_fires_scope_death(
            &mut failures,
            "agent-b",
            Ok(Vec::new()),
            Some("KEEPER")
        ));
    }

    #[test]
    fn partial_sample_still_kills_a_single_disappeared_card_on_first_sighting() {
        // Frosset bevidst valg: naar keeperen ER i samplet, er target-listen
        // reelt afleveret, og et manglende kort-target betyder "tab lukket
        // udefra". At kraeve to observationer ville laegge 2 s oveni den
        // ENESTE doeds-detektor vi har, uden at fjerne nogen kendt fejlkilde.
        let cards = vec![
            ("card-1".to_string(), "GONE".to_string(), true),
            ("card-4".to_string(), "T-ALIVE".to_string(), true),
            // Mid-creation (intet target endnu) skaanes fortsat.
            ("card-2".to_string(), String::new(), true),
            // Allerede doedt kort re-rapporteres aldrig.
            ("card-3".to_string(), "DEAD-ALREADY".to_string(), false),
        ];
        let sample = pages(&[
            ("KEEPER", "about:blank#keeper-agent-a"),
            ("T-ALIVE", "https://example.com/"),
        ]);
        // Samme kaede som reconcile_scope: samplet skal foerst bestaa
        // trusted_sample, og det er DENS output reconcile_actions faar.
        let trusted =
            trusted_sample(Ok(sample), Some("KEEPER")).expect("keeperen er med ⇒ Ok-armen");

        let mut pending = HashMap::new();
        let mut seen = HashSet::new();
        let actions = reconcile_actions(&cards, &trusted, Some("KEEPER"), &mut pending, &mut seen);
        assert_eq!(actions.dead_names, vec!["card-1".to_string()]);
        assert!(actions.close_targets.is_empty());
    }

    #[test]
    fn close_target_locks_serialize_same_target_but_not_disjoint_targets() {
        let held_locks = close_target_locks(&names(&["p3-same-target"]));
        let held_guard = held_locks[0]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let (attempted_tx, attempted_rx) = mpsc::channel();
        let (same_acquired_tx, same_acquired_rx) = mpsc::channel();
        let same = thread::spawn(move || {
            let locks = close_target_locks(&names(&["p3-same-target"]));
            attempted_tx.send(()).unwrap();
            let _guard = locks[0]
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            same_acquired_tx.send(()).unwrap();
        });
        attempted_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        let (other_acquired_tx, other_acquired_rx) = mpsc::channel();
        let other = thread::spawn(move || {
            let locks = close_target_locks(&names(&["p3-disjoint-target"]));
            let _guard = locks[0]
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            other_acquired_tx.send(()).unwrap();
        });

        other_acquired_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("a disjoint target must not queue behind the held target");
        assert!(
            matches!(same_acquired_rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "the same target acquired before its first lifecycle released"
        );
        drop(held_guard);
        same_acquired_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("same-target waiter did not resume after release");
        same.join().unwrap();
        other.join().unwrap();
    }

    #[test]
    fn close_target_locks_sort_overlapping_batches_to_avoid_ab_ba_deadlock() {
        let barrier = Arc::new(Barrier::new(3));
        let (done_tx, done_rx) = mpsc::channel();
        let mut workers = Vec::new();
        for order in [
            names(&["p3-overlap-a", "p3-overlap-b"]),
            names(&["p3-overlap-b", "p3-overlap-a"]),
        ] {
            let barrier = Arc::clone(&barrier);
            let done_tx = done_tx.clone();
            workers.push(thread::spawn(move || {
                let locks = close_target_locks(&order);
                barrier.wait();
                let _guards: Vec<_> = locks
                    .iter()
                    .map(|lock| lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
                    .collect();
                done_tx.send(()).unwrap();
            }));
        }
        drop(done_tx);
        barrier.wait();
        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first overlapping batch deadlocked");
        done_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second overlapping batch deadlocked");
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn close_target_lock_recovers_after_poison() {
        let locks = close_target_locks(&names(&["p3-poison-target"]));
        let poisoned = Arc::clone(&locks[0]);
        assert!(thread::spawn(move || {
            let _guard = poisoned.lock().unwrap();
            panic!("synthetic close panic");
        })
        .join()
        .is_err());

        let recovered = locks[0]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        drop(recovered);
    }

    #[test]
    fn close_target_lock_map_recovers_after_poison() {
        assert!(thread::spawn(|| {
            let _guard = CLOSE_TARGET_LOCKS
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            panic!("synthetic close-lock-map panic");
        })
        .join()
        .is_err());

        let locks = close_target_locks(&names(&["p3-after-map-poison"]));
        let recovered = locks[0]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        drop(recovered);
    }

    #[test]
    fn http_get_reads_content_length_without_waiting_for_close() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (release_tx, release_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request).unwrap();
            let body = r#"[{"id":"keeper","type":"page","url":"about:blank"}]"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length:{}\r\nContent-Type:application/json\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            stream.flush().unwrap();
            // WebView2 holder forbindelsen aaben efter et komplet svar.
            let _ = release_rx.recv_timeout(Duration::from_secs(5));
        });

        let started = Instant::now();
        let response = http_get(port, "/json");
        let elapsed = started.elapsed();
        let _ = release_tx.send(());
        server.join().unwrap();

        assert_eq!(
            response.unwrap(),
            r#"[{"id":"keeper","type":"page","url":"about:blank"}]"#
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "complete response waited for socket close: {elapsed:?}"
        );
    }
}
