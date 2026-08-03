//! Projektlisten. Kilden er `projects/*/project.json` — der findes ingen delt
//! liste-fil, så flere processer og CLI'en kan registrere samtidigt uden race.
//!
//! `hidden` bor i en sidecar (`.hidden`), ikke i project.json: filen skal netop
//! kunne være korrupt og alligevel kunne skjules, og `write_project_meta`
//! canonicalizer `root`, hvilket fejler når mappen er forsvundet.

use crate::project::read_project_meta;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceEntry {
    pub slug: String,
    pub name: String,
    pub root: Option<PathBuf>,
    pub path_hint: Option<String>,
    pub hidden: bool,
    pub defect: bool,
    pub added_at: Option<String>,
}

pub fn list(global_base: &Path) -> Vec<WorkspaceEntry> {
    let dir = global_base.join("projects");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<WorkspaceEntry> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| {
            let slug = entry.file_name().to_string_lossy().into_owned();
            let hidden = entry.path().join(".hidden").exists();
            match read_project_meta(&entry.path()) {
                Ok(Some(meta)) => WorkspaceEntry {
                    name: meta.name,
                    root: Some(meta.root),
                    path_hint: None,
                    added_at: meta.added_at,
                    slug,
                    hidden,
                    defect: false,
                },
                // Ok(None) = ingen project.json endnu (mappen er under oprettelse);
                // Err = korrupt. Begge vises, korrupte markeres.
                Ok(None) | Err(_) => WorkspaceEntry {
                    name: slug.clone(),
                    root: None,
                    path_hint: None,
                    added_at: None,
                    slug,
                    hidden,
                    defect: true,
                },
            }
        })
        .collect();

    out.sort_by(|a, b| match (&a.added_at, &b.added_at) {
        (Some(x), Some(y)) => x.cmp(y).then(a.name.cmp(&b.name)).then(a.slug.cmp(&b.slug)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.name.cmp(&b.name).then(a.slug.cmp(&b.slug)),
    });

    let roots: Vec<String> = out
        .iter()
        .map(|entry| {
            entry
                .root
                .as_ref()
                .map(|root| root.display().to_string())
                .unwrap_or_default()
        })
        .collect();
    for (entry, suffix) in out.iter_mut().zip(unique_suffixes(&roots)) {
        if suffix != entry.name {
            entry.path_hint = Some(suffix);
        }
    }

    skjul_default_naar_rigtige_findes(global_base, &mut out);
    out
}

/// `projects/default` oprettes ALTID af `instance::resolve_startup_home()` ved
/// allerførste start og peger på brugerens hjemmemappe. Den ER
/// førstegangs-workspacet (ejer-beslutning 10) og skal derfor være synlig når
/// den står alene — men den må ikke blive liggende som en post der hedder
/// "default" øverst i en liste af rigtige projekter.
///
/// **Reglen er UDLEDT, ikke en `.hidden`-fil på disken.** Skrev vi `.hidden`
/// selv, kunne systemets skjulning ikke skelnes fra brugerens egen ("Fjern fra
/// listen"), og "Hent frem" ville blive undertrykt igen næste gang reglen kørte.
/// Udledningen gælder desuden uanset hvordan et projekt kom ind i listen — CLI,
/// `add_workspace`, eller en mappe kopieret ind i hånden — og den rører aldrig
/// `resolve_startup_home`s sidste udvej.
///
/// `.unhidden` er brugerens eksplicitte modsigelse af reglen. Markøren skrives
/// af HVER `unhide` (se dér), så den må **kun** konsulteres for `default`; læst
/// generelt ville den blive et "kan aldrig skjules igen"-flag for enhver post
/// brugeren én gang har hentet frem.
fn skjul_default_naar_rigtige_findes(global_base: &Path, out: &mut [WorkspaceEntry]) {
    // BEVIDST valg: skjulte og defekte poster tæller også som "rigtige". Brugeren
    // HAR andre projekter, også når han lige har skjult dem, og et defekt
    // project.json gør ikke posten uvirkelig. Konsekvensen er brugersynlig —
    // skjuler man sit eneste rigtige projekt, forsvinder default med det, og
    // rail'en viser "alle fjernet fra listen" frem for førstegangs-teksten — men
    // alternativet ville lade default poppe op igen hver gang listen tilfældigvis
    // stod tom, længe efter at førstegangs-oplevelsen var forbi.
    let har_rigtige = out.iter().any(|entry| entry.slug != "default");
    if !har_rigtige {
        return;
    }
    let eksplicit_frem = global_base
        .join("projects")
        .join("default")
        .join(".unhidden")
        .exists();
    for entry in out.iter_mut().filter(|entry| entry.slug == "default") {
        // `||` og ikke en ren tildeling: `hide()` sletter ikke `.unhidden`, så
        // begge sidecars kan ligge der samtidig. En eksplicit `.hidden` er
        // brugerens seneste ord og skal vinde over en gammel `.unhidden`.
        entry.hidden = entry.hidden || !eksplicit_frem;
    }
}

pub fn hide(global_base: &Path, slug: &str) -> Result<(), String> {
    let dir = global_base.join("projects").join(slug);
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    std::fs::write(dir.join(".hidden"), b"").map_err(|error| error.to_string())
}

pub fn unhide(global_base: &Path, slug: &str) -> Result<(), String> {
    let dir = global_base.join("projects").join(slug);
    match std::fs::remove_file(dir.join(".hidden")) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    // `.unhidden` registrerer at brugeren aktivt har hentet posten frem. Kun
    // default-workspacet læser markøren (Task 12): det skjules ellers automatisk
    // når rigtige projekter findes.
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    std::fs::write(dir.join(".unhidden"), b"").map_err(|error| error.to_string())
}

/// Korteste sti-suffiks der gør hver rod unik. Poster hvis basename allerede er
/// unikt får kun basename tilbage.
pub fn unique_suffixes(roots: &[String]) -> Vec<String> {
    let parts: Vec<Vec<&str>> = roots
        .iter()
        .map(|root| {
            root.split(['\\', '/'])
                .filter(|part| !part.is_empty())
                .collect()
        })
        .collect();
    // Sammenligningen sker paa KOMPONENTER, ikke paa sammensatte strenge.
    // Den tidligere form `join("\\")`ede baade kandidaten og hver anden posts
    // suffiks paa hvert dybde-trin — én ny `String` pr. (post, dybde, anden).
    // Badge-ticken (1 Hz) kalder `summaries()` ubetinget, saa den allokation
    // loeb hvert sekund for et resultat der kun aendrer sig naar et projekt
    // tilfoejes eller fjernes.
    parts
        .iter()
        .enumerate()
        .map(|(index, mine)| {
            for depth in 1..=mine.len() {
                let candidate = &mine[mine.len() - depth..];
                let unique = parts.iter().enumerate().all(|(other_index, other)| {
                    other_index == index
                        || other.len() < depth
                        || other[other.len() - depth..] != *candidate
                });
                if unique {
                    return candidate.join("\\");
                }
            }
            mine.join("\\")
        })
        .collect()
}
