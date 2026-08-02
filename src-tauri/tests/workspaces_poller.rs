// Danske testnavne bruger VERSALER som betoning (suite-konvention).
#![allow(non_snake_case)]

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use talminal_canvas_lib::workspaces::status::StatusWriter;
use talminal_canvas_lib::workspaces::surface::{FakeSurface, WindowSurface};
use talminal_canvas_lib::workspaces::{self, ActiveRequest, RequestId, WorkspaceStatus};

fn vent_paa(mut betingelse: impl FnMut() -> bool, hvad: &str) {
    let frist = Instant::now() + Duration::from_secs(3);
    while Instant::now() < frist {
        if betingelse() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timeout mens der blev ventet på: {hvad}");
}

/// RAII-håndtag til den navngivne mutex `Talminal-<slug>`. Lukker handlet ved drop,
/// præcis som en rigtig proces frigiver den ved exit.
struct LevendeTarget(windows_sys::Win32::Foundation::HANDLE);

impl Drop for LevendeTarget {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// Opretter den navngivne mutex `Talminal-<slug>` (instance.rs:51-56 opretter samme
/// mutex ved en rigtig proces-opstart via `CreateMutexW`), så `instance_alive` melder
/// LEVENDE for et target der aldrig faktisk startede en proces.
///
/// Kendelse CORRECTIONS.md C-T3: et eksklusivt open af `.lock` (planens/briefens
/// oprindelige variant) racer mod en proces der er midt i `acquire_instance_lock`
/// (instance.rs:50) og kan derfor fejle og melde "kører allerede" forkert. Den
/// navngivne mutex er instance.rs' eget liveness-signal og har ingen af disse racer
/// — den forsvinder først når processen (her: testen) dør eller lukker handlet.
/// Samme klasse fejl som T16's barriere-test, hvor lukkeren vandt 149 af 150
/// kørsler bag en grøn assert: en test der "ligner" en race uden faktisk at
/// ramme den rigtige gren består af den forkerte grund.
fn hold_lock(base: &std::path::Path, slug: &str) -> LevendeTarget {
    use std::os::windows::ffi::OsStrExt;

    // Mappen oprettes stadig — status.json og andre filer for workspacet skal
    // kunne skrives under samme test, selvom liveness-signalet nu er mutexen.
    let dir = base.join("projects").join(slug);
    std::fs::create_dir_all(&dir).unwrap();

    let name = format!("Talminal-{slug}");
    let wide: Vec<u16> = std::ffi::OsStr::new(&name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        windows_sys::Win32::System::Threading::CreateMutexW(std::ptr::null(), 0, wide.as_ptr())
    };
    assert!(
        !handle.is_null(),
        "testen kunne ikke oprette target-mutexen"
    );
    LevendeTarget(handle)
}

/// Pollerens status-kanal. Briefen til Task 3 lod polleren kalde `write_status`
/// direkte og gav den derfor fem parametre; Task 6's brief (linje 115) siger
/// ordret at "pollertråden (Task 3) skifter fra `write_status` til
/// `StatusWriter::update`, så der kun er én skriver", og løfter dermed writeren
/// op i signaturen. Den nyere kontrakt vinder: PTY-readeren, registry-vejene og
/// polleren skal dele ÉN writer, fordi `StatusWriter` laver read-modify-write på
/// sin EGEN state i hukommelsen — to writere i samme proces ville hver skrive et
/// halvt billede af `status.json` oven i hinanden (jf. status.rs' modul-doc).
fn writer(base: &std::path::Path, slug: &str, instance_id: &str) -> Arc<StatusWriter> {
    Arc::new(StatusWriter::new(
        base.to_path_buf(),
        slug.into(),
        instance_id.into(),
    ))
}

fn request(seq: u64, slug: &str) -> ActiveRequest {
    ActiveRequest {
        id: RequestId {
            issuer: "test".into(),
            seq,
        },
        slug: slug.into(),
        launch_deadline: "2099-01-01T00:00:00.000Z".into(),
    }
}

#[test]
fn target_viser_sig_og_ack_er_korreleret() {
    let _g = common::serial();
    let base = tempfile::tempdir().unwrap();
    let surface = Arc::new(FakeSurface::skjult());
    let stop = Arc::new(AtomicBool::new(false));

    // Requesten står FØR polleren starter. Ellers kan første tick se en tom
    // `active_workspace.json`, fyre bootstrap-grenen (`Action::TakeOver`, som er
    // korrekt for en kold start) og skrive sin EGEN request oven i testens —
    // hvorefter ack'et bærer pollerens id i stedet for `{test, 7}`. Det ville
    // være en flaky test, ikke et fund.
    workspaces::write_active(base.path(), &request(7, "b")).unwrap();

    let w = writer(base.path(), "b", "inst-b");
    let (b, s, st) = (
        base.path().to_path_buf(),
        Arc::clone(&surface) as _,
        Arc::clone(&stop),
    );
    let handle = std::thread::spawn(move || {
        workspaces::run_poller(b, "b".into(), "inst-b".into(), w, s, st)
    });

    // `surface.reveal()` sætter visible før status-writeren når at persistére
    // ack'et. Den event-drevne fast path gør vinduet så kort, at testen skal
    // vente på det den faktisk asserterer i stedet for kun på mellemtilstanden.
    vent_paa(
        || {
            workspaces::read_status(base.path(), "b")
                .is_some_and(|status| status.acked_request == Some(request(7, "b").id))
        },
        "at target skrev sit korrelerede ack",
    );
    assert!(
        surface.is_visible(),
        "target skal være synligt når ack'et står på disk"
    );
    let status = workspaces::read_status(base.path(), "b").expect("status skrevet");
    assert_eq!(
        status.acked_request,
        Some(RequestId {
            issuer: "test".into(),
            seq: 7
        }),
        "ack'et skal bære HELE requestens identitet"
    );
    assert!(status.visible);

    stop.store(true, Ordering::SeqCst);
    handle.join().unwrap();
}

#[test]
fn kilden_skjuler_sig_IKKE_uden_kvittering_fra_et_LEVENDE_target() {
    let _g = common::serial();
    let base = tempfile::tempdir().unwrap();

    // Target LEVER (den navngivne mutex holdes), men acker aldrig DENNE request.
    //
    // ÆRLIGHED OM HVAD DENNE TEST DÆKKER (review-fund T3): mutexen er IKKE
    // load-bearing her. B's ack bærer `seq: 6` mens requesten er `seq: 7`, så
    // `ackede_denne` er falsk i protocol::decide, `target_alive` læses aldrig,
    // og kaldet falder igennem til deadline-tjekket (2099) → `Nothing`. Testen
    // ville være grøn med og uden mutexen. Den beviser derfor præcis ét:
    // **en ack der ikke bærer requestens identitet må ikke få kilden til at
    // skjule sig** — hverken via ack-grenen eller via en forhastet rollback
    // (en gammel status.json er eksplicit ikke dødsbevis i rev 2).
    //
    // Den gren hvor `target_alive` FAKTISK afgør udfaldet, dækkes af de to
    // tests nedenfor (`...giver_conceal...` / `...giver_rollback...`), som
    // kører samme fikstur med ack == request og skifter netop mutexen.
    let _lock = hold_lock(base.path(), "b");

    let a = WorkspaceStatus {
        visible: true,
        acked_request: Some(RequestId {
            issuer: "test".into(),
            seq: 6,
        }),
        pid: 1,
        instance_id: "inst-a".into(),
        ..Default::default()
    };
    workspaces::write_status(base.path(), "a", &a).unwrap();

    // B har et STALE visible:true med et FORÆLDET ack fra en tidligere session.
    let b = WorkspaceStatus {
        visible: true,
        acked_request: Some(RequestId {
            issuer: "test".into(),
            seq: 6,
        }),
        pid: 999_999,
        instance_id: "inst-b-gammel".into(),
        ..Default::default()
    };
    workspaces::write_status(base.path(), "b", &b).unwrap();
    workspaces::write_active(base.path(), &request(7, "b")).unwrap();

    let surface = Arc::new(FakeSurface::synlig());
    let stop = Arc::new(AtomicBool::new(false));
    // A's egen status flyder nu gennem writeren; filen ovenfor er fikstur fra en
    // tidligere session. Det input branch (3) faktisk hænger på — "jeg ER den
    // synlige kilde" — kommer fra `FakeSurface::synlig()`.
    let w = writer(base.path(), "a", "inst-a");
    let (base_p, s, st) = (
        base.path().to_path_buf(),
        Arc::clone(&surface) as _,
        Arc::clone(&stop),
    );
    let handle = std::thread::spawn(move || {
        workspaces::run_poller(base_p, "a".into(), "inst-a".into(), w, s, st)
    });

    std::thread::sleep(Duration::from_millis(800));
    assert!(
        surface.is_visible(),
        "kilden skjulte sig mod en forældet kvittering"
    );
    assert_eq!(surface.conceal_kald(), 0);

    stop.store(true, Ordering::SeqCst);
    handle.join().unwrap();
}

/// Fælles fikstur for de to tests nedenfor: A er den SYNLIGE kilde, requesten
/// peger på B, og B HAR kvitteret for præcis den request. Det eneste der skiller
/// dem ad, er om B's navngivne mutex holdes — altså `target_alive`.
fn kilde_a_med_target_b_der_har_acked(base: &std::path::Path) {
    let acket = RequestId {
        issuer: "test".into(),
        seq: 7,
    };
    workspaces::write_status(
        base,
        "b",
        &WorkspaceStatus {
            visible: true,
            acked_request: Some(acket),
            pid: 4242,
            instance_id: "inst-b".into(),
            ..Default::default()
        },
    )
    .unwrap();
    workspaces::write_active(base, &request(7, "b")).unwrap();
}

#[test]
fn et_LEVENDE_target_der_har_acked_faar_kilden_til_at_skjule_sig() {
    // Conceal-grenen (mod.rs) havde NUL dækning. Det er ét af de to steder hvor
    // invarianten "der er altid mindst ét synligt vindue" kan brydes, og
    // rækkefølgen er selve værnet: `visible:false` skal stå PÅ DISKEN før
    // vinduet skjules, så en samtidig poller aldrig kan se A som synlig kilde
    // efter at A er væk fra skærmen.
    let _g = common::serial();
    let base = tempfile::tempdir().unwrap();
    let _lock = hold_lock(base.path(), "b"); // <- det eneste der skiller de to tests
    kilde_a_med_target_b_der_har_acked(base.path());

    let surface = Arc::new(FakeSurface::synlig());
    let stop = Arc::new(AtomicBool::new(false));
    let w = writer(base.path(), "a", "inst-a");
    let (base_p, s, st) = (
        base.path().to_path_buf(),
        Arc::clone(&surface) as _,
        Arc::clone(&stop),
    );
    let handle = std::thread::spawn(move || {
        workspaces::run_poller(base_p, "a".into(), "inst-a".into(), w, s, st)
    });

    vent_paa(|| surface.conceal_kald() >= 1, "at kilden skjulte sig");
    let paa_disk = workspaces::read_status(base.path(), "a").expect("A's status skrevet");
    assert!(
        !paa_disk.visible,
        "visible:false skal staa PAA DISKEN allerede naar conceal er kaldt"
    );
    assert_eq!(
        surface.conceal_kald(),
        1,
        "conceal er kant-trigget, ikke pr. tick"
    );

    stop.store(true, Ordering::SeqCst);
    handle.join().unwrap();
    assert_eq!(
        surface.conceal_kald(),
        1,
        "og den maa ikke gentages efter hide()"
    );
}

#[test]
fn et_DOEDT_target_der_har_acked_giver_rollback_ikke_conceal() {
    // Rollback-grenen (mod.rs) havde ligeledes NUL dækning. Samme fikstur som
    // ovenfor — B's ack matcher requesten — men mutexen holdes IKKE. En
    // kvittering fra en proces der siden er død, må ikke få kilden til at
    // skjule sig: så ville INGEN være fremme.
    let _g = common::serial();
    let base = tempfile::tempdir().unwrap();
    kilde_a_med_target_b_der_har_acked(base.path());
    assert!(
        !workspaces::instance_alive(base.path(), "b"),
        "fiksturen kraever et doedt target"
    );

    let surface = Arc::new(FakeSurface::synlig());
    let stop = Arc::new(AtomicBool::new(false));
    let w = writer(base.path(), "a", "inst-a");
    let (base_p, s, st) = (
        base.path().to_path_buf(),
        Arc::clone(&surface) as _,
        Arc::clone(&stop),
    );
    let handle = std::thread::spawn(move || {
        workspaces::run_poller(base_p, "a".into(), "inst-a".into(), w, s, st)
    });

    vent_paa(
        || workspaces::read_active(base.path()).is_some_and(|a| a.slug == "a"),
        "at kilden tog requesten tilbage",
    );
    let nu = workspaces::read_active(base.path()).unwrap();
    assert!(
        nu.id.seq > 7,
        "rollback skal udstede en NY seq, fik {}",
        nu.id.seq
    );
    assert_eq!(nu.id.issuer, "inst-a", "og staa i kildens eget navn");
    assert_eq!(
        surface.conceal_kald(),
        0,
        "kilden maa ALDRIG skjule sig mod et doedt target"
    );

    stop.store(true, Ordering::SeqCst);
    handle.join().unwrap();
}

#[test]
fn en_fejlet_reveal_skriver_ikke_ack() {
    let _g = common::serial();
    let base = tempfile::tempdir().unwrap();
    let surface = Arc::new(FakeSurface::skjult());
    surface.fejl_ved_naeste_reveal();
    let stop = Arc::new(AtomicBool::new(false));

    // Samme grund som i den første test: requesten skal stå før polleren starter,
    // ellers kan bootstrap-grenen overskrive den.
    workspaces::write_active(base.path(), &request(7, "b")).unwrap();

    let w = writer(base.path(), "b", "inst-b");
    let (b, s, st) = (
        base.path().to_path_buf(),
        Arc::clone(&surface) as _,
        Arc::clone(&stop),
    );
    let handle = std::thread::spawn(move || {
        workspaces::run_poller(b, "b".into(), "inst-b".into(), w, s, st)
    });

    vent_paa(|| surface.reveal_kald() >= 1, "at reveal blev forsøgt");

    // Første forsøg fejlede: der må ikke stå et ack for request 7 på det tidspunkt.
    // (Næste tick lykkes, fordi fake'en kun fejler én gang — så vi tjekker at der
    // fandtes et vindue hvor ack'et var fraværende.)
    let efter_fejl = workspaces::read_status(base.path(), "b");
    assert!(
        efter_fejl.is_none() || efter_fejl.unwrap().acked_request.is_none(),
        "et ack oven på en fejlet reveal ville få kilden til at skjule sig bag ingenting"
    );

    stop.store(true, Ordering::SeqCst);
    handle.join().unwrap();
}
