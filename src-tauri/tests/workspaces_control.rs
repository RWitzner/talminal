use talminal_canvas_lib::workspaces::control;

#[test]
fn en_lukkeanmodning_hentes_praecis_en_gang() {
    let base = tempfile::tempdir().unwrap();
    control::request_close(base.path(), "b-2222").unwrap();

    let request =
        control::take_pending(base.path(), "b-2222").expect("første poll skal se anmodningen");
    assert_eq!(request.action, "close");
    assert!(
        control::take_pending(base.path(), "b-2222").is_none(),
        "anden poll må ikke se den igen — ellers lukker vi to gange"
    );
}

#[test]
fn gentagne_anmodninger_er_idempotente_indtil_de_hentes() {
    let base = tempfile::tempdir().unwrap();
    control::request_close(base.path(), "b-2222").unwrap();
    control::request_close(base.path(), "b-2222").unwrap();
    control::request_close(base.path(), "b-2222").unwrap();

    let request = control::take_pending(base.path(), "b-2222").unwrap();
    assert_eq!(request.action, "close");
    assert!(
        control::take_pending(base.path(), "b-2222").is_none(),
        "tre klik må give én lukning"
    );
}

#[test]
fn quit_all_anmodningen_hentes_med_sin_egen_action() {
    let base = tempfile::tempdir().unwrap();
    control::request_quit_all(base.path(), "b-2222").unwrap();

    let request = control::take_pending(base.path(), "b-2222").unwrap();
    assert_eq!(request.action, "quit_all");
    assert!(control::take_pending(base.path(), "b-2222").is_none());
}

#[test]
fn en_ventende_quit_all_kan_ikke_nedgraderes_til_close() {
    let base = tempfile::tempdir().unwrap();
    control::request_quit_all(base.path(), "b-2222").unwrap();
    control::request_close(base.path(), "b-2222").unwrap();

    let request = control::take_pending(base.path(), "b-2222").unwrap();
    assert_eq!(request.action, "quit_all");
}

#[test]
fn ingen_anmodning_er_none() {
    let base = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(base.path().join("projects").join("b-2222")).unwrap();
    assert!(control::take_pending(base.path(), "b-2222").is_none());
}
