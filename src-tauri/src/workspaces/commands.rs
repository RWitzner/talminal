//! Tauri-fladen for workspace-rail'en — tynde wrappers, al logik i lib'en.
//!
//! To ting er bindende her (plan Global Constraints):
//! - **Ingen IPC-flade tager en eksekverbar sti som parameter.** `activate_workspace`
//!   opløser exe'en internt; ellers var kommandoen arbitrær proces-spawn.
//! - **Slugs fra frontenden valideres mod den FAKTISKE liste før path-join.**
//!   En rå slug i et `Path::join` er path-traversal.

use super::WorkspaceSummary;
use std::path::Path;

#[tauri::command]
pub fn list_workspaces() -> Vec<WorkspaceSummary> {
    let base = crate::project::global_base();
    let active = crate::workspaces::read_active(&base);
    crate::workspaces::summaries(&base, active.as_ref(), &crate::workspaces::now_iso_z())
}

/// Slug'en skal findes i den faktiske liste — ikke bare "se rigtig ud".
fn kendt_slug(base: &Path, slug: &str) -> Result<(), String> {
    crate::workspaces::listing::list(base)
        .iter()
        .any(|e| e.slug == slug)
        .then_some(())
        .ok_or_else(|| format!("ukendt workspace: {slug}"))
}

#[tauri::command]
pub fn activate_workspace(slug: String) -> Result<(), String> {
    let base = crate::project::global_base();
    kendt_slug(&base, &slug)?;
    let state_dir = base.join("projects").join(&slug);

    // Klik på rækken brugeren allerede står i er et ægte no-op. Uden gaten
    // udstedte vi et nyt request-id, så polleren kørte reveal/show/focus igen,
    // `workspace-revealed` refetchede hele canvas-snapshottet, og rail'en stod
    // lokalt pending selv om intet skulle skifte.
    if crate::workspaces::instance_alive(&base, &slug) {
        if let (Some(active), Some(status)) = (
            crate::workspaces::read_active(&base),
            crate::workspaces::read_status(&base, &slug),
        ) {
            if active.slug == slug
                && status.acked_request.as_ref() == Some(&active.id)
                && status.visible
            {
                return Ok(());
            }
        }
    }

    // Kører den? Overdrag forgrunds-retten før requesten skrives — et skjult
    // vindue kan ikke selv aktivere sig, og Windows nægter cross-process
    // aktivering uden denne overdragelse.
    if crate::workspaces::instance_alive(&base, &slug) {
        if let Some(status) = crate::workspaces::read_status(&base, &slug) {
            crate::workspaces::surface::allow_foreground_for(status.pid);
        }
    } else {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        crate::workspaces::spawn_workspace(&exe, &state_dir)?;
    }

    let request = crate::workspaces::next_request(
        &base,
        &slug,
        crate::workspaces::my_instance_id(),
        crate::workspaces::deadline_from_now(),
    );
    crate::workspaces::write_active(&base, &request)
}

/// "+ Tilføj projekt". Mappevælgeren ER handlingen: den valgte mappe
/// registreres, hentes frem i listen og aktiveres i én bevægelse. Uden det
/// sidste skridt ville "+" blot lægge en post i listen som brugeren derefter
/// selv skulle klikke på.
///
/// **Vejen er ren Rust** — ingen npm-pakke, ingen frontend-permission. Dialogen
/// kaldes fra kommandoens krop, som ligger uden for capability-ACL'en; derfor
/// står der intet om `dialog:` i `capabilities/default.json`.
///
/// **`async` er ikke kosmetik.** `blocking_pick_folder` blokerer den kaldende
/// tråd indtil brugeren har valgt, og Tauri kører SYNKRONE commands på
/// hovedtråden. En `pub fn` her ville blokere event-loopet mens mappevælgeren
/// selv pumper beskeder — frossen app.
///
/// **Annullering er `Ok(None)`, aldrig `Err`.** Frontendens `workspaceCommand`
/// viser en fejlnotits på `catch`, og "jeg fortrød" er ikke en fejl.
#[tauri::command]
pub async fn add_workspace(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let Some(valgt) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let sti = valgt.into_path().map_err(|error| error.to_string())?;
    // `find_project_root` går OP til nærmeste `.git`: vælger brugeren
    // `repo\packages\web`, registreres `repo`. Det er spec'ens adfærd — og det
    // betyder at "tilføj" af et projekt man allerede har, skal være IDEMPOTENT
    // frem for at fejle. Derfor overskriver vi metaen og aktiverer, i stedet for
    // at afvise en eksisterende post.
    let root = crate::project::find_project_root(&sti)?;
    let slug = crate::project::project_slug(&root)?;
    let state_dir = crate::project::project_state_dir(&root)?;
    let navn = root
        .file_name()
        .ok_or_else(|| format!("mappen har intet navn: {}", root.display()))?
        .to_string_lossy()
        .into_owned();

    // `added_at` BEVARES ved genregistrering. `write_project_meta` overskriver
    // hele filen, og `added_at` er primærnøglen i `listing::list`s sortering —
    // et ubetinget `now` ville sende et projekt du tilføjer igen nederst i listen
    // og bryde den stabile rækkefølge.
    let added_at = crate::project::read_project_meta(&state_dir)
        .ok()
        .flatten()
        .and_then(|meta| meta.added_at)
        .or_else(|| Some(crate::workspaces::now_iso_z()));

    // RÆKKEFØLGEN ER BINDENDE. `activate_workspace` starter med `kendt_slug`,
    // som slår op i `listing::list` — og listen læses fra
    // `projects/*/project.json`. Aktiveres der før metaen er skrevet, fejler den
    // med "ukendt workspace: …", og brugeren har tilføjet et projekt der ikke
    // åbner. `unhide` skal ligeledes ligge før, så et projekt der tidligere er
    // fjernet fra listen faktisk kommer frem igen.
    crate::project::write_project_meta(
        &state_dir,
        &crate::project::ProjectMeta {
            root,
            name: navn,
            added_at,
        },
    )
    .map_err(|error| error.to_string())?;
    let base = crate::project::global_base();
    crate::workspaces::listing::unhide(&base, &slug)?;
    // Vælger brugeren mappen for det workspace der kører LIGE NU, bliver
    // aktiveringen en request til vores egen poller. Harmløst — og ikke en fejl.
    activate_workspace(slug.clone())?;
    Ok(Some(slug))
}

