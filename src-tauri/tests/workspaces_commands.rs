// Danske testnavne bruger VERSALER som betoning (suite-konvention).
#![allow(non_snake_case)]

mod common;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use talminal_canvas_lib::workspaces::{self, WorkspaceState};

fn skriv_projekt(base: &std::path::Path, slug: &str, navn: &str) {
    let dir = base.join("projects").join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("project.json"),
        format!(
            r#"{{"root":"C:\\r\\{navn}","name":"{navn}","added_at":"2026-07-01T00:00:00.000Z"}}"#
        ),
    )
    .unwrap();
}

const NU: &str = "2026-07-26T10:00:00.000Z";
const SENERE: &str = "2026-07-26T10:00:30.000Z";

fn request(slug: &str, deadline: &str) -> workspaces::ActiveRequest {
    workspaces::ActiveRequest {
        id: workspaces::RequestId {
            issuer: "test".into(),
            seq: 1,
        },
        slug: slug.into(),
        launch_deadline: deadline.into(),
    }
}

fn find<'a>(s: &'a [workspaces::WorkspaceSummary], slug: &str) -> &'a workspaces::WorkspaceSummary {
    s.iter().find(|w| w.slug == slug).unwrap()
}

#[test]
fn stoppede_workspaces_uden_request_er_stopped() {
    let base = tempfile::tempdir().unwrap();
    skriv_projekt(base.path(), "a-1111", "alpha");
    skriv_projekt(base.path(), "b-2222", "beta");

    let s = workspaces::summaries(base.path(), None, NU);
    assert_eq!(s[0].state, WorkspaceState::Stopped);
    assert_eq!(s[1].state, WorkspaceState::Stopped);
    assert!(!s[0].is_active);
}

#[test]
fn en_udstedt_men_ukvitteret_request_giver_STARTING() {
    // Rev 1 kunne aldrig producere denne tilstand: summaries kendte kun slug'en,
    // ikke requesten, så rail'en havde ingen måde at vise "er på vej".
    let base = tempfile::tempdir().unwrap();
    skriv_projekt(base.path(), "b-2222", "beta");

    let s = workspaces::summaries(base.path(), Some(&request("b-2222", SENERE)), NU);
    assert_eq!(find(&s, "b-2222").state, WorkspaceState::Starting);
    assert!(!find(&s, "b-2222").is_active, "starting er ikke aktiv");
}

#[test]
fn en_request_hvis_deadline_er_loebet_ud_giver_FAILED() {
    let base = tempfile::tempdir().unwrap();
    skriv_projekt(base.path(), "b-2222", "beta");

    let s = workspaces::summaries(base.path(), Some(&request("b-2222", NU)), SENERE);
    assert_eq!(find(&s, "b-2222").state, WorkspaceState::Failed);
}

#[test]
fn aktiv_kraever_bade_kvittering_og_synlighed() {
    let base = tempfile::tempdir().unwrap();
    skriv_projekt(base.path(), "b-2222", "beta");
    let r = request("b-2222", SENERE);

    // Kvitteret, men endnu ikke synlig
    let mut status = workspaces::WorkspaceStatus {
        acked_request: Some(r.id.clone()),
        visible: false,
        instance_id: "i".into(),
        ..Default::default()
    };
    workspaces::write_status(base.path(), "b-2222", &status).unwrap();
    assert!(!find(&workspaces::summaries(base.path(), Some(&r), NU), "b-2222").is_active);

    status.visible = true;
    workspaces::write_status(base.path(), "b-2222", &status).unwrap();
    assert!(find(&workspaces::summaries(base.path(), Some(&r), NU), "b-2222").is_active);
}

