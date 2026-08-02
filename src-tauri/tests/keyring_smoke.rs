// T1 — OPT-IN smoke mod den AEGTE Windows Credential Manager.
//
// Koeres KUN eksplicit:
//   cargo test --features keyring-smoke --test keyring_smoke
//
// Targetet er erklaeret med `required-features = ["keyring-smoke"]` i
// Cargo.toml, saa ritualets normale `cargo test` slet ikke BYGGER filen —
// hverken som kode eller som en tom binaer. Det er hele pointen: den ophobning
// af 311 `talminal-test-*`-poster som T1 stopper, kom af at den aegte store var
// default-vejen for enhver test.
//
// Servicen er "Talminal-smoke" — ALDRIG "Talminal". Smoken maa hverken laese
// eller overskrive brugerens rigtige noegler, og en post der mod forventning
// bliver efterladt, skal kunne kendes fra produktionens paa navnet alene.
//
// MAALT ADVARSEL (2026-08-02, foerste koersel af denne fil): en koersel paa
// 0,11 s efterlod TO poster i credential-storen — `mig-old-a` og
// `mig-keep-old`, de to noegler med det korteste vindue mellem skrivning og
// sletning. Begge var bagefter intakte og laesbare med `cmdkey /list:<target>`
// (Type: Generic, User sat), mens keyrings egen `get_password` meldte NoEntry
// for praecis dem — testens `assert_eq!(..., None)` bestod altsaa mens posten
// laa der. En umiddelbart foelgende koersel paa 0,94 s efterlod nul.
//
// Konsekvensen for enhver der laeser denne fil: **en sletning i OS-storen kan
// ikke verificeres gennem keyring-API'et.** En teardown der "rapporterer sin
// fejl" (T2) melder stadig succes her, fordi delete_credential returnerede Ok.
// Det er den vigtigste enkeltgrund til at normale tests IKKE roerer OS-storen:
// problemet er ikke kun at teardown kan fejle tavst — det er at oprydningen kan
// vaere ufuldstaendig uden at nogen kode kan opdage det. Efter en smoke-koersel
// er `cmdkey /list` det eneste sande facit.
//
// HVAD DEN DAEKKER, og hvorfor det ikke maa krympe: da seamet flyttede
// tests/secrets.rs over paa in-memory-storen, mistede ritualet sin eneste
// udfoerte kontrol af keyrings FAKTISKE semantik. Det er ikke ligegyldigt for
// `migrate_key_slots`: den koerer ved HVER app-start (main.rs kalder
// `migrate_provider_key_slots`) og SLETTER brugerens gamle slots. En HashMap
// kan ikke fejle en sletning; Credential Manager kan. Derfor koerer migrations-
// koreografien her mod den aegte store.

use std::time::{SystemTime, UNIX_EPOCH};

use talminal_canvas_lib::secrets::store::smoke::{RealKeyring, SMOKE_SERVICE};
use talminal_canvas_lib::secrets::{
    delete_secret, load_secret, migrate_key_slots, store_secret, SERVICE,
};

fn smoke_key(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .subsec_nanos();
    format!("talminal-smoke-{tag}-{}-{nanos}", std::process::id())
}

/// Teardown: sletter noeglen i den AEGTE store. Vagten skal stadig vaere
/// engageret naar denne koerer, ellers rydder den blot in-memory-storen —
/// derfor lever `RealKeyring` laengere end `SmokeGuard` i hver test.
///
/// En efterladt post i brugerens credential-store SKAL vaere et roedt test —
/// det er hele grunden til at smoken findes. Men panikken er vaernet af
/// `std::thread::panicking()`: paniker vi i en `Drop` der koerer under
/// unwinding fra en anden panik, kalder Rust `abort()`, og saa doer hele
/// binaeren uden at printe den OPRINDELIGE fejl. Man ville staa med et
/// diagnoseloest abort i praecis det tilfaelde smoken er bygget til at
/// diagnosticere.
struct SmokeGuard(String);

impl Drop for SmokeGuard {
    fn drop(&mut self) {
        if let Err(e) = delete_secret(self.0.clone()) {
            if std::thread::panicking() {
                eprintln!("smoke-teardown af '{}' fejlede under unwind: {e}", self.0);
            } else {
                panic!("smoke-teardown af '{}' fejlede: {e}", self.0);
            }
        }
    }
}

#[test]
fn smoke_service_er_aldrig_produktionens() {
    assert_ne!(
        SMOKE_SERVICE, SERVICE,
        "smoken maa aldrig dele service-navn med produktionen"
    );
    assert_eq!(SMOKE_SERVICE, "Talminal-smoke");
}

