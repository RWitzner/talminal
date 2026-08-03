//! Delt vinduesplacering, så alle workspaces bruger samme ramme.
//!
//! Vi gemmer normal restore-rect og maximized-flag, ikke den aktuelle
//! outer-rect. Ellers ville et maksimeret vindue miste sin normale størrelse.

use crate::atomic;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const DEBOUNCE_MS: u64 = 300;

/// **Positionen er OUTER, størrelsen er INNER — og det er ikke vilkårligt.**
///
/// Målene skal spejle præcis de to Tauri-kald `surface::apply_placement` bruger,
/// ellers er det vi gemmer ikke det vi sætter:
///  - `set_position` → `tao::set_outer_position` (tauri-runtime-wry) → OUTER
///  - `set_size` → `tao::set_inner_size` (tauri-runtime-wry:3527) → INNER
///
/// Rev 1 målte `outer_size()` på skrive-siden mod `set_size()`s inner på
/// læse-siden. Round-trippet var derfor ikke lukket: en gemt outer-bredde W blev
/// anvendt som INNER-bredde W, hvorefter vinduets outer blev W + 2 × kant. Blev
/// den nye outer gemt igen, voksede vinduet ved hver runde — og
/// `placement_already_applied` kunne pr. konstruktion aldrig matche i normal
/// tilstand, fordi den sammenlignede en outer-måling med et tal der var anvendt
/// som inner. Det var netop symptomet: fixet virkede maksimeret (hvor rect'en
/// ignoreres) og aldrig i normal tilstand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    /// OUTER x (`outer_position`).
    pub x: i32,
    /// OUTER y (`outer_position`).
    pub y: i32,
    /// INNER bredde (`inner_size`) — IKKE `outer_size`. Se typens doc.
    pub w: i32,
    /// INNER højde (`inner_size`) — IKKE `outer_size`. Se typens doc.
    pub h: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub normal: Rect,
    pub maximized: bool,
}

pub fn path(global_base: &Path) -> PathBuf {
    global_base.join("window_geometry.json")
}

/// REN funktion (kontrakt B, CORRECTIONS.md afsnit B). `on_window_event` leverer
/// `&Window`, ikke `&WebviewWindow` (tauri-2.11.5/src/app.rs:2060) — kalderen laeser
/// selv `is_maximized()`/`outer_position()`/`outer_size()` (findes paa begge typer)
/// og giver dem ind som primitiver, saa funktionen bliver testbar og kaldbar fra
/// begge sider.
///
/// Maksimeret vindue: bevarer den forrige normal-rect (mistes ellers, jf. modul-
/// docen ovenfor) fremfor at gemme den aktuelle (maksimerede) outer-rect.
///
/// `None` betyder "der er intet at gemme" — maksimeret UDEN en tidligere
/// placering. Kontrakt B skrev returtypen som `WindowPlacement`, men den form
/// kan ikke udtrykke netop dét tilfælde, og alternativet er en decideret fejl:
/// den aktuelle outer-rect ER fuldskærmen, så vi ville skrive
/// `{normal: fuldskærm, maximized: true}`. Ved næste reveal spiller
/// `apply_placement` den tilbage som `unmaximize → set_position → set_size →
/// maximize`, hvilket også sætter Windows' EGEN interne restore-rect til
/// fuldskærm — brugerens "gendan" krymper så ikke vinduet. Den gamle
/// `current_placement` returnerede `None` her af præcis samme grund. Ved at
/// springe skrivningen over lader vi den første ikke-maksimerede flyt/resize
/// etablere baselinen korrekt.
/// **`inner_size` er bevidst inner og ikke outer** — kalderen skal give
/// `window.inner_size()`. Se [`Rect`]s doc for hvorfor de to mål ikke må blandes.
pub fn placement_from(
    maximized: bool,
    outer_pos: (i32, i32),
    inner_size: (i32, i32),
    previous: Option<WindowPlacement>,
) -> Option<WindowPlacement> {
    let current = Rect {
        x: outer_pos.0,
        y: outer_pos.1,
        w: inner_size.0,
        h: inner_size.1,
    };
    if maximized {
        return Some(WindowPlacement {
            normal: previous?.normal,
            maximized: true,
        });
    }
    Some(WindowPlacement {
        normal: current,
        maximized: false,
    })
}