#[test]
fn badge_felter_og_rod_sti_kommer_med_paa_wiren() {
    // Mutexen er ikke pynt: efter slutreview B3 er de statusafledte felter
    // liveness-gatede, saa "kommer med paa wiren" kan kun bevises for et LEVENDE
    // workspace. Modstykket staar i testen lige nedenfor.
    let _g = common::serial();
    let base = tempfile::tempdir().unwrap();
    let slug = unik_slug("wire");
    skriv_projekt(base.path(), &slug, "alpha");
    let _levende = hold_mutex(&slug);
    let status = workspaces::WorkspaceStatus {
        cards: 3,
        running_cards: 2,
        attention: true,
        instance_id: "i".into(),
        ..Default::default()
    };
    workspaces::write_status(base.path(), &slug, &status).unwrap();

    let s = workspaces::summaries(base.path(), None, NU);
    let w = find(&s, &slug);
    assert_eq!(w.cards, 3);
    assert_eq!(w.running_cards, 2);
    assert!(w.attention);
    assert_eq!(
        w.attention_kind,
        workspaces::AttentionKind::NeedsYou,
        "legacy attention:true uden kind skal eskalere"
    );
    assert_eq!(
        w.root.as_deref(),
        Some(r"C:\r\alpha"),
        "rail'en lover fuld sti i title"
    );
}

#[test]
fn et_stoppet_workspace_baerer_hverken_attention_eller_koerende_kort() {
    // SLUTREVIEW B3. `status.attention` har PRAECIS én skriver — `run_badge_tick`
    // — og loekken stoppes af det delte stop-flag uden nogensinde at skrive en
    // afsluttende `attention = false`. Filen overlever processen.
    //
    // Prikken taendes desuden KUN mens workspacet er skjult (attention.rs' egen
    // gate), hvilket er praecis den situation hvor ejeren derefter lukker posten
    // fra rail'en. Uden liveness-gaten stod den ravgule "venter paa dig"-prik
    // saa PERMANENT paa et doedt workspace — og `dotStyle`/`statusText`
    // (WorkspaceRail.tsx) laeser `attention` FOER `state`, saa den undertrykte
    // ogsaa skaermlaeserens "ikke aaben". Featurens vigtigste signal blev stoej.
    //
    // `running_cards` er med i samme gate: tallet gater luk-bekraeftelsen i
    // frontenden, saa ✕ paa et doedt workspace spurgte om kort der ikke findes.
    let base = tempfile::tempdir().unwrap();
    let slug = unik_slug("doed");
    skriv_projekt(base.path(), &slug, "doed");
    workspaces::write_status(
        base.path(),
        &slug,
        &workspaces::WorkspaceStatus {
            cards: 3,
            running_cards: 2,
            attention: true,
            visible: true,
            instance_id: "i".into(),
            ..Default::default()
        },
    )
    .unwrap();

    // Ingen mutex holdes: processen er doed, men dens status.json ligger der.
    let s = workspaces::summaries(base.path(), None, NU);
    let w = find(&s, &slug);
    assert_eq!(
        w.state,
        WorkspaceState::Stopped,
        "forudsaetningen: posten er nede"
    );
    assert!(
        !w.attention,
        "en doed proces maa ikke kunne kalde paa ejeren gennem sin efterladte fil"
    );
    assert_eq!(
        w.running_cards, 0,
        "der koerer ingen sessioner i en proces der ikke findes"
    );
    // `cards` er projektets kort paa disken — de kommer tilbage naar workspacet
    // startes igen — og forbliver derfor UGATEDE. Rail'ens taller er ikke et
    // liveness-signal.
    assert_eq!(
        w.cards, 3,
        "kortantallet hoerer til projektet, ikke til sessionen"
    );
}

#[test]
fn skjulte_poster_er_med_i_summaries_men_markerede() {
    let base = tempfile::tempdir().unwrap();
    skriv_projekt(base.path(), "a-1111", "alpha");
    workspaces::listing::hide(base.path(), "a-1111").unwrap();

    // Frontenden filtrerer — backenden skjuler ikke information.
    let s = workspaces::summaries(base.path(), None, NU);
    assert_eq!(s.len(), 1);
    assert!(s[0].hidden);
}

// ---------------------------------------------------------------------------
// Liveness i summaries: RUNNING og ABA
// ---------------------------------------------------------------------------

