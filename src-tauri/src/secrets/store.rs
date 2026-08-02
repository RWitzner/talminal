//! Test-seam for keyring-adgang — KUN kompileret under `test-seams` (T1).
//!
//! HVORFOR seamet findes: tests/secrets.rs ramte indtil T1 brugerens AEGTE
//! Windows Credential Manager. Én `cargo test` var maalt til at efterlade fem
//! nye poster, og 311 `talminal-test-*`-poster havde ophobet sig i brugerens
//! credential-store — teardown-guarden slugte dengang sin fejl i et
//! `let _ = delete_secret(...)`, saa oprydningen kunne fejle tavst i aarevis
//! (den rettelse hoerer i T2; denne fil beskriver hvorfor seamet blev til, ikke
//! hvordan teardown'en ser ud i dag). Suiten maalte desuden OS'ets
//! credential-store i stedet for vores egen kontrakt, og kunne ikke koere paa
//! en maskine uden en Credential Manager overhovedet.
//!
//! HVORFOR in-memory er DEFAULT og ikke opt-in: et seam hvor den aegte keyring
//! er default, og hver test skal huske at vaelge mocken, genskaber praecis den
//! fejlklasse vi fjerner — den foerste test nogen skriver uden at kende reglen
//! skriver igen i brugerens credential-store, og intet fanger det. Her er der
//! ingen regel at huske: uden `keyring-smoke` findes der ikke en vej til den
//! aegte store i dette modul, hverken at vaelge eller at glemme.
//!
//! HVORFOR hele filen er cfg'et vaek i produktion: seamet er en
//! test-affordance, ikke en refaktorering af noeglehaandteringen. Uden
//! `test-seams` kompileres ikke én linje herfra, og secrets.rs' egen vej er
//! ordret den fra foer seamet — `Entry::new(SERVICE, key)` direkte, ingen
//! dyn-dispatch, ingen OnceLock-check og ingen ekstra allokering paa
//! store-/load-/delete-vejen.
//!
//! Bemaerk at `SERVICE` ("Talminal") derfor ALDRIG naar `Entry::new` fra en
//! test: produktionens service-navn er kompileret vaek i test-buildet, og
//! smoken bruger sit eget navn.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// In-memory-storen.
///
/// Proces-global og ikke thread-local, fordi den skal efterligne Credential
/// Manager: en noegle skrevet i én traad skal kunne laeses i en anden (og
/// `migrate_key_slots` maa kunne gaa gennem de samme tre funktioner). Tests
/// bruger unikke noeglenavne pr. koersel, saa parallelle tests i samme binaer
/// ikke deler noegler.
fn memory() -> &'static Mutex<HashMap<String, String>> {
    static STORE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// En forgiftet laas er ikke en grund til at faelde resten af suiten: en
/// panikkende test har allerede sit eget verdict, og de oevrige tests bruger
/// andre noegler.
fn lock() -> MutexGuard<'static, HashMap<String, String>> {
    memory().lock().unwrap_or_else(|p| p.into_inner())
}

/// Bevis-hook: svarer paa om noeglen ligger i IN-MEMORY-storen.
///
/// Findes for at en test kan bevise at den offentlige API skrev til mocken og
/// ikke til OS'et — uden at kunne rydde storen under foedderne paa parallelle
/// tests. Under et engageret `smoke::RealKeyring` skriver de tre funktioner
/// udenom denne map, saa svaret er `false` dér; det er med vilje.
pub fn contains_key_for_test(key: &str) -> bool {
    lock().contains_key(key.trim())
}

/// Windows' loft for en credential-blob, replikeret i mocken.
///
/// keyring-3.6.3/src/windows.rs:224 afviser en vaerdi naar
/// `password.encode_utf16().count() * 2 > CRED_MAX_CREDENTIAL_BLOB_SIZE`.
/// **Mocken SKAL fejle praecis dér.** En mock der er mildere end den ting den
/// erstatter, goer testen aktivt skadelig: en test der gemmer en ~2 kB
/// gateway-token er groen mod HashMap'en og roed i den byggede app, og
/// seamet ville skjule netop den fejl det er bygget for ikke at skjule.
const CRED_MAX_CREDENTIAL_BLOB_SIZE: usize = 2560;

