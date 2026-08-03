//! En delt atomisk fil-skriver (tmp + rename).
//!
//! HVORFOR ét sted: project.rs, workspace.rs og instance.rs havde hver sin modul-lokale
//! tæller og dannede samme navn `.tmp-<pid>-<counter>`. To førstegangsskrivninger i samme
//! mappe valgte begge `.tmp-<pid>-0`, og den ene tråds bytes kunne havne i den andens
//! målfil. Temp-navnet er derfor afledt af MÅLFILENS navn plus en PROCES-GLOBAL nonce,
//! og filen åbnes med create_new(true), så en kollision fejler i stedet for at overskrive.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NONCE: AtomicU64 = AtomicU64::new(0);

fn temp_path(path: &Path, nonce: u64) -> PathBuf {
    let stem = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    dir.join(format!(".tmp-{stem}-{}-{nonce}", std::process::id()))
}

fn next_nonce() -> u64 {
    NONCE.fetch_add(1, Ordering::Relaxed)
}

/// Skriver `contents` til `path` atomisk. Overskriver eksisterende fil.
pub fn write(path: &Path, contents: &[u8]) -> io::Result<()> {
    write_with(path, contents, next_nonce)
}

/// Serialiserer `value` som pretty JSON med AFSLUTTENDE NEWLINE og skriver det
/// atomisk.
///
/// Newlinen er ikke kosmetik: `tests/workspace.rs`' byte-stabile roundtrip
/// asserter den. Ni persistere gentog derfor de samme tre linjer
/// (`to_string_pretty` → `push('\n')` → `write`), og konventionen levede kun i
/// hukommelsen hos den der skrev den tiende. Nu er den en egenskab ved
/// funktionen.
pub fn write_json_pretty<T: serde::Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let mut body = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    body.push('\n');
    write(path, body.as_bytes())
}