/// RAII-håndtag til den navngivne mutex `Talminal-<slug>` — samme signal som
/// `instance.rs::acquire_instance_lock` opretter ved en rigtig proces-opstart,
/// og dermed det `instance_alive` læser.
struct Levende(windows_sys::Win32::Foundation::HANDLE);

impl Drop for Levende {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

fn hold_mutex(slug: &str) -> Levende {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = std::ffi::OsStr::new(&format!("Talminal-{slug}"))
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        windows_sys::Win32::System::Threading::CreateMutexW(std::ptr::null(), 0, wide.as_ptr())
    };
    assert!(!handle.is_null(), "kunne ikke oprette test-mutexen");
    Levende(handle)
}

/// Slug'et er maskinbredt (mutex-navnet er ikke rod-namespaced), så det skal
/// være unikt pr. proces for ikke at kollidere med en parallel testbinary.
fn unik_slug(navn: &str) -> String {
    format!("{navn}-{}", std::process::id())
}

#[test]
fn et_levende_workspace_er_RUNNING_ogsaa_uden_request() {
    // `Running` blev aldrig produceret i nogen test — hele liveness-grenen i
    // summaries (mod.rs) var udækket, selvom den er den eneste kilde til
    // rail'ens "kører"-markering for et workspace ingen lige har klikket på.
    let _g = common::serial();
    let base = tempfile::tempdir().unwrap();
    let slug = unik_slug("run");
    skriv_projekt(base.path(), &slug, "koerende");

    let s = workspaces::summaries(base.path(), None, NU);
    assert_eq!(
        find(&s, &slug).state,
        WorkspaceState::Stopped,
        "uden mutex: stoppet"
    );

    let _levende = hold_mutex(&slug);
    let s = workspaces::summaries(base.path(), None, NU);
    assert_eq!(find(&s, &slug).state, WorkspaceState::Running);
    assert!(
        !find(&s, &slug).is_active,
        "running uden kvittering er ikke AKTIV"
    );
}

#[test]
fn et_GAMMELT_ack_gaelder_ikke_den_nye_request() {
    // ABA: status.json bærer et ack fra en TIDLIGERE request. Det er hele
    // grunden til at `ackede_denne` sammenligner hele `RequestId` og ikke bare
    // slug'et — ellers ville rail'en vise posten som aktiv, mens den i
    // virkeligheden er ved at starte op på en helt ny request.
    let _g = common::serial();
    let base = tempfile::tempdir().unwrap();
    let slug = unik_slug("aba");
    skriv_projekt(base.path(), &slug, "aba");
    let _levende = hold_mutex(&slug);

    let gammel = workspaces::RequestId {
        issuer: "test".into(),
        seq: 1,
    };
    workspaces::write_status(
        base.path(),
        &slug,
        &workspaces::WorkspaceStatus {
            acked_request: Some(gammel),
            visible: true,
            instance_id: "i".into(),
            ..Default::default()
        },
    )
    .unwrap();

    // NY request (seq 2) mod samme slug — samme udsteder, nyt nummer.
    let ny = workspaces::ActiveRequest {
        id: workspaces::RequestId {
            issuer: "test".into(),
            seq: 2,
        },
        slug: slug.clone(),
        launch_deadline: SENERE.into(),
    };
    let s = workspaces::summaries(base.path(), Some(&ny), NU);
    assert!(
        !find(&s, &slug).is_active,
        "et ack for en FORAELDET request maa ikke tælle som kvittering"
    );
    assert_eq!(
        find(&s, &slug).state,
        WorkspaceState::Starting,
        "posten er paa vej frem paa den nye request, ikke allerede fremme"
    );
}

// ---------------------------------------------------------------------------
// Command-fladen
// ---------------------------------------------------------------------------

/// Peger `global_base()` på en frisk mappe. Skal holdes sammen med
/// `common::serial()`-guarden — env er proces-global.
fn global_home(_guard: &std::sync::MutexGuard<'static, ()>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("TALMINAL_GLOBAL_HOME", dir.path());
    dir
}