pub fn read(global_base: &Path) -> Option<WindowPlacement> {
    serde_json::from_str(&std::fs::read_to_string(path(global_base)).ok()?).ok()
}

/// Ejer geometri-skrivningerne for én proces.
pub struct GeometryWriter {
    global_base: PathBuf,
    suspended: Arc<AtomicBool>,
    generation: Arc<AtomicU64>,
    pending: Arc<Mutex<Option<WindowPlacement>>>,
}

impl GeometryWriter {
    pub fn new(global_base: PathBuf) -> Self {
        Self {
            global_base,
            suspended: Arc::new(AtomicBool::new(false)),
            generation: Arc::new(AtomicU64::new(0)),
            pending: Arc::new(Mutex::new(None)),
        }
    }

    /// Suspension invaliderer også en debounce, der allerede var armeret.
    pub fn suspend(&self, on: bool) {
        self.suspended.store(on, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
        if on {
            self.pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
        }
    }

    /// Coalescer flytte- og resize-events og skriver kun den seneste placering.
    pub fn schedule(&self, placement: WindowPlacement) {
        if self.suspended.load(Ordering::SeqCst) {
            return;
        }
        *self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(placement);

        let my_generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let base = self.global_base.clone();
        let suspended = Arc::clone(&self.suspended);
        let generation = Arc::clone(&self.generation);
        let pending = Arc::clone(&self.pending);

        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(DEBOUNCE_MS));
            if suspended.load(Ordering::SeqCst)
                || generation.load(Ordering::SeqCst) != my_generation
            {
                return;
            }
            let placement = pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            if let Some(placement) = placement {
                let _ = write_file(&base, &placement);
            }
        });
    }

    pub fn write_now(&self, placement: &WindowPlacement) -> Result<(), String> {
        if self.suspended.load(Ordering::SeqCst) {
            return Ok(());
        }
        write_file(&self.global_base, placement)
    }

    /// Skriver en evt. ARMERET (debounced) placering NU. Kaldes ved app-luk.
    ///
    /// Uden den taber vi den sidste vinduesplacering hver gang brugeren flytter
    /// eller resizer og lukker inden for `DEBOUNCE_MS`: `schedule`s baggrundstråd
    /// dør sammen med processen, før den når at skrive, og næste opstart falder
    /// tilbage til den forrige gemte placering. `ExitRequested`-handlerens
    /// `workspace::persist_now()` hjælper ikke — den flusher kort-layoutet i
    /// `workspace.json` og rører aldrig `window_geometry.json`.
    ///
    /// Tager `pending`, så et efterfølgende debounce-tick ikke skriver igen.
    pub fn flush(&self) -> Result<(), String> {
        if self.suspended.load(Ordering::SeqCst) {
            return Ok(());
        }
        let placement = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        match placement {
            Some(placement) => write_file(&self.global_base, &placement),
            None => Ok(()),
        }
    }
}

fn write_file(global_base: &Path, placement: &WindowPlacement) -> Result<(), String> {
    atomic::write_json_pretty(&path(global_base), placement).map_err(|error| error.to_string())
}

