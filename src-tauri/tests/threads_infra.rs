mod common;

/// Beviser at test-seams-featuren faktisk er aktiv under `cargo test`. Fejler
/// den, er alle senere tasks blokeret — derfor staar den foerst.
#[test]
fn test_seams_are_visible_from_integration_tests() {
    let _g = common::serial();
    talminal_canvas_lib::threads::reset_for_test();
}

#[test]
fn temp_home_redirects_talminal_base() {
    let _g = common::serial();
    let home = common::temp_home();
    assert!(
        talminal_canvas_lib::cards::talminal_base().starts_with(home.path()),
        "TALMINAL_HOME skal styre base-mappen"
    );
}