#[test]
fn ukendte_slugs_afvises_foer_nogen_sti_samles() {
    // `kendt_slug` er path-traversal-værnet: uvalideret ville "../.." række uden
    // for datamappen. Værnet havde nul tests, så en refaktor der fjernede
    // kaldet fra én af kommandoerne ville efterlade suiten grøn.
    let g = common::serial();
    let base = global_home(&g);
    skriv_projekt(base.path(), "a-1111", "alpha");

    for slug in ["../..", r"..\..", "a-1111/../..", "findes-ikke"] {
        // Ogsaa den BEKRAEFTEDE vej valideres: bekraeftelsen er brugerens svar
        // paa et spoergsmaal om kort, ikke en tilladelse til at forlade basen.
        for confirmed in [false, true] {
            let fejl = workspaces::commands::set_workspace_hidden(slug.into(), true, confirmed)
                .expect_err(&format!(
                    "set_workspace_hidden accepterede {slug:?} (confirmed={confirmed})"
                ));
            assert!(fejl.starts_with("ukendt workspace"), "fik {fejl:?}");
        }

        for confirmed in [false, true] {
            let fejl = workspaces::commands::request_close_workspace(slug.into(), confirmed)
                .expect_err(&format!(
                    "request_close_workspace accepterede {slug:?} (confirmed={confirmed})"
                ));
            assert!(fejl.starts_with("ukendt workspace"), "fik {fejl:?}");
        }

        let fejl = workspaces::commands::activate_workspace(slug.into())
            .expect_err(&format!("activate_workspace accepterede {slug:?}"));
        assert!(fejl.starts_with("ukendt workspace"), "fik {fejl:?}");
    }

    // ...og intet blev skrevet uden for basen.
    assert!(!base
        .path()
        .join("projects")
        .join("a-1111")
        .join(".hidden")
        .exists());
}

#[test]
fn aktivering_af_det_allerede_synlige_workspace_er_et_no_op() {
    let g = common::serial();
    let base = global_home(&g);
    let slug = unik_slug("allerede-aktiv");
    skriv_projekt(base.path(), &slug, "allerede-aktiv");
    let _levende = hold_mutex(&slug);
    let active = request(&slug, SENERE);
    workspaces::write_active(base.path(), &active).unwrap();
    workspaces::write_status(
        base.path(),
        &slug,
        &workspaces::WorkspaceStatus {
            acked_request: Some(active.id.clone()),
            visible: true,
            instance_id: "inst-aktiv".into(),
            ..Default::default()
        },
    )
    .unwrap();

    workspaces::commands::activate_workspace(slug).expect("aktiv række er en gyldig no-op");

    assert_eq!(
        workspaces::read_active(base.path()),
        Some(active),
        "et klik på den aktive række må ikke udstede et nyt request-id"
    );
}

#[test]
fn fjern_fra_listen_river_ikke_et_workspace_med_koerende_sessioner_ned() {
    // Spec: lukning sker "med bekraeftelse hvis der er kort med koerende
    // sessioner", og "Fjern fra listen" skal lukke "ad samme vej" som ✕.
    // Uden gaten sad to knapper paa samme raede med samme destruktive
    // konsekvens, og kun den ene spurgte.
    let g = common::serial();
    let base = global_home(&g);
    let slug = unik_slug("beta");
    skriv_projekt(base.path(), &slug, "beta");
    let _levende = hold_mutex(&slug);
    workspaces::write_status(
        base.path(),
        &slug,
        &workspaces::WorkspaceStatus {
            running_cards: 2,
            cards: 3,
            instance_id: "i".into(),
            ..Default::default()
        },
    )
    .unwrap();

    let fejl = workspaces::commands::set_workspace_hidden(slug.clone(), true, false)
        .expect_err("to levende sessioner skal kraeve bekraeftelse");
    assert_eq!(
        fejl,
        format!("{}:2", workspaces::commands::KRAEVER_BEKRAEFTELSE)
    );
    assert!(
        workspaces::control::take_pending(base.path(), &slug).is_none(),
        "der maa ikke vaere skrevet en close-anmodning"
    );
    assert!(
        !base
            .path()
            .join("projects")
            .join(&slug)
            .join(".hidden")
            .exists(),
        "posten maa ikke vaere skjult"
    );
}