/// Fejlteksten er keyrings egen (keyring-3.6.3/src/error.rs:70-73) pakket i
/// produktionens `store_secret`-indpakning, saa en assertion paa beskeden
/// holder BAADE mod mocken og mod den aegte store. `key` er den UTRIMMEDE
/// noegle, praecis som produktionens fejlstreng bruger den.
fn value_within_platform_limit(key: &str, value: &str) -> Result<(), String> {
    if value.encode_utf16().count() * 2 > CRED_MAX_CREDENTIAL_BLOB_SIZE {
        return Err(format!(
            "store_secret '{key}' failed: Attribute 'password' is longer than \
             platform limit of {CRED_MAX_CREDENTIAL_BLOB_SIZE} chars"
        ));
    }
    Ok(())
}

pub fn store_secret(key: &str, value: &str) -> Result<(), String> {
    let name = super::normalized_key(key)?;
    #[cfg(feature = "keyring-smoke")]
    {
        if smoke::selected() {
            // Smoke-vejen ER keyring — den haandhaever selv sit loft, og dens
            // egen fejl er den sande.
            return smoke::entry(name)?
                .set_password(value)
                .map_err(|e| format!("store_secret '{key}' failed: {e}"));
        }
    }
    value_within_platform_limit(key, value)?;
    lock().insert(name.to_string(), value.to_string());
    Ok(())
}

pub fn load_secret(key: &str) -> Result<Option<String>, String> {
    let name = super::normalized_key(key)?;
    #[cfg(feature = "keyring-smoke")]
    {
        if smoke::selected() {
            return match smoke::entry(name)?.get_password() {
                Ok(value) => Ok(Some(value)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(format!("load_secret '{key}' failed: {e}")),
            };
        }
    }
    Ok(lock().get(name).cloned())
}

pub fn delete_secret(key: &str) -> Result<(), String> {
    let name = super::normalized_key(key)?;
    #[cfg(feature = "keyring-smoke")]
    {
        if smoke::selected() {
            return match smoke::entry(name)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(format!("delete_secret '{key}' failed: {e}")),
            };
        }
    }
    // Idempotens som i produktion: en ukendt noegle er ikke en fejl.
    lock().remove(name);
    Ok(())
}

/// Opt-in til den AEGTE Windows Credential Manager — kun med `keyring-smoke`.
///
/// Modulet er den ENESTE vej fra en test til OS'ets credential-store, og det
/// findes ikke uden featuren. Featuren staar ikke i `default` og ikke i
/// ritualets normale job.
#[cfg(feature = "keyring-smoke")]
pub mod smoke {
    use std::cell::Cell;

    use keyring::Entry;

    /// Smoke-servicen er BEVIDST et andet navn end `secrets::SERVICE`: en
    /// smoke-koersel maa hverken laese eller overskrive brugerens rigtige
    /// noegler, og en post der alligevel bliver efterladt skal kunne kendes
    /// fra produktionens paa navnet alene.
    pub const SMOKE_SERVICE: &str = "Talminal-smoke";

    thread_local! {
        /// Thread-local og ikke proces-global: valget gaelder KUN den traad der
        /// traf det. Under libtests default (én traad pr. test) betyder det
        /// ogsaa at en smoke-test ikke kan trykke en parallel test over paa den
        /// aegte store — men den egenskab hviler paa harnessen og forsvinder
        /// under `--test-threads=1`. Det der holder UANSET er RAII-vagten
        /// nedenfor: `Drop` gendanner det forrige valg, saa det aldrig
        /// overlever sit scope. Byg ikke videre paa traad-adskillelsen alene.
        static REAL: Cell<bool> = const { Cell::new(false) };
    }

    /// RAII-vagt: den aegte Credential Manager er valgt saa laenge vagten
    /// lever. En vagt og ikke et frit `set(true)`, fordi en glemt nulstilling
    /// ville laekke valget til resten af traadens tests — og et laekket valg
    /// er praecis den fejl seamet er bygget for at umuliggoere.
    #[must_use]
    pub struct RealKeyring {
        prev: bool,
    }

    impl RealKeyring {
        pub fn engage() -> Self {
            Self {
                prev: REAL.with(|c| c.replace(true)),
            }
        }
    }

    impl Drop for RealKeyring {
        fn drop(&mut self) {
            REAL.with(|c| c.set(self.prev));
        }
    }

    pub(super) fn selected() -> bool {
        REAL.with(Cell::get)
    }

    /// Modstykket til produktionens `secrets::entry`, med smoke-servicen.
    /// Noeglen er allerede normaliseret af kalderen.
    pub(super) fn entry(key: &str) -> Result<Entry, String> {
        Entry::new(SMOKE_SERVICE, key).map_err(|e| format!("keyring entry for '{key}' failed: {e}"))
    }
}
