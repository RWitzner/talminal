//! `talminal` CLI launcher (B-light Task 3).
//!
//! Flow: find project root → state dir → ensure project.json → focus or spawn.
//! Spawn is injected via a seam so unit tests can record without launching the app.

use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use talminal_canvas_lib::instance::focus_existing_instance;
use talminal_canvas_lib::project::{
    canonical_key, find_project_root, global_base, project_slug, project_state_dir,
    read_project_meta, write_project_meta, ProjectMeta,
};
use talminal_canvas_lib::workspaces::{
    deadline_from_now, listing, next_request, spawn_workspace, write_active,
};

#[derive(Debug)]
enum LaunchOutcome {
    Focused,
    FocusFailedButRunning,
    Spawned(PathBuf),
}

fn app_exe_path() -> PathBuf {
    if let Ok(p) = std::env::var("TALMINAL_APP_EXE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let current = std::env::current_exe().expect("current_exe");
    current.with_file_name("talminal-canvas.exe")
}

/// Udfaldet af `.lock`-proben — samme tre-deling som
/// `instance::AcquireOutcome` (fund M11), fordi launcheren laver PRAECIS den
/// samme klassifikation og foer begik praecis den samme fejl.
#[derive(Debug)]
enum RunningProbe {
    Running,
    NotRunning,
    /// Vi ved det ikke: `.lock` kunne ikke aabnes af en grund der intet har med
    /// en koerende instans at goere.
    Failed(std::io::Error),
}

/// Prober om en anden proces holder `state_dir\.lock` eksklusivt.
///
/// M11: `Err(_) => true` loej. Kun ERROR_SHARING_VIOLATION (32) og
/// ERROR_LOCK_VIOLATION (33) betyder "en anden proces har filen"; ACCESS_DENIED,
/// fuld disk, read-only mappe og en for lang sti gjorde tidligere at launcheren
/// meldte "vinduet koerer" og afsluttede med 0 — uden vindue og uden spor.
///
/// NB (uaendret, selvforskyldt race): proben aabner selv `.lock` med
/// `share_mode(0)`, saa en app der starter i praecis det oejeblik ser en
/// sharing violation og tror den er dublet. Vinduet er faa mikrosekunder og
/// klassifikationen er den samme som foer — aendringen her goer det hverken
/// vaerre eller bedre. En rigtig lukning kraever en ikke-forstyrrende probe
/// (fx `OpenMutexW` mod `Talminal-<slug>`), hvilket er en anden opgave.
fn instance_appears_running(state_dir: &Path) -> RunningProbe {
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    let lock_path = state_dir.join(".lock");
    let result = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .share_mode(0)
        .open(&lock_path);
    match result {
        Ok(_held_briefly) => RunningProbe::NotRunning,
        Err(e) => match e.raw_os_error() {
            Some(ERROR_SHARING_VIOLATION) | Some(ERROR_LOCK_VIOLATION) => RunningProbe::Running,
            _ => RunningProbe::Failed(e),
        },
    }
}

fn ensure_project_meta(root: &Path, state_dir: &Path) -> Result<(), String> {
    match read_project_meta(state_dir)? {
        None => {
            let name = root
                .file_name()
                .ok_or_else(|| format!("project root has no basename: {}", root.display()))?
                .to_string_lossy()
                .into_owned();
            write_project_meta(
                state_dir,
                &ProjectMeta {
                    root: root.to_path_buf(),
                    name,
                    added_at: Some(talminal_canvas_lib::workspaces::now_iso_z()),
                },
            )
            .map_err(|e| format!("write project.json failed: {e}"))
        }
        Some(meta) => {
            let existing = canonical_key(&meta.root)?;
            let incoming = canonical_key(root)?;
            if existing != incoming {
                return Err(format!(
                    "project.json root mismatch: stored {} vs {}",
                    meta.root.display(),
                    root.display()
                ));
            }
            Ok(())
        }
    }
}

/// Meld projektet ind i workspace-protokollen (plan Task 12).
///
/// **Skal køre FØR `focus_existing_instance` og gælde ALLE tre udfald** —
/// `Focused`, `FocusFailedButRunning` og `Spawned`. Det er netop i
/// `Focused`-tilfældet at det hidtil aktive vindues poll-løkke ellers ville
/// skjule præcis det vindue vi lige hentede frem: CLI og protokol ville
/// modarbejde hinanden, og brugeren ville se sit vindue blinke og forsvinde.
///
/// `unhide` hører med: et projekt brugeren tidligere fjernede fra listen skal
/// komme frem igen når han udtrykkeligt starter det fra sin terminal.
fn meld_ind_i_protokollen(root: &Path) -> Result<(), String> {
    let base = global_base();
    let slug = project_slug(root)?;
    listing::unhide(&base, &slug)?;
    // `talminal` er en ANDEN binary end appen og kalder aldrig
    // `set_my_instance_id`; `my_instance_id()` ville falde tilbage paa
    // "<pid>-uinitialiseret". Launcheren stempler derfor sig selv.
    let issuer = format!("cli-{}", std::process::id());
    let request = next_request(&base, &slug, &issuer, deadline_from_now());
    write_active(&base, &request)
}

fn run(
    cwd: &Path,
    arg: Option<&Path>,
    spawn: &dyn Fn(&Path /*exe*/, &Path /*home*/) -> Result<(), String>,
) -> Result<LaunchOutcome, String> {
    let start = arg.unwrap_or(cwd);
    let root = find_project_root(start)?;
    let state_dir = project_state_dir(&root)?;
    ensure_project_meta(&root, &state_dir)?;
    meld_ind_i_protokollen(&root)?;

    if focus_existing_instance(&state_dir) {
        return Ok(LaunchOutcome::Focused);
    }

    match instance_appears_running(&state_dir) {
        RunningProbe::Running => return Ok(LaunchOutcome::FocusFailedButRunning),
        // M11: en ubestemmelig probe maa hverken spawne en mulig dublet eller
        // paastaa at vinduet koerer. Fejlen baeres ud til `main`, som printer
        // den og afslutter med exit-kode 1.
        RunningProbe::Failed(e) => {
            return Err(format!(
                "kunne ikke afgoere om en instans koerer ({}): {e}",
                state_dir.join(".lock").display()
            ))
        }
        RunningProbe::NotRunning => {}
    }

    let exe = app_exe_path();
    spawn(&exe, &state_dir)?;
    Ok(LaunchOutcome::Spawned(exe))
}

/// Én spawn-vej for CLI og app (Task 8): DETACHED_PROCESS + null-stdio bor nu i
/// lib'en, så app-siden ikke kan drive fra launcherens adfærd. Seamen og dens
/// tests er uændrede.
fn real_spawn(exe: &Path, home: &Path) -> Result<(), String> {
    spawn_workspace(exe, home)
}

fn main() {
    let cwd = std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("talminal: cannot read cwd: {e}");
        std::process::exit(1);
    });
    let arg = std::env::args().nth(1).map(PathBuf::from);
    match run(&cwd, arg.as_deref(), &real_spawn) {
        Ok(LaunchOutcome::Focused) => {
            println!("talminal: focused existing instance");
        }
        Ok(LaunchOutcome::FocusFailedButRunning) => {
            println!("talminal: vinduet kører; kunne ikke bringes i forgrunden");
        }
        Ok(LaunchOutcome::Spawned(exe)) => {
            println!("talminal: spawned {}", exe.display());
        }
        Err(e) => {
            eprintln!("talminal: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::{Mutex, MutexGuard};
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_guard() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn make_git_project() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::create_dir(root.join(".git")).unwrap();
        (dir, root)
    }

    #[test]
    fn run_skriver_project_json_ved_foerste_kald() {
        let _g = env_guard();
        let global = tempdir().unwrap();
        std::env::set_var("TALMINAL_GLOBAL_HOME", global.path());
        let (_proj, root) = make_git_project();

        let calls = Mutex::new(Vec::<(PathBuf, PathBuf)>::new());
        let outcome = run(&root, None, &|exe, home| {
            calls
                .lock()
                .unwrap()
                .push((exe.to_path_buf(), home.to_path_buf()));
            Ok(())
        })
        .expect("run ok");

        let expected_home = project_state_dir(&root).unwrap();
        match outcome {
            LaunchOutcome::Spawned(_) => {}
            other => panic!("expected Spawned, got {:?}", outcome_tag(&other)),
        }
        let recorded = calls.lock().unwrap().clone();
        assert_eq!(recorded.len(), 1, "spawn must be called once");
        assert_eq!(recorded[0].1, expected_home);

        let meta = read_project_meta(&expected_home)
            .unwrap()
            .expect("project.json written");
        assert_eq!(
            canonical_key(&meta.root).unwrap(),
            canonical_key(&root).unwrap()
        );
        assert_eq!(meta.name, root.file_name().unwrap().to_string_lossy());

        std::env::remove_var("TALMINAL_GLOBAL_HOME");
    }

    #[test]
    fn run_fejler_ved_root_mismatch() {
        let _g = env_guard();
        let global = tempdir().unwrap();
        std::env::set_var("TALMINAL_GLOBAL_HOME", global.path());
        let (_proj, root) = make_git_project();
        let state_dir = project_state_dir(&root).unwrap();
        let other = tempdir().unwrap();
        fs::create_dir(other.path().join(".git")).unwrap();
        let other_root = find_project_root(other.path()).unwrap();
        write_project_meta(
            &state_dir,
            &ProjectMeta {
                root: other_root,
                name: "other".into(),
                added_at: None,
            },
        )
        .unwrap();

        let err = run(&root, None, &|_, _| Ok(())).expect_err("mismatch must err");
        assert!(
            err.to_ascii_lowercase().contains("mismatch") || err.contains("root"),
            "unexpected err: {err}"
        );
        // Must not overwrite
        let meta = read_project_meta(&state_dir).unwrap().unwrap();
        assert_eq!(meta.name, "other");

        std::env::remove_var("TALMINAL_GLOBAL_HOME");
    }

    #[test]
    fn run_fejler_ved_korrupt_meta() {
        let _g = env_guard();
        let global = tempdir().unwrap();
        std::env::set_var("TALMINAL_GLOBAL_HOME", global.path());
        let (_proj, root) = make_git_project();
        let state_dir = project_state_dir(&root).unwrap();
        fs::create_dir_all(&state_dir).unwrap();
        fs::write(state_dir.join("project.json"), b"{not json!!!").unwrap();

        let err = run(&root, None, &|_, _| panic!("spawn must not run")).expect_err("corrupt");
        assert!(
            err.contains("parse") || err.contains("project.json"),
            "unexpected err: {err}"
        );

        std::env::remove_var("TALMINAL_GLOBAL_HOME");
    }

    #[test]
    #[allow(non_snake_case)] // plan-named: IKKE must stay visible in the test id
    fn run_fokusfejl_mens_lock_holdes_spawner_IKKE() {
        let _g = env_guard();
        let global = tempdir().unwrap();
        std::env::set_var("TALMINAL_GLOBAL_HOME", global.path());
        let (_proj, root) = make_git_project();
        let state_dir = project_state_dir(&root).unwrap();
        write_project_meta(
            &state_dir,
            &ProjectMeta {
                root: root.clone(),
                name: root.file_name().unwrap().to_string_lossy().into_owned(),
                added_at: None,
            },
        )
        .unwrap();

        // Hold .lock exclusively for the duration of the test (simulates running instance).
        let _lock = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .share_mode(0)
            .open(state_dir.join(".lock"))
            .expect("test holds lock");

        let spawn_count = Mutex::new(0u32);
        let outcome = run(&root, None, &|_, _| {
            *spawn_count.lock().unwrap() += 1;
            Ok(())
        })
        .expect("run ok");

        assert!(
            matches!(outcome, LaunchOutcome::FocusFailedButRunning),
            "expected FocusFailedButRunning, got {:?}",
            outcome_tag(&outcome)
        );
        assert_eq!(
            *spawn_count.lock().unwrap(),
            0,
            "must NOT spawn while lock held"
        );

        std::env::remove_var("TALMINAL_GLOBAL_HOME");
    }

    /// M11-regression: proben maa ikke laese "jeg kunne ikke aabne filen" som
    /// "en anden instans koerer". Foer klassifikationen svarede `run` med
    /// `FocusFailedButRunning` og exit 0 — operatoeren fik at vide at vinduet
    /// koerte, uden at der var noget vindue.
    #[test]
    fn run_melder_io_fejl_fra_lock_proben_frem_for_at_paastaa_koerer() {
        let _g = env_guard();
        let global = tempdir().unwrap();
        std::env::set_var("TALMINAL_GLOBAL_HOME", global.path());
        let (_proj, root) = make_git_project();
        let state_dir = project_state_dir(&root).unwrap();
        write_project_meta(
            &state_dir,
            &ProjectMeta {
                root: root.clone(),
                name: root.file_name().unwrap().to_string_lossy().into_owned(),
                added_at: None,
            },
        )
        .unwrap();
        // `.lock` som MAPPE: CreateFileW afviser den uden
        // FILE_FLAG_BACKUP_SEMANTICS. Fejlkoden er hverken 32 eller 33, saa den
        // maa aldrig taelle som dublet.
        fs::create_dir_all(state_dir.join(".lock")).unwrap();

        let spawn_count = Mutex::new(0u32);
        let err = run(&root, None, &|_, _| {
            *spawn_count.lock().unwrap() += 1;
            Ok(())
        })
        .expect_err("en ubestemmelig probe skal fejle synligt");

        std::env::remove_var("TALMINAL_GLOBAL_HOME");
        assert!(
            err.contains(".lock"),
            "fejlteksten skal pege paa laasefilen: {err}"
        );
        assert_eq!(
            *spawn_count.lock().unwrap(),
            0,
            "en ubestemmelig probe maa heller aldrig spawne en mulig dublet"
        );
    }

    #[test]
    fn instance_appears_running_skelner_dublet_fra_io_fejl() {
        let tmp = tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        fs::create_dir_all(&state_dir).unwrap();

        // (a) fri mappe => NotRunning.
        assert!(
            matches!(
                instance_appears_running(&state_dir),
                RunningProbe::NotRunning
            ),
            "fri .lock skal give NotRunning"
        );

        // (b) en anden aabner holder .lock eksklusivt => Running (kode 32).
        let held = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .share_mode(0)
            .open(state_dir.join(".lock"))
            .expect("testen holder laasen");
        assert!(
            matches!(instance_appears_running(&state_dir), RunningProbe::Running),
            "eksklusivt holdt .lock skal give Running"
        );
        drop(held);

        // (c) uaabnelig .lock af en ANDEN grund => Failed, ikke Running.
        let other = tempdir().unwrap();
        let broken = other.path().join("state");
        fs::create_dir_all(broken.join(".lock")).unwrap();
        assert!(
            matches!(instance_appears_running(&broken), RunningProbe::Failed(_)),
            "en I/O-fejl maa ikke maskere sig som en koerende instans"
        );
    }

    #[test]
    fn app_exe_path_env_override_vinder() {
        let _g = env_guard();
        std::env::set_var("TALMINAL_APP_EXE", r"C:\fake\custom-app.exe");
        let p = app_exe_path();
        assert_eq!(p, PathBuf::from(r"C:\fake\custom-app.exe"));
        std::env::remove_var("TALMINAL_APP_EXE");
    }

    #[test]
    fn app_exe_path_default_er_talminal_canvas_exe() {
        let _g = env_guard();
        std::env::remove_var("TALMINAL_APP_EXE");
        let p = app_exe_path();
        assert_eq!(
            p.file_name().and_then(|n| n.to_str()),
            Some("talminal-canvas.exe"),
            "default sibling must be Cargo target name, got {}",
            p.display()
        );
    }

    fn outcome_tag(o: &LaunchOutcome) -> &'static str {
        match o {
            LaunchOutcome::Focused => "Focused",
            LaunchOutcome::FocusFailedButRunning => "FocusFailedButRunning",
            LaunchOutcome::Spawned(_) => "Spawned",
        }
    }
}