#[test]
fn rail_ens_kryds_kraever_bekraeftelse_paa_BACKENDENS_tal() {
    // Slutreview-fund (Codex P1-1): frontenden afgjorde selv om der skulle
    // spørges, ud fra `running_cards` i `workspaces-changed`-listen. Den liste
    // udsendes på badge-kadencen (1 s), så et kort der startede i det sekund var
    // usynligt for beslutningen: rail'en så 0 kørende, sprang dialogen over og
    // lukkede et workspace med en levende agent-session uden at spørge.
    // Beslutningen hører hjemme dér hvor tallet er sandt.
    let g = common::serial();
    let base = global_home(&g);
    let slug = unik_slug("kryds");
    skriv_projekt(base.path(), &slug, "kryds");
    let _levende = hold_mutex(&slug);
    workspaces::write_status(
        base.path(),
        &slug,
        &workspaces::WorkspaceStatus {
            running_cards: 2,
            cards: 3,
            instance_id: "i".into(),
            ..Default::default()
        },
    )
    .unwrap();

    let fejl = workspaces::commands::request_close_workspace(slug.clone(), false)
        .expect_err("to levende sessioner skal kraeve bekraeftelse");
    assert_eq!(
        fejl,
        format!("{}:2", workspaces::commands::KRAEVER_BEKRAEFTELSE)
    );
    assert!(
        workspaces::control::take_pending(base.path(), &slug).is_none(),
        "afvisningen maa ikke have skrevet en close-anmodning"
    );

    // Brugerens svar kommer tilbage — og NU lukkes der.
    workspaces::commands::request_close_workspace(slug.clone(), true)
        .expect("et bekraeftet kryds skal lukke");
    assert!(
        workspaces::control::take_pending(base.path(), &slug).is_some(),
        "et bekraeftet kryds skal skrive anmodningen"
    );
}

#[test]
fn rail_ens_kryds_spoerger_ikke_naar_ingen_session_koerer() {
    // Modstykket — og den positive kontrol for gaten ovenfor: uden kørende
    // sessioner er der intet at miste, og en dialog ville være støj.
    let g = common::serial();
    let base = global_home(&g);
    let slug = unik_slug("krydstom");
    skriv_projekt(base.path(), &slug, "krydstom");
    let _levende = hold_mutex(&slug);
    workspaces::write_status(
        base.path(),
        &slug,
        &workspaces::WorkspaceStatus {
            running_cards: 0,
            cards: 4,
            instance_id: "i".into(),
            ..Default::default()
        },
    )
    .unwrap();

    workspaces::commands::request_close_workspace(slug.clone(), false)
        .expect("ingen koerende sessioner: ingen dialog");
    assert!(
        workspaces::control::take_pending(base.path(), &slug).is_some(),
        "der skal stadig lukkes"
    );
}

#[test]
fn et_stoppet_workspace_lukkes_uden_at_spoerge_og_uden_landmine() {
    // Et stoppet workspace har intet at lukke. Bekræftelsen må ikke fyre på
    // efterladenskaberne i dets `status.json` (tallet er fra sidste session), og
    // der må ikke skrives en anmodning som en fremtidig opstart ville finde og
    // lukke sig selv på.
    let g = common::serial();
    let base = global_home(&g);
    let slug = unik_slug("stoppet");
    skriv_projekt(base.path(), &slug, "stoppet");
    workspaces::write_status(
        base.path(),
        &slug,
        &workspaces::WorkspaceStatus {
            running_cards: 5,
            cards: 5,
            instance_id: "doed".into(),
            ..Default::default()
        },
    )
    .unwrap();

    workspaces::commands::request_close_workspace(slug.clone(), false)
        .expect("en stoppet post maa ikke give en fejlbesked");
    assert!(
        workspaces::control::take_pending(base.path(), &slug).is_none(),
        "der maa ikke ligge en anmodning og vente paa naeste opstart"
    );
}

