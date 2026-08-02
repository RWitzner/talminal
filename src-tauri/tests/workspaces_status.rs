mod common;

use std::sync::Arc;
use talminal_canvas_lib::workspaces::{self, status::StatusWriter};

#[test]
fn samtidige_opdateringer_taber_ingen_felter() {
    let base = tempfile::tempdir().unwrap();
    let writer = Arc::new(StatusWriter::new(
        base.path().to_path_buf(),
        "w-1".into(),
        "inst-1".into(),
    ));

    // To tråde opdaterer HVER SIT felt 500 gange. Uden én ejer ville read-modify-write
    // fra to sider tabe opdateringer: den ene tråds snapshot ville overskrive den andens.
    let a = Arc::clone(&writer);
    let t = std::thread::spawn(move || {
        for i in 0..500 {
            a.update(|s| s.cards = i);
        }
    });
    for i in 0..500 {
        writer.update(|s| s.running_cards = i);
    }
    t.join().unwrap();

    let paa_disk = workspaces::read_status(base.path(), "w-1").expect("status skrevet");
    assert_eq!(paa_disk.cards, 499);
    assert_eq!(paa_disk.running_cards, 499);
    assert_eq!(paa_disk.instance_id, "inst-1");
}

#[test]
fn instance_id_og_pid_saettes_ved_hver_skrivning() {
    let base = tempfile::tempdir().unwrap();
    let writer = StatusWriter::new(base.path().to_path_buf(), "w-2".into(), "inst-2".into());
    writer.update(|s| s.attention = true);

    let s = workspaces::read_status(base.path(), "w-2").unwrap();
    assert_eq!(s.instance_id, "inst-2");
    assert_eq!(s.pid, std::process::id());
    assert!(!s.updated_at.is_empty());
    assert!(s.attention);
}

#[test]
fn en_uaendret_krop_roerer_ikke_disken() {
    // Pollertraaden kalder update() 5 gange i sekundet og badge-tick'et én
    // gang i sekundet — ubetinget. `atomic::write` er create_new + write_all +
    // sync_all (FSYNC) + rename, saa uden en aendrings-gate laver en HELT
    // inaktiv app ~500.000 fsync'er i doegnet uden at en eneste byte af den
    // interessante tilstand er aendret.
    let base = tempfile::tempdir().unwrap();
    let writer = StatusWriter::new(base.path().to_path_buf(), "w-3".into(), "inst-3".into());

    writer.update(|s| s.visible = true);
    let foerste =
        workspaces::read_status(base.path(), "w-3").expect("foerste skrivning opretter filen");

    // Samme vaerdi igen, 20 gange — praecis pollerens moenster i tomgang.
    for _ in 0..20 {
        writer.update(|s| s.visible = true);
    }
    let efter = workspaces::read_status(base.path(), "w-3").unwrap();
    assert_eq!(
        efter.updated_at, foerste.updated_at,
        "uaendret krop maa ikke give en ny skrivning (updated_at ville rykke)"
    );

    // ...men en REEL aendring skal stadig ramme disken.
    writer.update(|s| s.visible = false);
    let aendret = workspaces::read_status(base.path(), "w-3").unwrap();
    assert!(!aendret.visible);
    assert_ne!(
        aendret.updated_at, foerste.updated_at,
        "en reel aendring skal skrives"
    );
}

#[test]
fn en_fejlet_skrivning_proeves_igen_ved_naeste_identiske_kald() {
    // Aendrings-gaten maa kun huske den SIDST SUCCESFULDT SKREVNE krop. Husker
    // den den sidst FORSOEGTE, staar en fejlet skrivning som "allerede skrevet"
    // i hukommelsen, og naeste identiske kald springer disken over — for evigt.
    // Foer gaten skrev pollerne 5 gange i sekundet og selvhelede paa naeste
    // tick. Vaerst i ack-grenen: fejler DEN skrivning, ser peeren aldrig
    // kvitteringen, kilden concealer aldrig, og to vinduer staar fremme
    // permanent.
    let base = tempfile::tempdir().unwrap();
    let writer = StatusWriter::new(base.path().to_path_buf(), "w-5".into(), "inst-5".into());

    // En MAPPE paa maalfilens plads faar `atomic::write`s rename til at fejle
    // ("Adgang naegtet. (os error 5)") — samme forhindring revieweren brugte.
    let maal = workspaces::status_path(base.path(), "w-5");
    std::fs::create_dir_all(&maal).unwrap();
    writer.update(|s| s.visible = true);
    assert!(
        workspaces::read_status(base.path(), "w-5").is_none(),
        "forhindringen skal have forhindret skrivningen"
    );

    // Forhindringen ryddes. Pollertraaden kalder videre med SAMME krop.
    std::fs::remove_dir(&maal).unwrap();
    for _ in 0..5 {
        writer.update(|s| s.visible = true);
    }

    let paa_disk = workspaces::read_status(base.path(), "w-5")
        .expect("en fejlet skrivning skal proeves igen ved naeste kald, ikke gates vaek");
    assert!(paa_disk.visible);
    assert!(!paa_disk.updated_at.is_empty());
}

#[test]
fn refresh_card_counts_retter_et_stale_running_cards() {
    // Kernen i T6's Critical: fire veje (kill_card, spawn/respawn, exit-watcher)
    // muterer `term.pty` uden at gaa gennem create/close, saa tallet i
    // status.json blev staaende forkert indtil et tilfaeldigt create/close et
    // helt andet sted "kom til" at rette det. Bekraeftelsesdialogen paa
    // lukkevejen laeser praecis det tal.
    let _g = common::serial();
    let base = tempfile::tempdir().unwrap();
    let writer = Arc::new(StatusWriter::new(
        base.path().to_path_buf(),
        "w-4".into(),
        "inst-4".into(),
    ));
    talminal_canvas_lib::workspaces::status::install(Arc::clone(&writer));
    assert!(
        std::ptr::eq(
            Arc::as_ptr(talminal_canvas_lib::workspaces::status::installed().unwrap()),
            Arc::as_ptr(&writer)
        ),
        "denne testbinary skal eje den installerede writer (OnceLock: foerste vinder)"
    );

    // Et STALE tal, som en draebt session ville efterlade.
    writer.update(|s| {
        s.cards = 3;
        s.running_cards = 3;
    });
    assert_eq!(
        workspaces::read_status(base.path(), "w-4")
            .unwrap()
            .running_cards,
        3
    );

    let dir = tempfile::tempdir().unwrap();
    let kort = talminal_canvas_lib::registry::create_card(
        dir.path().display().to_string(),
        "claude".to_string(),
        None,
    )
    .expect("create_card");

    talminal_canvas_lib::workspaces::status::refresh_card_counts();

    let paa_disk = workspaces::read_status(base.path(), "w-4").unwrap();
    assert_eq!(paa_disk.cards, 1, "cards skal genberegnes fra registryet");
    assert_eq!(
        paa_disk.running_cards, 0,
        "kortet har ingen PTY — det stale 3-tal skal vaere vaek"
    );

    let _ = talminal_canvas_lib::registry::close_cards(vec![kort.name]);
}
