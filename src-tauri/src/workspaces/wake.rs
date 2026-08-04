//! Lav-latens wake-up for workspace-polleren.
//!
//! Synlighedsprotokollen er fortsat filbaseret og bliver derfor ved med at
//! virke efter crash og på tværs af app-versioner. Den oprindelige poller sov
//! dog altid `POLL_MS` mellem hvert kig. Et klik skulle dermed vente på både
//! targetets næste tick (reveal + ack) og kildens næste tick (conceal).
//!
//! På Windows har hvert workspace nu et auto-reset named event. En skrivning af
//! `active_workspace.json` vækker targetet, og targetets ack vækker de øvrige
//! levende workspaces. Eventet bærer INGEN state; det er kun et hint om at læse
//! de autoritative filer nu. Fejler oprettelse eller signalering, er den gamle
//! bounded poll derfor stadig det fulde sikkerhedsnet.

use std::path::Path;
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::Duration;

/// Process-local wake-up for badge/list thread. Cross-process state remains in
/// the files; this generation only interrupts its one-second sleep when this
/// process knows that the active marker changed.
static BADGE_WAKE: LazyLock<(Mutex<u64>, Condvar)> =
    LazyLock::new(|| (Mutex::new(0), Condvar::new()));

pub struct BadgeWake {
    seen: u64,
}

impl BadgeWake {
    pub fn new() -> Self {
        let seen = *BADGE_WAKE
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self { seen }
    }

    pub fn wait(&mut self, timeout: Duration) {
        let generation = BADGE_WAKE
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (generation, _) = BADGE_WAKE
            .1
            .wait_timeout_while(generation, timeout, |current| *current == self.seen)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.seen = *generation;
    }
}

impl Default for BadgeWake {
    fn default() -> Self {
        Self::new()
    }
}

pub fn notify_badge_refresh() {
    let mut generation = BADGE_WAKE
        .0
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *generation = generation.wrapping_add(1);
    BADGE_WAKE.1.notify_all();
}

#[cfg(windows)]
mod platform {
    use super::*;
    use sha2::{Digest, Sha256};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        CreateEventW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
    };

    /// Fast navn uden rå stier eller slugs. Global-roden indgår, så parallelle
    /// tests og separate installationer ikke vækker hinandens pollere.
    fn event_name(global_base: &Path, slug: &str) -> Vec<u16> {
        let mut digest = Sha256::new();
        // Windows-stier er case-insensitive. Samme rod stavet med forskellig
        // casing skal derfor stadig pege på samme kernel-event.
        let normalized_base = global_base
            .to_string_lossy()
            .replace('/', "\\")
            .to_lowercase();
        digest.update(normalized_base.as_bytes());
        digest.update([0]);
        digest.update(slug.as_bytes());
        let hash = format!("{:x}", digest.finalize());
        crate::instance::to_wide(&format!("Local\\Talminal-WorkspaceWake-{hash}"))
    }

    pub struct PollWake {
        handle: HANDLE,
    }

    impl PollWake {
        pub fn new(global_base: &Path, slug: &str) -> Self {
            let name = event_name(global_base, slug);
            // Auto-reset: præcis den ene poller der ejer workspacet skal vækkes.
            // `false` initial state forhindrer et gratis ekstra tick ved opstart.
            let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) };
            Self { handle }
        }

        /// `true` betyder at eventet vækkede os; `false` er timeout/fejl.
        pub fn wait(&self, timeout: Duration) -> bool {
            if self.handle.is_null() {
                std::thread::sleep(timeout);
                return false;
            }
            let millis = timeout.as_millis().min(u32::MAX as u128) as u32;
            unsafe { WaitForSingleObject(self.handle, millis) == WAIT_OBJECT_0 }
        }
    }

    impl Drop for PollWake {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe {
                    CloseHandle(self.handle);
                }
            }
        }
    }

    /// Best-effort. Findes eventet ikke endnu (fx et workspace der netop
    /// spawnes), ser dets poller requesten i sit første, umiddelbare tick.
    pub fn notify(global_base: &Path, slug: &str) -> bool {
        let name = event_name(global_base, slug);
        let handle = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
        if handle.is_null() {
            return false;
        }
        let signaled = unsafe { SetEvent(handle) } != 0;
        unsafe {
            CloseHandle(handle);
        }
        signaled
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::{Arc, Barrier};

        /// Beviset er RETURVAERDIEN, ikke et stopur.
        ///
        /// Testen havde ogsaa `assert!(elapsed < 250 ms)` med teksten "den faldt
        /// sandsynligvis tilbage til polling". Den paastand kan ikke vaere sand
        /// i det oejeblik den skrives ud: falder vi tilbage, sover
        /// `PollWake::wait` hele timeouten og returnerer `false` — og saa var
        /// vi faeldet paa `vaekket` i linjen over. `WaitForSingleObject` giver
        /// kun `WAIT_OBJECT_0`, hvis eventet faktisk blev signaleret.
        ///
        /// Graensen maalte derfor ikke koden, men hvor travlt OS'et havde med
        /// at skedulere traaden igen. Paa GitHubs delte runnere faeldede den
        /// main to gange samme dag — 331,9 ms og 520,98 ms — mens de samme
        /// commits var groenne i deres PR-koersler. Og fordi jobbet stopper
        /// ved foerste roede step, naaede fmt, clippy, secret-scan og
        /// link-check aldrig at koere paa de commits.
        ///
        /// Skal latensen vogtes igen, skal graensen bindes til det den skelner
        /// IMOD — poll-intervallet — og ikke til et tal der foeles hurtigt paa
        /// en maskine man ikke ejer.
        #[test]
        fn named_event_vaekker_foer_poll_timeout() {
            let dir = tempfile::tempdir().unwrap();
            let base = dir.path().to_path_buf();
            let slug = "workspace-a";
            let klar = Arc::new(Barrier::new(2));
            let traad_klar = Arc::clone(&klar);
            let base_i_traad = base.clone();

            let handle = std::thread::spawn(move || {
                let wake = PollWake::new(&base_i_traad, slug);
                traad_klar.wait();
                // Timeouten er rundelig med vilje: den er ikke det testen
                // maaler, kun et loft saa en fejlet test doer frem for at
                // haenge. Skelnen mellem event og fallback ligger i svaret.
                wake.wait(Duration::from_secs(2))
            });

            // Barrieren passeres først EFTER CreateEventW, så OpenEventW må
            // kunne finde præcis det event polleren venter på. Kaldet går via
            // write_active (produktionsvejen), ikke direkte til notify.
            klar.wait();
            crate::workspaces::write_active(
                &base,
                &crate::workspaces::ActiveRequest {
                    id: crate::workspaces::RequestId {
                        issuer: "test".into(),
                        seq: 1,
                    },
                    slug: slug.into(),
                    launch_deadline: "2099-01-01T00:00:00.000Z".into(),
                },
            )
            .expect("active request + wake-up");
            let vaekket = handle.join().unwrap();
            assert!(
                vaekket,
                "polleren sov timeouten ud i stedet for at blive vaekket af eventet \
                 — den filbaserede fallback baerer nu hele latensen"
            );
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub struct PollWake;

    impl PollWake {
        pub fn new(_global_base: &Path, _slug: &str) -> Self {
            Self
        }

        pub fn wait(&self, timeout: Duration) -> bool {
            std::thread::sleep(timeout);
            false
        }
    }

    pub fn notify(_global_base: &Path, _slug: &str) -> bool {
        false
    }
}

pub use platform::{notify, PollWake};