#[test]
fn fjern_fra_listen_lukker_uden_dialog_naar_ingen_session_koerer() {
    // Modstykket: 0 koerende sessioner er ikke destruktivt, saa der er intet at
    // bekraefte — men processen skal stadig lukkes, ellers efterlader vi en
    // usynlig proces brugeren ikke laengere kan naa.
    let g = common::serial();
    let base = global_home(&g);
    let slug = unik_slug("gamma");
    skriv_projekt(base.path(), &slug, "gamma");
    let _levende = hold_mutex(&slug);
    workspaces::write_status(
        base.path(),
        &slug,
        &workspaces::WorkspaceStatus {
            running_cards: 0,
            cards: 2,
            instance_id: "i".into(),
            ..Default::default()
        },
    )
    .unwrap();

    workspaces::commands::set_workspace_hidden(slug.clone(), true, false).expect("skal lykkes");
    assert!(
        workspaces::control::take_pending(base.path(), &slug).is_some(),
        "et levende workspace skal stadig bedes lukke"
    );
    assert!(base
        .path()
        .join("projects")
        .join(&slug)
        .join(".hidden")
        .exists());
}

#[test]
fn en_bekraeftet_fjernelse_skjuler_posten_i_selve_kaldet() {
    // SLUTREVIEW B1. Bekraeftelsen kommer TILBAGE hertil, og skjulningen skal
    // ske HER — synkront, i samme kald som lukningen.
    //
    // Foer rettelsen svarede frontenden med et raat `request_close_workspace` og
    // armerede skjulningen i React-state, som ventede paa at rail'ens liste
    // meldte posten `stopped` eller `hidden`. For ejerens EGET workspace kan den
    // betingelse pr. konstruktion aldrig indtraeffe foer processen doer:
    // `summaries` udleder `state` af `alive`, og jeg er alive hele vejen gennem
    // overdragelsen (op til HANDOFF_TIMEOUT) og gennem ExitRequested-teardown.
    // Staten doede med processen, og `.hidden` blev aldrig skrevet: knappen
    // gjorde alt undtagen det den hedder.
    let g = common::serial();
    let base = global_home(&g);
    let slug = unik_slug("bekraeftet");
    skriv_projekt(base.path(), &slug, "bekraeftet");
    let _levende = hold_mutex(&slug);
    workspaces::write_status(
        base.path(),
        &slug,
        &workspaces::WorkspaceStatus {
            running_cards: 2,
            cards: 3,
            visible: true,
            instance_id: "i".into(),
            ..Default::default()
        },
    )
    .unwrap();

    // Ubekraeftet afvises stadig — gaten er ikke fjernet, kun besvaret.
    workspaces::commands::set_workspace_hidden(slug.clone(), true, false)
        .expect_err("uden bekraeftelse skal den stadig afvise");
    assert!(
        !base
            .path()
            .join("projects")
            .join(&slug)
            .join(".hidden")
            .exists(),
        "afvisningen maa ikke have skrevet noget"
    );

    // Brugeren svarede ja.
    workspaces::commands::set_workspace_hidden(slug.clone(), true, true)
        .expect("en bekraeftet fjernelse skal lykkes");

    assert!(
        base.path()
            .join("projects")
            .join(&slug)
            .join(".hidden")
            .exists(),
        "skjulningen skal vaere skrevet af BACKENDEN — den er det eneste sted \
         hvor handlingen overlever processens doed"
    );
    assert!(
        workspaces::control::take_pending(base.path(), &slug).is_some(),
        "og workspacet skal stadig bedes lukke"
    );
    // Og listen melder posten skjult med det samme — ingen ventetilstand.
    let s = workspaces::summaries(base.path(), None, NU);
    assert!(find(&s, &slug).hidden);
}

