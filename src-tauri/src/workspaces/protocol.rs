//! Overdragelsens beslutningslogik — rene funktioner, ingen IO og intet vindue.
//!
//! Invarianten der skal holde under alle racer: der er ALTID mindst ét synligt vindue.
//! Derfor vises target FØR kilden skjuler sig, og kilden skjuler sig kun mod en
//! kvittering der bærer PRÆCIS det request_id kilden selv bad om.

use super::{ActiveRequest, RequestId, WorkspaceStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Jeg er target og skal vise mig. Requesten følger med, så ack'et kan
    /// genvalideres mod PRÆCIS den request der udløste reveal'et.
    Reveal {
        request: ActiveRequest,
    },
    /// Target har kvitteret og lever — jeg kan skjule mig.
    Conceal,
    /// Target nåede det ikke; jeg tager requesten tilbage.
    Rollback {
        failed_slug: String,
        observed: RequestId,
    },
    /// Ingen er synlig og ingen driver noget: jeg udsteder en request til mig selv.
    TakeOver,
    Nothing,
}

pub struct Inputs<'a> {
    pub me: &'a str,
    pub my_status: &'a WorkspaceStatus,
    pub active: Option<&'a ActiveRequest>,
    pub target_status: Option<&'a WorkspaceStatus>,
    pub now: &'a str,
    pub target_alive: bool,
    /// Alle slugs med en levende `.lock`, inkl. mig selv. Bruges KUN til den
    /// deterministiske overtagelse, så to processer ikke tager over samtidigt.
    pub live_slugs: &'a [String],
    /// Er MINDST ÉN LEVENDE proces synlig lige nu? Beregnes af pollertråden som:
    /// for hvert slug i `live_slugs`, læs status.json og se om `visible == true`.
    /// KUN levende slugs tæller — en crashet proces efterlader `visible: true` i
    /// sin fil, og det er præcis det stale-signal hele korrelationen er bygget
    /// for at afvise.
    pub any_visible: bool,
}

/// Tilstandsmaskinen. Rækkefølgen er bindende — den er selve rettelsen efter
/// plan-reviewet, hvor en flad if-kæde gav fire veje til sort skærm.
pub fn decide(i: &Inputs) -> Action {
    let Some(active) = i.active else {
        // Ingen request overhovedet: allerførste opstart, eller filen er væk.
        // Vinduet starter skjult, så nogen SKAL udstede en request — ellers
        // forbliver skærmen sort. Deterministisk: laveste levende slug, og kun
        // hvis ingen står fremme (en slettet active_workspace.json må ikke lade
        // en skjult, lavere slug rive fladen fra en synlig proces).
        return if maa_overtage(i) {
            Action::TakeOver
        } else {
            Action::Nothing
        };
    };

    // (1) Jeg er target.
    if active.slug == i.me {
        let allerede_fremme =
            i.my_status.acked_request.as_ref() == Some(&active.id) && i.my_status.visible;
        return if allerede_fremme {
            Action::Nothing
        } else {
            Action::Reveal {
                request: active.clone(),
            }
        };
    }

    // (2) Jeg er ikke target og ikke synlig: normalt passiv — men er target dødt
    // OG er der ingen levende synlig, driver ingen noget. Så tager laveste over.
    //
    // `!any_visible` er ikke pynt: uden det fyrer grenen midt i en HELT NORMAL
    // kold start. Den spawnede proces opretter først sin navngivne mutex sent i
    // opstarten, så `target_alive` er falsk og slug'et mangler i `live_slugs` —
    // begge betingelser er opfyldt mens target booter. Den laveste SKJULTE slug
    // ville da rive fladen til sig, kilden ville skjule sig, og det workspace
    // brugeren faktisk bad om ville aldrig blive synligt (spec §3.3: "Fravær af
    // filer må aldrig udløse dead-target-værnet — kun launch_deadline gælder i
    // opstartsvinduet"). Så længe kilden står fremme, er `any_visible` sand, og
    // grenen kan pr. konstruktion ikke fyre.
    if !i.my_status.visible {
        let target_væk = !i.target_alive && !i.live_slugs.iter().any(|s| s == &active.slug);
        return if target_væk && maa_overtage(i) {
            Action::TakeOver
        } else {
            Action::Nothing
        };
    }

    // (3) Jeg er den synlige kilde og driver overdragelsen.
    // LIVENESS FØRST: en kvittering fra en proces der siden er død, må ikke få
    // mig til at skjule mig — så ville ingen være fremme.
    //
    // **`visible` er en del af kvitteringen, ikke pynt** (spec §3.2 punkt 3:
    // "A ser B's status med PRÆCIS `acked_request == n+1` OG `visible == true`").
    // `acked_request` ryddes ALDRIG — feltet bliver stående i target'ens
    // status.json efter at target selv har skjult sig igen. Uden `visible` er
    // id-matchet derfor kun bevis for at target EN GANG har vist sig for denne
    // request, ikke for at der står et vindue på skærmen lige nu; jeg ville
    // kunne skjule mig mod en kvittering fra et vindue der ikke er der, og
    // ingen ville være fremme. Med feltet fejler tvivlen LUKKET: jeg bliver
    // stående (to synlige vinduer er grimt, sort skærm er en fejl), og næste
    // tick retter billedet, fordi target selv skriver sin faktiske synlighed
    // ved hvert tick.
    let ackede_denne = i
        .target_status
        .is_some_and(|s| s.acked_request.as_ref() == Some(&active.id) && s.visible);

    if ackede_denne {
        return if i.target_alive {
            Action::Conceal
        } else {
            Action::Rollback {
                failed_slug: active.slug.clone(),
                observed: active.id.clone(),
            }
        };
    }

    // Ikke acket DENNE request. En gammel status.json fra en tidligere session er
    // IKKE dødsbevis — et stoppet workspace beholder sin fil, og rev 1 rullede
    // derfor tilbage før deadline, så et stoppet workspace aldrig kunne startes.
    // Kun deadline gælder her.
    if i.now > active.launch_deadline.as_str() {
        return Action::Rollback {
            failed_slug: active.slug.clone(),
            observed: active.id.clone(),
        };
    }
    Action::Nothing
}

/// Overtagelsen har to nødvendige betingelser, og de svarer til hver sin halvdel
/// af grenens formål "ingen er synlig, og ingen driver noget":
/// `!any_visible` (ingen er fremme) og `lowest_live` (kun én må reagere).
fn maa_overtage(i: &Inputs) -> bool {
    !i.any_visible && lowest_live(i)
}

/// Er jeg den laveste levende slug? FAIL-OPEN med vilje.
///
/// `live_slugs.iter().min() == Some(me)` fejler LUKKET: står `me` ikke i listen
/// — tom liste ved frisk installation, eller én fejlet dir-læsning i tick'et —
/// er svaret falsk for ALLE, ingen gren fyrer, og fordi vinduet er konfigureret
/// `visible: false` er skærmen sort indtil brugeren dræber processen. En ren
/// funktion må ikke kunne producere en uoprettelig sort skærm ud fra en
/// input-antagelse den ikke selv kan håndhæve. `all(>= me)` giver præcis samme
/// svar når kontrakten holder ("mig selv er med i listen"), og lader mig tage
/// over frem for at efterlade skærmen sort når den ikke gør.
fn lowest_live(i: &Inputs) -> bool {
    i.live_slugs.iter().all(|s| s.as_str() >= i.me)
}