/// Kernen bag [`write`]. `nonce` er en seam: produktionen leverer den proces-globale
/// tæller, testen kan levere en deterministisk sekvens og dermed øve retry-løkken.
/// Seamen er privat — modulets offentlige flade er stadig kun `write`.
fn write_with(path: &Path, contents: &[u8], mut nonce: impl FnMut() -> u64) -> io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    // create_new fejler hvis navnet findes; en anden skriver kan have efterladt
    // en rest efter et crash. Vi prøver et nyt nonce et bundet antal gange.
    for _ in 0..64 {
        let tmp = temp_path(path, nonce());
        match fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)
        {
            Ok(mut f) => {
                let result = f.write_all(contents).and_then(|()| f.sync_all());
                drop(f);
                if let Err(e) = result {
                    let _ = fs::remove_file(&tmp);
                    return Err(e);
                }
                if let Err(e) = fs::rename(&tmp, path) {
                    let _ = fs::remove_file(&tmp);
                    return Err(e);
                }
                return Ok(());
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "atomic: kunne ikke finde et frit temp-navn",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    /// Regressionsværn mod den oprindelige bug (tre modul-lokale tællere), MEN den er
    /// bevisligt IKKE et værn om nonce'en: to forskellige målfiler får altid forskellige
    /// temp-navne alene af `file_name()`. Målt mod en nonce-løs `temp_path()` blev denne
    /// test grøn hver eneste gang. Beviset for nonce'en er testen nedenunder.
    #[test]
    fn samtidige_skrivninger_til_forskellige_filer_i_samme_mappe_blandes_aldrig() {
        let dir = tempfile::tempdir().unwrap();
        // 200 runder: kollisionen kræver at to tråde vælger temp-navn i samme øjeblik.
        // Én runde ville kunne være grøn ved et tilfælde.
        for round in 0..200 {
            let barrier = Arc::new(Barrier::new(2));
            let a_path = dir.path().join(format!("alpha-{round}"));
            let b_path = dir.path().join(format!("beta-{round}"));
            let a_body = vec![b'A'; 4096];
            let b_body = vec![b'B'; 4096];

            let (ap, bp) = (a_path.clone(), b_path.clone());
            let (ab, bb) = (a_body.clone(), b_body.clone());
            let barrier_b = Arc::clone(&barrier);
            let t = std::thread::spawn(move || {
                barrier_b.wait();
                write(&bp, &bb).unwrap();
            });
            barrier.wait();
            write(&ap, &ab).unwrap();
            t.join().unwrap();

            assert_eq!(
                std::fs::read(&a_path).unwrap(),
                a_body,
                "runde {round}: alpha fik fremmede bytes"
            );
            assert_eq!(
                std::fs::read(&b_path).unwrap(),
                b_body,
                "runde {round}: beta fik fremmede bytes"
            );
        }
    }

    /// DETTE er beviset for at nonce'en virker.
    ///
    /// Den forrige test skriver til to FORSKELLIGE målfiler. Fordi `temp_path()` afleder
    /// temp-navnet af målfilens `file_name()`, får de to skrivninger altid forskellige
    /// temp-navne — uanset trådplanlægning, og uanset om NONCE overhovedet findes. Den test
    /// ville være lige så grøn mod en regression der fjernede nonce'en helt.
    ///
    /// Her rammer alle tråde SAMME målfil. Stem'et er dermed identisk, og nonce'en er det
    /// ENESTE der adskiller temp-navnene. Fjernes den fra `temp_path()`, vælger alle tråde
    /// samme navn: én vinder `create_new(true)`, resten løber deres 64 forsøg tør på det
    /// samme optagne navn og returnerer "kunne ikke finde et frit temp-navn" — testen bliver
    /// rød på `write fejlede`.
    ///
    /// FALSIFICERBARHEDEN ER MÅLT, ikke påstået: med nonce'en fjernet fra `temp_path()`
    /// fejlede testen 8 ud af 8 kørsler; med nonce'en intakt er den grøn 8 ud af 8.
    ///
    /// Testen fastholder samtidig invarianten: målfilen ender med PRÆCIS én tråds payload,
    /// aldrig en blanding af to, og der er ingen `.tmp-`-rester tilbage.
    #[test]
    fn samtidige_skrivninger_til_samme_maalfil_giver_praecis_een_traads_payload() {
        // En Barrier + ÉN skrivning pr. tråd er ikke nok: målt mod en nonce-løs
        // temp_path() blev det mønster grønt 3 ud af 5 gange, fordi vinderens
        // create→write→sync→rename kan nå at være færdig før taberne vågner.
        // Derfor hamrer hver tråd i stedet en løkke — overlappet er så uundgåeligt.
        const TRAADE: usize = 8;
        const SKRIVNINGER: usize = 25;
        let dir = tempfile::tempdir().unwrap();
        for round in 0..4 {
            let path = dir.path().join(format!("samme-{round}.json"));
            let barrier = Arc::new(Barrier::new(TRAADE));
            let payloads: Vec<Vec<u8>> = (0..TRAADE).map(|t| vec![b'A' + t as u8; 4096]).collect();

            let handles: Vec<_> = payloads
                .iter()
                .cloned()
                .map(|body| {
                    let p = path.clone();
                    let b = Arc::clone(&barrier);
                    std::thread::spawn(move || {
                        b.wait();
                        for _ in 0..SKRIVNINGER {
                            write(&p, &body)?;
                        }
                        Ok::<(), io::Error>(())
                    })
                })
                .collect();

            for (t, h) in handles.into_iter().enumerate() {
                h.join()
                    .unwrap()
                    .unwrap_or_else(|e| panic!("runde {round}, traad {t}: write fejlede: {e}"));
            }

            let faktisk = std::fs::read(&path).unwrap();
            let vinder = payloads.iter().position(|p| *p == faktisk);
            assert!(
                vinder.is_some(),
                "runde {round}: maalfilen er en BLANDING — laengde {}, foerste byte {:?}, sidste byte {:?}",
                faktisk.len(),
                faktisk.first(),
                faktisk.last()
            );
        }

        let rester: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-"))
            .map(|e| e.file_name())
            .collect();
        assert!(rester.is_empty(), "temp-filer efterladt: {rester:?}");
    }

    /// Fund 2: retry-løkken ved `AlreadyExists`. Tre crash-efterladte temp-filer optager de
    /// tre første navne løkken vil vælge; writeren skal finde det fjerde og lykkes — uden at
    /// røre resterne (de tilhører en anden, muligvis stadig levende, skriver).
    ///
    /// Nonce-sekvensen injiceres gennem den private `write_with`-seam, fordi NONCE er
    /// proces-global: andre tests i samme binary bumper den samtidig, så et forsøg på at
    /// "gætte" de næste navne udefra ville være flaky.
    #[test]
    fn retry_loekken_springer_crash_rester_over_og_finder_et_frit_navn() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.json");
        for n in 0..3u64 {
            std::fs::write(temp_path(&path, n), b"rest efter crash").unwrap();
        }

        let mut seq = 0u64;
        write_with(&path, b"nyt", || {
            let n = seq;
            seq += 1;
            n
        })
        .unwrap();

        assert_eq!(
            seq, 4,
            "loekken skulle have proevet fire navne (0,1,2 optaget + 3 frit)"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"nyt");
        for n in 0..3u64 {
            assert_eq!(
                std::fs::read(temp_path(&path, n)).unwrap(),
                b"rest efter crash",
                "rest {n} blev roert"
            );
        }
        assert!(
            !temp_path(&path, 3).exists(),
            "det brugte temp-navn blev ikke renamet vaek"
        );
    }

    /// Fund 2, fejlgrenen: alle 64 navne optaget → writeren giver op med `AlreadyExists`
    /// og lader målfilen stå urørt frem for at overskrive noget.
    #[test]
    fn giver_op_med_alreadyexists_naar_alle_64_navne_er_optaget() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.json");
        for n in 0..64u64 {
            std::fs::write(temp_path(&path, n), b"rest").unwrap();
        }

        let mut seq = 0u64;
        let err = write_with(&path, b"nyt", || {
            let n = seq;
            seq += 1;
            n
        })
        .unwrap_err();

        assert_eq!(seq, 64, "loekken skal proeve praecis 64 gange");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(
            err.to_string()
                .contains("kunne ikke finde et frit temp-navn"),
            "uventet fejltekst: {err}"
        );
        assert!(
            !path.exists(),
            "maalfilen maa ikke opstaa naar writeren gav op"
        );
    }

    #[test]
    fn erstatter_eksisterende_fil() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.json");
        write(&path, b"foerste").unwrap();
        write(&path, b"anden").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"anden");
    }

    #[test]
    fn efterlader_ingen_temp_filer() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("f.json"), b"x").unwrap();
        let rester: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-"))
            .collect();
        assert!(rester.is_empty(), "temp-filer efterladt: {rester:?}");
    }
}