/// Står vinduet allerede sådan som en anvendelse ville sætte det?
///
/// `reveal` anvendte oprindeligt geometrien UBETINGET, og
/// `surface::apply_placement` er `unmaximize → set_position → set_size →
/// maximize`. På et maksimeret vindue er det fire Win32-geometriændringer, hver
/// med sit WM_SIZE — så WebView2 resizede fire gange, `CanvasSurface`s
/// ResizeObserver genberegnede hele tile-layoutet, og hvert korts egen
/// observer refittede sin terminal. Ved HVERT workspace-skift, også når
/// geometrien var fuldstændig uændret. Fladen krympede til normal-rect og voksede
/// til maksimeret igen, mens brugeren så på. Geometrien er delt og global, så
/// den er netop i det almindelige tilfælde allerede korrekt.
///
/// **Sammenligningen er på den SYNLIGE sluttilstand, ikke på restore-rect'en.**
/// To maksimerede vinduer ser ens ud uanset hvad de hver især ville gendanne
/// til, og et maksimeret vindues `outer_position`/`outer_size` ER fuldskærmen —
/// den bærer ingen information om normal-rect'en. `GetWindowPlacement` ville
/// kunne læse den, men dens `rcNormalPosition` er i workspace-koordinater og
/// kan derfor ikke sammenlignes direkte med de skærm-koordinater vi gemmer.
/// Konsekvensen er bevidst og kosmetisk: skifter man fra et maksimeret A til et
/// maksimeret B, beholder B sin egen restore-rect, så en efterfølgende "gendan"
/// giver B's gamle størrelse frem for A's. Intet af det er synligt før brugeren
/// gendanner, og prisen for at dele den perfekt er en resize-storm ved hvert
/// eneste skift.
///
/// **`None` betyder "kunne ikke læses" og svarer `false` — fail-safe.** Kan vi
/// ikke bevise at vinduet står rigtigt, anvender vi geometrien. En overflødig
/// resize er grim; et vindue der lander uden for skærmen er en fejl.
pub fn placement_already_applied(
    wanted: Rect,
    wanted_maximized: bool,
    actual: Option<(Rect, bool)>,
) -> bool {
    let Some((actual_rect, actual_maximized)) = actual else {
        return false;
    };
    if wanted_maximized != actual_maximized {
        return false;
    }
    wanted_maximized || wanted == actual_rect
}

