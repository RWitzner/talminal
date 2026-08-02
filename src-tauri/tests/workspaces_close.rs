//! Lukkeprotokollens tilstandsmaskine.
//!
//! Hvorfor testene bor HER og ikke i main.rs: `WindowEvent::CloseRequested` kører
//! på event-loopet, og planens Step 3 skrev forløbet direkte ind i handleren.
//! `tests/` kan ikke nå binary-craten, så den variant var utestbar — og en
//! utestet tilstandsmaskine bag et vindueslukke er præcis det sted hvor en fejl
//! først opdages når brugeren ikke kan lukke sit vindue. Kendelse CORRECTIONS.md
//! C-T11 fryser derfor rettelsen: maskinen er en REN funktion i lib'en, og
//! main.rs' handler kalder kun den og udfører resultatet.

mod common;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use talminal_canvas_lib::workspaces::close::{self, CloseAction, ClosePhase, CloseState};
use talminal_canvas_lib::workspaces::control;
use talminal_canvas_lib::workspaces::status::StatusWriter;
use talminal_canvas_lib::workspaces::surface::FakeSurface;

fn slugs(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_string()).collect()
}

// --- Den rene overgangsfunktion ------------------------------------------

#[test]
fn idle_med_koerende_kort_spoerger_foerst() {
    assert_eq!(
        close::next_phase(ClosePhase::Idle, 3),
        (
            ClosePhase::Confirming,
            CloseAction::PreventAndConfirm { running: 3 }
        ),
        "kørende kort må aldrig stoppes uden at brugeren har sagt ja"
    );
}

#[test]
fn idle_uden_koerende_kort_spoerger_ogsaa_foerst() {
    assert_eq!(
        close::next_phase(ClosePhase::Idle, 0),
        (
            ClosePhase::Confirming,
            CloseAction::PreventAndConfirm { running: 0 }
        ),
        "titel-X lukker hele Talminal og skal derfor altid bekræftes"
    );
}

#[test]
fn approved_lader_lukningen_passere() {
    // Anden runde: `start_handoff_then_close` har kaldt `window.close()` igen,
    // og NU skal eventet passere uhindret videre til den normative
    // ExitRequested-teardown. Preventer vi her, kan vinduet aldrig lukkes.
    assert_eq!(
        close::next_phase(ClosePhase::Approved, 7),
        (ClosePhase::Approved, CloseAction::Allow),
        "kørende kort må IKKE kunne standse en allerede godkendt lukning"
    );
}

#[test]
fn gentagne_close_requested_starter_ikke_forfra() {
    // Brugeren trykker ✕ tre gange mens dialogen står åben: fasen skal blive
    // stående i `Confirming` — ét spørgsmål, ét svar, én lukning.
    let (fase, _) = close::next_phase(ClosePhase::Confirming, 3);
    assert_eq!(fase, ClosePhase::Confirming);

    assert_eq!(
        close::next_phase(ClosePhase::HandingOff, 3),
        (ClosePhase::HandingOff, CloseAction::Prevent),
        "en igangværende overdragelse må ikke startes forfra af et nyt ✕"
    );
    assert_eq!(
        close::next_phase(ClosePhase::Quitting, 3),
        (ClosePhase::Quitting, CloseAction::Prevent),
        "en igangværende global lukning må ikke starte en ny dialog"
    );
}

#[test]
fn et_nyt_forsoeg_i_confirming_stiller_spoergsmaalet_igen() {
    // Spørgsmålet stilles med et EVENT, og et event har ingen leveringsgaranti:
    // lander `close-confirm-requested` før App.tsx har registreret sin lytter
    // (registreringen er asynkron efter mount), findes der ingen dialog og
    // dermed intet `confirm_close` — og et rent `Prevent` her ville låse
    // vinduet i `Confirming` for evigt, hvor hvert eneste senere ✕/Alt+F4 blot
    // afvises. Genfremsættelsen er den eneste vej ud der ikke går uden om
    // brugerens samtykke.
    //
    // To dialoger bliver det ikke af: App.tsx' kø tager højst én
    // `application`-post (`queue.some(kind === "application")`).
    assert_eq!(
        close::next_phase(ClosePhase::Confirming, 3),
        (
            ClosePhase::Confirming,
            CloseAction::PreventAndConfirm { running: 3 }
        ),
    );
}

// --- Svaret fra frontenden ------------------------------------------------

#[test]
fn et_ja_starter_global_lukning() {
    assert_eq!(
        close::on_confirm(ClosePhase::Confirming, true),
        (ClosePhase::Quitting, CloseAction::QuitAll),
    );
}

#[test]
fn et_nej_ruller_tilbage_til_idle() {
    // Uden denne gren bliver vinduet stående i Confirming, og et nyt ✕ svarer
    // kun `Prevent`: vinduet kunne aldrig lukkes igen.
    assert_eq!(
        close::on_confirm(ClosePhase::Confirming, false),
        (ClosePhase::Idle, CloseAction::Prevent),
    );
}

