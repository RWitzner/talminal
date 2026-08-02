//! Præcis én skriver af `status.json`.
//!
//! Felterne opdateres fra PTY-readeren, registry-mutationer og pollertråden.
//! Atomisk rename forhindrer torn reads, ikke lost updates — to read-modify-write
//! fra hver sin tråd ville tabe den enes ændring. Derfor holder én mutex hele
//! læs-ret-skriv-cyklussen.
//!
//! # LÅSEORDEN (bindende)
//!
//! **`update()`-closuren må ikke tage andre låse, og `StatusWriter` må aldrig
//! låses mens en `CardRuntime`-lås holdes.**
//!
//! `update` tager `self.state` FØR closuren kører. Kaldte closuren noget der
//! selv låser et kort (fx `attention_kind_now()`, der låser hvert `CardRuntime`),
//! ville låseordenen være StatusWriter → CardRuntime. Den MODSATTE orden findes
//! allerede på kort-vejene (`spawn_into` holder kortlåsen og læser derefter
//! writerens snapshot for at vide om workspacet er skjult), og de to ordener
//! sammen er en klassisk inversion: badge-tick'et og et kort-spawn kan låse
//! hinanden fast for altid. Følgeskaden er større end de to tråde — writeren
//! slippes aldrig igen, så pollertrådens synligheds-opdatering blokerer ved
//! næste tick, og hele reveal/conceal-protokollen dør med et vindue der er
//! konfigureret `visible: false`.
//!
//! Reglen i praksis: **beregn værdien før du tager writeren.**
//! `let kind = attention_kind_now(); writer.update(|s| s.attention_kind = kind);`

use super::{write_status, WorkspaceStatus};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// Den levende tilstand OG den baseline ændrings-gaten sammenligner mod. De
/// bor under SAMME lås, fordi de skal opdateres i ét stykke: en baseline der
/// kunne komme ud af trit med `current` er præcis den fejl gaten indførte.
struct Inner {
    current: WorkspaceStatus,
    /// Kroppen som SIDST ramte disken UDEN fejl. `None` = vi har aldrig
    /// skrevet. **Ikke** den sidst forsøgte — se `update`.
    skrevet: Option<WorkspaceStatus>,
}

pub struct StatusWriter {
    global_base: PathBuf,
    slug: String,
    instance_id: String,
    state: Mutex<Inner>,
}

impl StatusWriter {
    pub fn new(global_base: PathBuf, slug: String, instance_id: String) -> Self {
        Self {
            global_base,
            slug,
            instance_id,
            state: Mutex::new(Inner {
                current: WorkspaceStatus::default(),
                skrevet: None,
            }),
        }
    }

    /// **Closuren må ikke tage andre låse** — se modulets LÅSEORDEN.
    ///
    /// Skriver KUN når den meningsbærende krop faktisk ændrer sig. Uden den gate
    /// er hver eneste kalder en disk-skrivning: pollertråden kalder her 5 gange i
    /// sekundet og badge-tick'et én gang i sekundet, og `atomic::write` er
    /// `create_new` + `write_all` + **`sync_all()`** (fsync) + `rename`. En
    /// inaktiv app ville lave ~500.000 fsync'er i døgnet uden at en eneste byte
    /// af den interessante tilstand var ændret. `updated_at` holdes uden for
    /// sammenligningen, for ellers ville feltet gøre hvert billede forskelligt
    /// fra det forrige og gaten kunne aldrig fange.
    ///
    /// Feltet er sikkert at fryse: liveness læses af den navngivne mutex
    /// (`instance_alive`), ALDRIG af `updated_at`s friskhed — en gammel
    /// `status.json` er eksplicit ikke dødsbevis (protocol.rs' gren 3).
    ///
    /// # Baseline er den sidst SUCCESFULDT skrevne krop
    ///
    /// Gaten sammenligner mod `Inner::skrevet`, ikke mod den forrige værdi i
    /// hukommelsen. Gjorde den det sidste, ville en fejlet skrivning efterlade
    /// den nye krop som "allerede skrevet": næste identiske kald ville ramme
    /// "uændret ⇒ spring disken over" og aldrig prøve igen. Før gaten skrev
    /// pollerne 5 gange i sekundet og selvhelede på næste tick. Værst i
    /// ack-grenen — fejler DEN skrivning, ser peeren aldrig kvitteringen,
    /// kilden concealer aldrig, og to vinduer står fremme permanent.
    pub fn update(&self, f: impl FnOnce(&mut WorkspaceStatus)) {
        let mut guard = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let inner = &mut *guard;
        f(&mut inner.current);
        inner.current.instance_id = self.instance_id.clone();
        inner.current.pid = std::process::id();

        // Ingen baseline ⇒ vi har aldrig skrevet: filen SKAL oprettes, også når
        // kroppen tilfældigvis er lig defaultværdien.
        let uaendret = inner.skrevet.as_ref().is_some_and(|skrevet| {
            let mut sammenlign = inner.current.clone();
            sammenlign.updated_at = skrevet.updated_at.clone();
            &sammenlign == skrevet
        });
        if uaendret {
            return;
        }

        inner.current.updated_at = super::now_iso_z();
        // Skrivningen sker UNDER låsen: ellers kunne to snapshots nå disken i
        // omvendt rækkefølge og efterlade den ældste som sandhed.
        match write_status(&self.global_base, &self.slug, &inner.current) {
            // Først NU er kroppen på disken, og først nu må den bruges som
            // baseline for "uændret".
            Ok(()) => inner.skrevet = Some(inner.current.clone()),
            Err(error) => {
                // Baseline står urørt, så næste kald med samme krop skriver
                // igen. Det er retry'en pollerne altid har haft.
                eprintln!("[canvas] status.json write failed: {error}");
            }
        }
    }