#[test]
fn aegte_credential_manager_roundtrip() {
    let _real = RealKeyring::engage();
    let key = smoke_key("roundtrip");
    let guard = SmokeGuard(key.clone());

    store_secret(key.clone(), "sk-smoke-æøå".to_string()).expect("store_secret mod OS-store");
    assert_eq!(
        load_secret(key.clone()).expect("load_secret mod OS-store"),
        Some("sk-smoke-æøå".to_string()),
        "den aegte store skal give praecis den gemte vaerdi tilbage"
    );

    // Overskrivning: nyeste vaerdi vinder ogsaa i OS-storen (Settings-panelets
    // erstat-flow). In-memory-mocken beviser det samme, men mod en HashMap
    // hvor det er trivielt sandt.
    store_secret(key.clone(), "sk-smoke-2".to_string()).expect("re-store mod OS-store");
    assert_eq!(
        load_secret(key.clone()).expect("load efter overskrivning"),
        Some("sk-smoke-2".to_string())
    );

    delete_secret(key.clone()).expect("delete_secret mod OS-store");
    assert_eq!(
        load_secret(key.clone()).expect("load efter delete"),
        None,
        "slettet noegle skal loade som Ok(None) ogsaa i OS-storen"
    );

    drop(guard);
}

/// Den daekning ritualet MISTEDE da tests/secrets.rs flyttede paa mocken.
///
/// `migrate_key_slots`' kontrakt er "kopier alle -> verificer alle -> slet
/// foerst derefter", og det er den ENESTE kodevej der sletter brugerens
/// noegler uden at brugeren har bedt om det. Mod en HashMap kan sletningen
/// ikke fejle og `load` efter `delete` er altid `None` — altsaa beviser
/// in-memory-versionen af de fem migrationstests intet om OS'et. Fejler
/// `delete_credential` tavst i en fremtidig keyring-version, ville
/// `migrate_provider_key_slots()` kopiere ved hver opstart og aldrig faa
/// ryddet den gamle slot, mens alt var groent. Denne test er stedet hvor det
/// bliver roedt.
#[test]
fn migration_mod_den_aegte_store() {
    let _real = RealKeyring::engage();
    let old_a = smoke_key("mig-old-a");
    let new_a = smoke_key("mig-new-a");
    let keep_old = smoke_key("mig-keep-old");
    let keep_new = smoke_key("mig-keep-new");
    let dead = smoke_key("mig-dead");
    let _g1 = SmokeGuard(old_a.clone());
    let _g2 = SmokeGuard(new_a.clone());
    let _g3 = SmokeGuard(keep_old.clone());
    let _g4 = SmokeGuard(keep_new.clone());
    let _g5 = SmokeGuard(dead.clone());

    store_secret(old_a.clone(), "vaerdi-a".to_string()).expect("seed old_a");
    store_secret(keep_old.clone(), "gammel".to_string()).expect("seed keep_old");
    store_secret(keep_new.clone(), "allerede-sat".to_string()).expect("seed keep_new");
    store_secret(dead.clone(), "doed".to_string()).expect("seed dead");

    migrate_key_slots(
        &[
            (old_a.as_str(), new_a.as_str()),
            (keep_old.as_str(), keep_new.as_str()),
        ],
        &[dead.as_str()],
    )
    .expect("migration mod OS-store");

    assert_eq!(
        load_secret(new_a.clone()).expect("load new_a"),
        Some("vaerdi-a".to_string()),
        "flyttet vaerdi skal staa i den nye slot"
    );
    assert_eq!(
        load_secret(old_a.clone()).expect("load old_a"),
        None,
        "gammel slot skal vaere SLETTET i den aegte store — ikke bare i en HashMap"
    );
    assert_eq!(
        load_secret(keep_new.clone()).expect("load keep_new"),
        Some("allerede-sat".to_string()),
        "en allerede sat ny slot maa aldrig overskrives"
    );
    assert_eq!(
        load_secret(keep_old.clone()).expect("load keep_old"),
        None,
        "kilden ryddes ogsaa naar destinationen var sat i forvejen"
    );
    assert_eq!(
        load_secret(dead).expect("load dead"),
        None,
        "drop-listen skal vaere slettet i den aegte store"
    );
}

#[test]
fn valget_falder_tilbage_til_in_memory_naar_vagten_doer() {
    let key = smoke_key("fallback");
    {
        let _real = RealKeyring::engage();
        let guard = SmokeGuard(key.clone());
        store_secret(key.clone(), "i-os-storen".to_string()).expect("store mod OS-store");
        drop(guard);
    }
    // Vagten er doed: samme noegle er nu en in-memory-noegle, og den blev
    // aldrig skrevet dér. Fejler denne, laekker smoke-valget mellem tests.
    assert_eq!(
        load_secret(key).expect("load uden vagt"),
        None,
        "uden en levende vagt skal opslag gaa til in-memory-storen"
    );
}