/// Bevarer placeringen, hvis den overlapper en eksisterende monitor. En
/// frakoblet skærms placering trækkes ind og centreres på primærmonitoren.
pub fn clamp_to_work_area(rect: Rect, monitors: &[Rect]) -> Rect {
    let overlaps = monitors.iter().any(|monitor| {
        rect.x < monitor.x + monitor.w
            && rect.x + rect.w > monitor.x
            && rect.y < monitor.y + monitor.h
            && rect.y + rect.h > monitor.y
    });
    if overlaps {
        return rect;
    }

    let Some(primary) = monitors.first() else {
        return rect;
    };
    let w = rect.w.min(primary.w);
    let h = rect.h.min(primary.h);
    Rect {
        x: primary.x + (primary.w - w).max(0) / 2,
        y: primary.y + (primary.h - h).max(0) / 2,
        w,
        h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitorer() -> Vec<Rect> {
        vec![
            Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1040,
            },
            Rect {
                x: 1920,
                y: 0,
                w: 1920,
                h: 1040,
            },
        ]
    }

    #[test]
    fn placering_paa_frakoblet_skaerm_trækkes_ind_paa_primaer() {
        let vaek = Rect {
            x: 4000,
            y: 200,
            w: 1400,
            h: 900,
        };
        let clamped = clamp_to_work_area(vaek, &monitorer());
        assert!(
            clamped.x >= 0 && clamped.x + clamped.w <= 1920,
            "vinduet skal lande synligt på primær-monitoren, fik {clamped:?}"
        );
    }

    // --- placement_already_applied: reveal maa ikke resize et vindue der
    // allerede staar rigtigt. Se funktionens doc for hvad den ubetingede
    // anvendelse kostede ved hvert workspace-skift.

    #[test]
    fn maksimeret_vindue_der_allerede_er_maksimeret_roeres_ikke() {
        // Det ALMINDELIGE skift hos en bruger med maksimeret vindue. Rect'en er
        // med vilje forskellig: et maksimeret vindues outer-rect er fuldskaermen
        // og siger intet om den gemte normal-rect, saa den maa ikke indgaa.
        let gemt_normal = Rect {
            x: 100,
            y: 100,
            w: 1400,
            h: 900,
        };
        let fuldskaerm = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1040,
        };
        assert!(placement_already_applied(
            gemt_normal,
            true,
            Some((fuldskaerm, true))
        ));
    }

    #[test]
    fn uaendret_normal_placering_roeres_ikke() {
        let rect = Rect {
            x: 240,
            y: 120,
            w: 1400,
            h: 900,
        };
        assert!(placement_already_applied(rect, false, Some((rect, false))));
    }

    #[test]
    fn anden_normal_placering_anvendes() {
        let oensket = Rect {
            x: 240,
            y: 120,
            w: 1400,
            h: 900,
        };
        let staar_her = Rect {
            x: 10,
            y: 10,
            w: 1400,
            h: 900,
        };
        assert!(!placement_already_applied(
            oensket,
            false,
            Some((staar_her, false))
        ));
    }

    #[test]
    fn maksimeret_oensket_mod_gendannet_vindue_anvendes() {
        let rect = Rect {
            x: 100,
            y: 100,
            w: 1400,
            h: 900,
        };
        assert!(!placement_already_applied(rect, true, Some((rect, false))));
    }

    #[test]
    fn gendannet_oensket_mod_maksimeret_vindue_anvendes() {
        let rect = Rect {
            x: 100,
            y: 100,
            w: 1400,
            h: 900,
        };
        assert!(!placement_already_applied(rect, false, Some((rect, true))));
    }

    #[test]
    fn ulaeselig_vinduestilstand_anvender_geometrien() {
        // FAIL-SAFE. Kan vi ikke bevise at vinduet staar rigtigt, resizer vi
        // hellere overfloedigt end at vise et vindue det forkerte sted.
        let rect = Rect {
            x: 100,
            y: 100,
            w: 1400,
            h: 900,
        };
        assert!(!placement_already_applied(rect, false, None));
    }

    #[test]
    fn placering_paa_sekundaer_skaerm_bevares() {
        let paa_to = Rect {
            x: 2000,
            y: 100,
            w: 1400,
            h: 900,
        };
        assert_eq!(clamp_to_work_area(paa_to, &monitorer()), paa_to);
    }

    #[test]
    fn negativ_x_paa_en_rigtig_venstremonitor_bevares() {
        // Negative koordinater er IKKE i sig selv "uden for skærmen".
        let venstre = vec![
            Rect {
                x: -1920,
                y: 0,
                w: 1920,
                h: 1040,
            },
            Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1040,
            },
        ];
        let paa_venstre = Rect {
            x: -1800,
            y: 50,
            w: 1400,
            h: 900,
        };
        assert_eq!(clamp_to_work_area(paa_venstre, &venstre), paa_venstre);
    }

    #[test]
    fn maximized_gemmer_restore_rect_ikke_fuldskaerm() {
        let dir = tempfile::tempdir().unwrap();
        let w = GeometryWriter::new(dir.path().to_path_buf());
        let p = WindowPlacement {
            normal: Rect {
                x: 100,
                y: 100,
                w: 1400,
                h: 900,
            },
            maximized: true,
        };
        w.write_now(&p).unwrap();

        let laest = read(dir.path()).expect("placering læst");
        assert_eq!(
            laest.normal.w, 1400,
            "restore-rect må ikke overskrives af den maksimerede størrelse"
        );
        assert!(laest.maximized);
    }

    #[test]
    fn skrivning_er_suspenderet_under_conceal() {
        let dir = tempfile::tempdir().unwrap();
        // Suspensionen er INSTANS-state, ikke en global statisk: to tests der kørte
        // parallelt mod en global ville slå hinandens flag ned.
        let w = GeometryWriter::new(dir.path().to_path_buf());
        let foer = WindowPlacement {
            normal: Rect {
                x: 10,
                y: 10,
                w: 800,
                h: 600,
            },
            maximized: false,
        };
        w.write_now(&foer).unwrap();

        w.suspend(true);
        let parkering = WindowPlacement {
            normal: Rect {
                x: -32000,
                y: -32000,
                w: 800,
                h: 600,
            },
            maximized: false,
        };
        w.write_now(&parkering).unwrap();
        w.suspend(false);

        assert_eq!(
            read(dir.path()).unwrap(),
            foer,
            "plan B's parkeringsposition må aldrig gemmes"
        );
    }

    #[test]
    fn en_debounce_der_var_i_luften_lander_ikke_efter_suspension() {
        // Den farlige rækkefølge: brugeren flytter vinduet (debounce armeret),
        // skifter workspace inden de 300 ms er gået, og conceal suspenderer.
        // Den ventende skrivning må så IKKE lande bagefter.
        let dir = tempfile::tempdir().unwrap();
        let w = GeometryWriter::new(dir.path().to_path_buf());
        let foer = WindowPlacement {
            normal: Rect {
                x: 10,
                y: 10,
                w: 800,
                h: 600,
            },
            maximized: false,
        };
        w.write_now(&foer).unwrap();

        w.schedule(WindowPlacement {
            normal: Rect {
                x: 99,
                y: 99,
                w: 800,
                h: 600,
            },
            maximized: false,
        });
        w.suspend(true);
        std::thread::sleep(std::time::Duration::from_millis(DEBOUNCE_MS + 150));

        assert_eq!(
            read(dir.path()).unwrap(),
            foer,
            "en armeret debounce skal droppes ved suspension"
        );
    }

    // Kendelse B (CORRECTIONS.md afsnit B): planens `current_placement(&WebviewWindow)`
    // er erstattet af den rene `placement_from(maximized, outer_pos, outer_size, previous)`.
    // Testes separat: et maksimeret vindue skal bevare sin forrige normal-rect frem
    // for at gemme fuldskærms-koordinaterne som normal-størrelse.

    #[test]
    fn placement_from_ikke_maksimeret_bruger_outer_rect() {
        let resultat = placement_from(false, (10, 20), (800, 600), None);
        assert_eq!(
            resultat,
            Some(WindowPlacement {
                normal: Rect {
                    x: 10,
                    y: 20,
                    w: 800,
                    h: 600
                },
                maximized: false,
            })
        );
    }

    #[test]
    fn maksimeret_uden_forrige_placering_gemmer_ingenting() {
        // Frisk installation, ingen window_geometry.json, og brugerens
        // ALLERFOERSTE handling er at maksimere (dobbeltklik paa titellinjen).
        // Gemte vi den aktuelle rect som normal-rect, ville "gendan" ikke
        // krympe vinduet bagefter — apply_placement saetter ogsaa Windows' egen
        // restore-rect til fuldskaerm ved naeste reveal.
        assert_eq!(
            placement_from(true, (0, 0), (1920, 1040), None),
            None,
            "fuldskaerms-rect'en maa ALDRIG lande som normal-rect"
        );
    }

    #[test]
    fn flush_skriver_en_debounce_der_endnu_ikke_var_landet() {
        // App-luk inden for DEBOUNCE_MS: schedule()s traad naar aldrig at
        // skrive, fordi processen doer foerst. Uden flush() taber brugeren sin
        // sidste vinduesplacering uden varsel.
        let dir = tempfile::tempdir().unwrap();
        let w = GeometryWriter::new(dir.path().to_path_buf());
        let sidste = WindowPlacement {
            normal: Rect {
                x: 42,
                y: 43,
                w: 1000,
                h: 700,
            },
            maximized: false,
        };
        w.schedule(sidste);
        assert_eq!(
            read(dir.path()),
            None,
            "debouncen maa ikke have skrevet endnu"
        );

        w.flush().unwrap();
        assert_eq!(
            read(dir.path()),
            Some(sidste),
            "flush skal redde den armerede skrivning"
        );
    }

    #[test]
    fn flush_uden_noget_armeret_er_et_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let w = GeometryWriter::new(dir.path().to_path_buf());
        w.flush().unwrap();
        assert_eq!(
            read(dir.path()),
            None,
            "flush maa ikke opfinde en placering"
        );
    }

    #[test]
    fn placement_from_maksimeret_bevarer_forrige_normal_rect() {
        let forrige = WindowPlacement {
            normal: Rect {
                x: 100,
                y: 100,
                w: 1400,
                h: 900,
            },
            maximized: false,
        };
        // outer_pos/outer_size er her fuldskærms-koordinaterne — de må IKKE
        // ende i resultatets normal-rect.
        let resultat = placement_from(true, (0, 0), (1920, 1040), Some(forrige));
        assert_eq!(
            resultat,
            Some(WindowPlacement {
                normal: Rect {
                    x: 100,
                    y: 100,
                    w: 1400,
                    h: 900
                },
                maximized: true,
            }),
            "maksimeret vindue skal bevare forrige normal-rect, ikke gemme fuldskærms-koordinater"
        );
    }
}