/// Fejlpræfikset frontenden skal genkende: "kør ✕-flowet først".
/// Formen er `kraever_bekraeftelse:<antal kørende kort>`.
pub const KRAEVER_BEKRAEFTELSE: &str = "kraever_bekraeftelse";

/// Den destruktive gate for ALLE veje der lukker et kørende workspace.
///
/// **Gaten hører hjemme i backenden, ikke i rail'ens liste.** Frontenden kender
/// kun `running_cards` fra `workspaces-changed`, som udsendes på badge-kadencen
/// (1 s) og derfor er op til et sekund gammel. Startede et kort i det sekund,
/// ville frontenden se 0 kørende kort, springe bekræftelsen over og lukke et
/// workspace med en levende agent-session uden at spørge. Tallet skal læses på
/// beslutningstidspunktet, i den proces der ejer sandheden.
///
/// `Ok(())` betyder "kør bare"; `Err(kraever_bekraeftelse:<n>)` betyder "spørg
/// først og kald igen med `confirmed: true`". Formen er FROSSEN — frontendens
/// `confirmationDemand` (src/workspaces.ts) parser præfikset, og begge
/// kommandoer nedenfor deler den.
fn bekraeftelse_mangler(base: &Path, slug: &str, confirmed: bool) -> Result<(), String> {
    if confirmed {
        return Ok(());
    }
    let koerende = crate::workspaces::read_status(base, slug)
        .map(|status| status.running_cards)
        .unwrap_or(0);
    if koerende > 0 {
        return Err(format!("{KRAEVER_BEKRAEFTELSE}:{koerende}"));
    }
    Ok(())
}