#[test]
fn et_forsinket_svar_kan_ikke_rulle_et_igangvaerende_forloeb_tilbage() {
    // En stale dialog (fx et peer-luk der overhalede brugerens dialog) må ikke
    // kunne sætte fasen tilbage til Idle midt i en overdragelse — så ville
    // `Approved` aldrig blive nået og `window.close()` prellede af for evigt.
    for fase in [
        ClosePhase::HandingOff,
        ClosePhase::Quitting,
        ClosePhase::Approved,
        ClosePhase::Idle,
    ] {
        let (efter, _) = close::on_confirm(fase, false);
        assert_eq!(efter, fase, "{fase:?} må ikke ændres af et ubedt svar");
        let (efter, _) = close::on_confirm(fase, true);
        assert_eq!(efter, fase, "{fase:?} må ikke ændres af et ubedt svar");
    }
}

// --- Hvem overtager skærmen? ---------------------------------------------

#[test]
fn overdragelsen_springes_over_naar_jeg_ikke_er_synlig() {
    // Et SKJULT workspace der lukkes fjernstyret må ikke udstede en request:
    // det ville rive fladen væk fra den proces brugeren faktisk kigger på.
    assert_eq!(
        close::handoff_target(false, &slugs(&["a", "b", "c"]), "b"),
        None,
    );
}

#[test]
fn naermeste_er_den_naeste_i_listen_ellers_den_forrige() {
    let live = slugs(&["a", "b", "c"]);
    assert_eq!(close::handoff_target(true, &live, "a"), Some("b".into()));
    assert_eq!(close::handoff_target(true, &live, "b"), Some("c".into()));
    assert_eq!(
        close::handoff_target(true, &live, "c"),
        Some("b".into()),
        "sidste post har ingen efterfølger — så tages den forrige"
    );
}

#[test]
fn ingen_andre_levende_betyder_at_appen_bare_lukker() {
    assert_eq!(close::handoff_target(true, &slugs(&["a"]), "a"), None);
    assert_eq!(close::handoff_target(true, &[], "a"), None);
}

// --- Tilstandsholderen ----------------------------------------------------

#[test]
fn hele_forloebet_gennem_holderen() {
    let state = CloseState::new(std::path::PathBuf::from("."), "a".into());

    assert_eq!(
        state.on_close_requested(2),
        CloseAction::PreventAndConfirm { running: 2 }
    );
    assert_eq!(
        state.on_close_requested(2),
        CloseAction::PreventAndConfirm { running: 2 },
        "andet tryk genstiller spørgsmålet — fasen står stille, men et tabt \
         event må ikke kunne låse vinduet"
    );
    assert_eq!(
        state.phase(),
        ClosePhase::Confirming,
        "genfremsættelsen må ikke starte forløbet forfra"
    );
    assert_eq!(state.on_confirm(false), CloseAction::Prevent);
    assert_eq!(
        state.phase(),
        ClosePhase::Idle,
        "et nej frigiver vinduet igen"
    );

    assert_eq!(
        state.on_close_requested(0),
        CloseAction::PreventAndConfirm { running: 0 },
        "også en tom Talminal-instans skal advare før global lukning"
    );
    assert_eq!(state.on_confirm(true), CloseAction::QuitAll);
    assert_eq!(state.phase(), ClosePhase::Quitting);
    assert_eq!(
        state.on_close_requested(2),
        CloseAction::Prevent,
        "mens quit-all kører må et nyt CloseRequested ikke starte forfra"
    );
}

#[test]
fn en_mislykket_overdragelse_frigiver_vinduet_igen() {
    // Successoren kvitterede ikke inden for fristen, så lukningen blev AFLYST
    // (`hand_off` → `Handoff::Afbrudt`). Blev fasen stående i `HandingOff`,
    // ville vinduet være ulukkeligt resten af sessionen: hvert nyt ✕ svarer
    // `Prevent`, og der kommer aldrig et `approve()`. Brugeren skal kunne
    // trykke igen.
    let state = CloseState::new(std::path::PathBuf::from("."), "a".into());
    assert!(state.begin_peer_close());
    assert_eq!(state.phase(), ClosePhase::HandingOff);

    state.abort();

    assert_eq!(state.phase(), ClosePhase::Idle);
    assert!(
        state.begin_peer_close(),
        "et nyt ✕ skal kunne forsøge overdragelsen igen"
    );
}

#[test]
fn et_peer_luk_springer_bekraeftelsen_over() {
    // Rail'en i den SYNLIGE proces har allerede spurgt brugeren. Spurgte vi
    // igen her, ville spørgsmålet lande i et skjult vindue ingen kan se, og
    // lukningen ville hænge i Confirming for evigt.
    let state = CloseState::new(std::path::PathBuf::from("."), "a".into());
    assert!(
        state.begin_peer_close(),
        "peer-luk skal starte overdragelsen"
    );
    assert_eq!(state.phase(), ClosePhase::HandingOff);
    assert!(
        !state.begin_peer_close(),
        "to control-anmodninger må give én lukning"
    );
}

