//! Raadne-vagt for testsuitens datamappe.
//!
//! Sandkassen i `common::serial()` er usynlig naar den virker: intet fejler,
//! ingen mappe vokser, ingen ser noget. Fjerner nogen wiringen igen — eller
//! aendrer `talminal_base()`s opløsning — er der derfor ingen der maerker det
//! foer ejerens `%LOCALAPPDATA%\Talminal` er beskidt og runbookens §0.3
//! maaler oven paa skrald. Disse to tests er det der raaber i stedet.
//!
//! De asserter mekanismen, ikke en enkelt fil: at datamappen er FLYTTET, og at
//! traad-arkivet — den ene ting der faktisk blev forurenet — foelger med.

mod common;

use std::path::{Path, PathBuf};

use talminal_canvas_lib::cards;
use talminal_canvas_lib::threads::archive;

/// Den rigtige installations datamappe, opløst som appen selv gør det uden
/// test-override'et. `LOCALAPPDATA` roeres ikke af sandkassen, saa den er den
/// sande reference at maale afstand fra.
fn real_install_base() -> PathBuf {
    let localappdata = std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .expect("LOCALAPPDATA must be set (Windows-only app)");
    PathBuf::from(localappdata).join("Talminal")
}

#[test]
fn serial_flytter_datamappen_vaek_fra_den_rigtige_installation() {
    let _g = common::serial();

    let base = cards::talminal_base();
    assert_ne!(
        base,
        real_install_base(),
        "en test opløste datamappen til ejerens RIGTIGE installation — \
         sandkassen i tests/common/mod.rs::serial() er vaek eller virker ikke"
    );
    assert!(
        base.starts_with(Path::new(env!("CARGO_TARGET_TMPDIR"))),
        "datamappen skal ligge under target/tmp (cargo clean rydder den), men var {}",
        base.display()
    );
}

#[test]
fn traad_arkivet_lander_i_sandkassen() {
    let _g = common::serial();

    // Det er denne sti der blev forurenet: archive::threads_dir() ->
    // talminal_base()/threads. Assertionen ligger paa arkivet selv og ikke
    // kun paa basen, saa en fremtidig ekstra kilde til stien ogsaa fanges.
    let dir = archive::threads_dir();
    assert!(
        !dir.starts_with(real_install_base()),
        "traad-arkivet pegede paa den rigtige installation: {}",
        dir.display()
    );
    assert!(
        dir.starts_with(Path::new(env!("CARGO_TARGET_TMPDIR"))),
        "traad-arkivet laa uden for sandkassen: {}",
        dir.display()
    );
}