/// `hidden`-sidecaren, og — når posten er et kørende workspace — lukningen af
/// den.
///
/// **`confirmed` er brugerens svar, ikke en genvej.** Kommandoen kaldes to
/// gange i den destruktive vej: først med `confirmed: false`, så backenden kan
/// afvise med `kraever_bekraeftelse:<n>` og dialogen stilles på DENNE proces'
/// tal frem for på et gæt fra rail'ens liste; og efter brugerens "ja" med
/// `confirmed: true`, som udfører begge dele.
///
/// **Skjulningen SKAL ske her og ikke i frontenden** (slutreview B1). "Fjern
/// fra listen" rammer også det workspace brugeren SELV sidder i — `instance_alive`
/// er sand for en selv — og en skjulning der armeres i React-state og først
/// udføres når posten melder sig nede, kan pr. konstruktion aldrig udføres dér:
/// jeg er alive hele vejen gennem overdragelsen (op til `HANDOFF_TIMEOUT`) og
/// gennem `ExitRequested`-teardown, og så er staten væk med processen. Ejeren
/// kunne altså trykke "Fjern", besvare en destruktiv bekræftelse, miste sine
/// agent-sessioner — og finde posten uændret i listen ved næste opstart.
///
/// **Rækkefølgen er bindende: luk-anmodningen FØRST, sidecaren derefter.**
/// Fejler `request_close`, skjules der intet — en skjult post med levende
/// agenter brugeren ikke længere kan nå er præcis det denne vej findes for at
/// undgå. Der er ingen kapløb i mellemrummet: begge er filskrivninger i samme
/// synkrone kommando, mens den modtagende poller først tikker efter op til
/// `POLL_MS`, og for MIT EGET slug kan lukningen ikke gennemføres før
/// kommandoen har returneret — `window.close()` behandles på event-loopet, som
/// er optaget af netop dette kald.
#[tauri::command]
pub fn set_workspace_hidden(slug: String, hidden: bool, confirmed: bool) -> Result<(), String> {
    let base = crate::project::global_base();
    kendt_slug(&base, &slug)?;
    if !hidden {
        return crate::workspaces::listing::unhide(&base, &slug);
    }
    // Et kørende workspace lukkes FØRST — ellers ville "Fjern fra listen"
    // efterlade en usynlig proces med levende agenter som brugeren ikke længere
    // kan nå.
    //
    // Lukningen skal ske "ad samme vej" som ✕ (spec §punkt 7 og :211), og den
    // vej har en BEKRÆFTELSE når der er kort med kørende sessioner. Uden gaten
    // nedenfor sad to knapper ved siden af hinanden på samme række med samme
    // destruktive konsekvens, og kun den ene spurgte. Bekræftelsen selv hører
    // hjemme i den SYNLIGE proces (App.tsx) — et spørgsmål stillet i et skjult
    // vindue kan ingen svare på.
    if crate::workspaces::instance_alive(&base, &slug) {
        bekraeftelse_mangler(&base, &slug, confirmed)?;
        crate::workspaces::control::request_close(&base, &slug)?;
    }
    crate::workspaces::listing::hide(&base, &slug)
}

/// Bed et workspace lukke sig selv — rail'ens ✕. Gælder BÅDE et andet (evt.
/// skjult) workspace og det brugeren selv sidder i.
///
/// **Kun et LEVENDE workspace får en anmodning.** Skrev vi filen for et stoppet
/// workspace, ville den blive liggende som en landmine: næste gang brugeren
/// aktiverede projektet, ville processen starte, pollerens første tick ville
/// finde anmodningen og lukke vinduet igen. Der er intet at lukke, så et
/// stoppet workspace er et no-op-`Ok`, ikke en fejl — rail'ens ✕ på en stoppet
/// post skal ikke vise en fejlbesked.
///
/// **`confirmed` er brugerens svar, ikke en genvej** — samme to-trins-form som
/// `set_workspace_hidden`: første kald med `false` lader backenden afvise med
/// `kraever_bekraeftelse:<n>`, så dialogen stilles på DENNE proces' tal og ikke
/// på rail'ens op til ét sekund gamle liste (se [`bekraeftelse_mangler`]).
///
/// Bekræftelsen stilles i den SYNLIGE proces, fordi et spørgsmål i et skjult
/// vindue er ubesvarligt. Modtagerens funnel springer derfor sin egen
/// bekræftelse over — se `close::CloseState::begin_peer_close`. Det gælder også
/// når modtageren er en selv: rail'ens ✕ på den aktive post går ud gennem
/// control-kanalen og tilbage til vores egen poller, så der er præcis ÉN
/// bekræftelse uanset hvilken post der lukkes.
#[tauri::command]
pub fn request_close_workspace(slug: String, confirmed: bool) -> Result<(), String> {
    let base = crate::project::global_base();
    kendt_slug(&base, &slug)?;
    if !crate::workspaces::instance_alive(&base, &slug) {
        return Ok(());
    }
    bekraeftelse_mangler(&base, &slug, confirmed)?;
    crate::workspaces::control::request_close(&base, &slug)
}

/// Frontendens svar på den globale exit-advarsel. `false` annullerer, `true`
/// broadcaster `quit_all` til alle levende workspace-processer og afslutter
/// derefter denne proces gennem den normative `ExitRequested`-teardown.
///
/// Uden et svar bliver funnelen stående i `Confirming`, og vinduet kan aldrig
/// lukkes igen — derfor svarer App.tsx også på Escape og på klik i baggrunden.
#[tauri::command]
pub fn confirm_close(app: tauri::AppHandle, ok: bool) -> Result<(), String> {
    let state =
        tauri::Manager::try_state::<std::sync::Arc<crate::workspaces::close::CloseState>>(&app)
            .ok_or("lukketilstanden er ikke initialiseret")?;
    let action = state.on_confirm(ok);
    if action == crate::workspaces::close::CloseAction::QuitAll {
        crate::workspaces::close::quit_all(app.clone())?;
    }
    Ok(())
}