#[test]
fn et_peer_luk_overtager_en_aaben_bekraeftelse() {
    let state = CloseState::new(std::path::PathBuf::from("."), "a".into());
    assert_eq!(
        state.on_close_requested(1),
        CloseAction::PreventAndConfirm { running: 1 }
    );
    assert!(state.begin_peer_close());
    assert_eq!(state.phase(), ClosePhase::HandingOff);
    // Brugeren når at trykke Annullér på den nu forældede dialog.
    assert_eq!(state.on_confirm(false), CloseAction::Prevent);
    assert_eq!(
        state.phase(),
        ClosePhase::HandingOff,
        "det forældede nej må ikke afbryde peer-lukningen"
    );
}

#[test]
fn et_peer_quit_vinder_over_handoff_og_er_idempotent() {
    let state = CloseState::new(std::path::PathBuf::from("."), "a".into());
    assert!(state.begin_peer_close());
    assert_eq!(state.phase(), ClosePhase::HandingOff);

    assert!(
        state.begin_peer_quit(),
        "quit-all skal kunne overhale en lokal workspace-overdragelse"
    );
    assert_eq!(state.phase(), ClosePhase::Quitting);
    assert!(
        !state.begin_peer_quit(),
        "gentagne quit-all-anmodninger må kun starte teardown én gang"
    );
    assert!(
        !state.begin_peer_close(),
        "en senere workspace-close må ikke nedgradere quit-all til handoff"
    );
}

#[test]
fn approve_overskriver_ikke_quitting() {
    let state = CloseState::new(std::path::PathBuf::from("."), "a".into());
    assert!(state.begin_peer_quit());

    assert!(
        !state.approve(),
        "en sen handoff-kvittering må ikke godkende en global quit som lokal close"
    );
    assert_eq!(state.phase(), ClosePhase::Quitting);

    let handoff = CloseState::new(std::path::PathBuf::from("."), "b".into());
    assert!(handoff.begin_peer_close());
    assert!(
        handoff.approve(),
        "et rigtigt handoff skal stadig kunne godkendes"
    );
    assert_eq!(handoff.phase(), ClosePhase::Approved);
}

// --- Pollerens hook -------------------------------------------------------

#[test]
fn pollertraaden_dispatcher_close_og_quit_all_for_sit_eget_slug() {
    let _g = common::serial();
    let base = tempfile::tempdir().unwrap();

    // Seamen er proces-global (main.rs registrerer den én gang i setup), så den
    // sættes her og kun her i denne testbinary.
    static CLOSE_FYRINGER: AtomicU32 = AtomicU32::new(0);
    static QUIT_FYRINGER: AtomicU32 = AtomicU32::new(0);
    close::set_peer_close_hook(Box::new(|| {
        CLOSE_FYRINGER.fetch_add(1, Ordering::SeqCst);
    }));
    close::set_peer_quit_hook(Box::new(|| {
        QUIT_FYRINGER.fetch_add(1, Ordering::SeqCst);
    }));

    control::request_close(base.path(), "b").unwrap();
    // En anmodning til et ANDET slug må polleren ikke røre.
    control::request_close(base.path(), "c").unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let writer = Arc::new(StatusWriter::new(
        base.path().to_path_buf(),
        "b".into(),
        "inst-b".into(),
    ));
    let handle = {
        let (b, s, st) = (
            base.path().to_path_buf(),
            Arc::new(FakeSurface::synlig()) as Arc<_>,
            Arc::clone(&stop),
        );
        std::thread::spawn(move || {
            talminal_canvas_lib::workspaces::run_poller(
                b,
                "b".into(),
                "inst-b".into(),
                writer,
                s,
                st,
            )
        })
    };

    let frist = Instant::now() + Duration::from_secs(3);
    while CLOSE_FYRINGER.load(Ordering::SeqCst) == 0 && Instant::now() < frist {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        CLOSE_FYRINGER.load(Ordering::SeqCst),
        1,
        "polleren skal fyre lukningen præcis én gang"
    );

    control::request_quit_all(base.path(), "b").unwrap();
    let frist = Instant::now() + Duration::from_secs(3);
    while QUIT_FYRINGER.load(Ordering::SeqCst) == 0 && Instant::now() < frist {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        QUIT_FYRINGER.load(Ordering::SeqCst),
        1,
        "polleren skal sende quit_all til sin egen hook"
    );
    assert_eq!(
        CLOSE_FYRINGER.load(Ordering::SeqCst),
        1,
        "quit_all må ikke fejldispatches som workspace-close"
    );

    // Et par ticks mere: filerne er fjernet af `take_pending`, så der må ikke
    // komme flere fyringer.
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(CLOSE_FYRINGER.load(Ordering::SeqCst), 1);
    assert_eq!(QUIT_FYRINGER.load(Ordering::SeqCst), 1);
    assert!(
        control::take_pending(base.path(), "c").is_some(),
        "et andet workspaces anmodning må ikke være spist"
    );

    stop.store(true, Ordering::SeqCst);
    handle.join().unwrap();
}