    pub fn snapshot(&self) -> WorkspaceStatus {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .current
            .clone()
    }
}

/// Processens ENE writer, tilgængelig for de veje der ikke har en `AppHandle`.
///
/// `workspace.rs`' persist-hooks, exit-watcheren og kill-vejen skal alle kunne
/// genberegne kort-tallene, men ingen af dem kan nå Tauris managed state.
/// Kendelse CORRECTIONS.md C-T6 kræver desuden at hooket bor i `workspace.rs`
/// og ikke i `main.rs`' kommando-wrappers — det forudsætter en vej hertil fra
/// lib-craten.
static INSTALLED: OnceLock<Arc<StatusWriter>> = OnceLock::new();

pub fn install(writer: Arc<StatusWriter>) {
    let _ = INSTALLED.set(writer);
}

pub fn installed() -> Option<&'static Arc<StatusWriter>> {
    INSTALLED.get()
}

/// Genberegner `cards`/`running_cards` fra registryet og skriver dem.
///
/// Skal kaldes fra ENHVER vej der ændrer om et kort har en levende session —
/// ikke bare create/close. `kill_card`, `spawn_into`/`respawn_card` og
/// exit-watcheren muterer `term.pty` direkte, og uden et kald herfra bliver
/// tallet i `status.json` stående forkert indtil et tilfældigt create/close et
/// helt andet sted retter det. Tallet driver lukke-bekræftelsen, der eksplicit
/// skal skelne "kort" fra "kort med en levende session".
///
/// **Må ALDRIG kaldes mens en `CardRuntime`-lås holdes** (modulets LÅSEORDEN):
/// `list_cards()` låser hvert kort, og writeren tages bagefter.
///
/// # Tællingen og skrivningen er ÉT hele
///
/// Snapshottet skal tages uden for writer-låsen (låseordenen ovenfor er
/// bindende), men to kaldere der gør det samtidigt kan nå writeren i omvendt
/// rækkefølge af den de talte i: A tæller 0 kørende, B tæller 1, B skriver, A
/// skriver — og `status.json` står tilbage med 0 mens der kører et kort.
/// `StatusWriter`s ændrings-gate gør det VÆRRE, ikke bedre: det forkerte tal
/// bliver baseline, og næste kalder med det rigtige tal springer disken over,
/// fordi kroppen ligner den forrige. Tallet ville altså stå forkert indtil en
/// helt anden mutation tilfældigvis rettede det.
///
/// Konsekvensen er ikke kosmetisk: `running_cards` er præcis det gate der
/// afgør om en lukning spørger først. Et tal på 0 lukker et workspace med
/// levende agent-sessioner uden at spørge.
///
/// Låsen her serialiserer derfor hele læs-tæl-skriv-forløbet. Den kan ikke
/// invertere med kort-låsene: den tages KUN her, som det første, og alt der
/// låses indeni (`list_cards`, writeren) slippes igen inden den frigives.
pub fn refresh_card_counts() {
    static SERIALISERING: Mutex<()> = Mutex::new(());
    let _rækkefølge = SERIALISERING.lock().unwrap_or_else(|p| p.into_inner());

    let Some(writer) = INSTALLED.get() else {
        return;
    };
    // Registry-låsene tages og slippes HER — før writeren overhovedet røres.
    let kort = crate::registry::list_cards();
    let antal = kort.len() as u32;
    let koerende = kort
        .iter()
        .filter(|card| card.kind == "terminal" && card.running)
        .count() as u32;
    writer.update(|status| {
        status.cards = antal;
        status.running_cards = koerende;
    });
}