#[test]
fn hent_frem_kraever_ingen_bekraeftelse_og_lukker_intet() {
    // Modstykket til gaten: "Hent frem" er ikke destruktivt, saa `confirmed`
    // skal vaere uden betydning dér — og der maa ALDRIG skrives en
    // luk-anmodning paa den vej.
    let g = common::serial();
    let base = global_home(&g);
    let slug = unik_slug("hentfrem");
    skriv_projekt(base.path(), &slug, "hentfrem");
    let _levende = hold_mutex(&slug);
    workspaces::write_status(
        base.path(),
        &slug,
        &workspaces::WorkspaceStatus {
            running_cards: 4,
            cards: 4,
            instance_id: "i".into(),
            ..Default::default()
        },
    )
    .unwrap();
    workspaces::listing::hide(base.path(), &slug).unwrap();

    workspaces::commands::set_workspace_hidden(slug.clone(), false, false)
        .expect("hent frem maa aldrig afvises");
    assert!(!base
        .path()
        .join("projects")
        .join(&slug)
        .join(".hidden")
        .exists());
    assert!(
        workspaces::control::take_pending(base.path(), &slug).is_none(),
        "hent frem maa ikke lukke noget"
    );
}

// ---------------------------------------------------------------------------
// run_badge_tick
// ---------------------------------------------------------------------------

#[test]
fn badge_tick_skriver_attention_og_emitter_KUN_ved_diff() {
    // Kendelse CORRECTIONS C.T8 tvang loekken fra main.rs ned i lib-craten med
    // begrundelsen "logik skal kunne naas fra tests/" — og saa blev den ikke
    // naaet. En implementation der emitterede hvert sekund uden diff (praecis
    // det frossen kontrakt B forbyder) bestod hele suiten.
    let _g = common::serial();
    let base = tempfile::tempdir().unwrap();
    let slug = unik_slug("badge");
    skriv_projekt(base.path(), &slug, "badge");

    let writer = Arc::new(talminal_canvas_lib::workspaces::status::StatusWriter::new(
        base.path().to_path_buf(),
        slug.clone(),
        "inst-badge".into(),
    ));
    let stop = Arc::new(AtomicBool::new(false));
    let emits = Arc::new(AtomicU32::new(0));

    let (b, w, s, e) = (
        base.path().to_path_buf(),
        Arc::clone(&writer),
        Arc::clone(&stop),
        Arc::clone(&emits),
    );
    let handle = std::thread::spawn(move || {
        workspaces::run_badge_tick(b, w, s, move |liste| {
            assert!(!liste.is_empty(), "listen skal baere projekterne");
            e.fetch_add(1, Ordering::SeqCst);
        })
    });

    // (a) attention skrives gennem writeren, (b) der emitteres ved foerste tick.
    let frist = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while emits.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < frist {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        emits.load(Ordering::SeqCst),
        1,
        "foerste tick skal emittere"
    );
    let status = workspaces::read_status(base.path(), &slug).expect("attention skrevet");
    assert_eq!(
        status.instance_id, "inst-badge",
        "skrevet gennem DENNE writer"
    );
    assert!(!status.attention, "intet kort i registryet venter");

    // (c) INTET emit naar listen er uaendret — to ekstra tick.
    std::thread::sleep(std::time::Duration::from_millis(
        workspaces::BADGE_TICK_MS * 2 + 300,
    ));
    assert_eq!(
        emits.load(Ordering::SeqCst),
        1,
        "uaendret liste maa ikke give et event pr. sekund"
    );

    // ...men en ny post SKAL give præcis ét.
    skriv_projekt(base.path(), &unik_slug("badge2"), "badge2");
    let wake_start = std::time::Instant::now();
    workspaces::wake::notify_badge_refresh();
    let frist = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while emits.load(Ordering::SeqCst) < 2 && std::time::Instant::now() < frist {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        emits.load(Ordering::SeqCst),
        2,
        "en ny post skal give ét event"
    );

    // (d) stop-flaget afslutter loekken.
    assert!(
        wake_start.elapsed() < std::time::Duration::from_millis(500),
        "et eksplicit workspace-skift skal afbryde badge-trådens 1 s sleep"
    );
    stop.store(true, Ordering::SeqCst);
    let start = std::time::Instant::now();
    handle.join().unwrap();
    assert!(
        start.elapsed() < std::time::Duration::from_secs(3),
        "stop-flaget skal afslutte loekken inden for ét tick"
    );
}
