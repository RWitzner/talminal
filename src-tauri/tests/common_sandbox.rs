//! Beviser at `common::serial()` sandkasser BEGGE rødder — `TALMINAL_HOME` og
//! `TALMINAL_GLOBAL_HOME` — og at ingen af dem peger ind i den levende
//! `%LOCALAPPDATA%\Talminal`. Se `tests/common/mod.rs` for hvorfor det er
//! nødvendigt: `global_base()` (project.rs) opløser `TALMINAL_GLOBAL_HOME` og
//! er roden for `settings.json`, `last_project` og `active_workspace.json`.

mod common;

#[test]
fn serial_sandkasser_baade_home_og_global_home() {
    let _g = common::serial();

    let home = std::env::var("TALMINAL_HOME").expect("TALMINAL_HOME sat");
    let global = std::env::var("TALMINAL_GLOBAL_HOME").expect("TALMINAL_GLOBAL_HOME sat");

    assert_ne!(home, global, "de to rødder må ikke være samme mappe");

    let real = std::env::var("LOCALAPPDATA").unwrap();
    let real_talminal = std::path::Path::new(&real).join("Talminal");
    for (navn, sti) in [("TALMINAL_HOME", &home), ("TALMINAL_GLOBAL_HOME", &global)] {
        assert!(
            !std::path::Path::new(sti).starts_with(&real_talminal),
            "{navn} peger ind i den levende installation: {sti}"
        );
    }
}
