//! B-light Task 2 — instance lock, focus, bare-start resolution.
//!
//! Concurrency cases use the READY-/RELEASE-file worker protocol (M12),
//! matching tests/workspace.rs process isolation.

use std::fs::{self, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use talminal_canvas_lib::instance::{
    acquire_instance_lock, focus_existing_instance, read_instance_info, resolve_startup_home,
    write_instance_info, AcquireOutcome,
};
use talminal_canvas_lib::project::{
    self, normalize_path, project_state_dir, write_last_project, write_project_meta, ProjectMeta,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetDesktopWindow;

mod common;
use common::is_worker;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn wait_for_file(path: &Path, timeout: Duration) {
    let start = Instant::now();
    while !path.exists() {
        if start.elapsed() > timeout {
            panic!("timeout waiting for {}", path.display());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// M11: udfaldet har tre grene, og assert-teksterne skal kunne navngive dem —
/// ellers rapporterer en fejlet test bare "not None" som foer.
fn outcome_tag(outcome: &AcquireOutcome) -> &'static str {
    match outcome {
        AcquireOutcome::Acquired(_) => "Acquired",
        AcquireOutcome::AlreadyRunning => "AlreadyRunning",
        AcquireOutcome::Failed(_) => "Failed",
    }
}

fn spawn_worker(worker: &str, envs: &[(&str, &str)]) -> std::process::Output {
    let exe = std::env::current_exe().expect("current_exe");
    let mut cmd = Command::new(exe);
    cmd.arg("--exact")
        .arg(worker)
        .arg("--test-threads=1")
        .arg("--nocapture")
        .env("TALMINAL_WORKER", worker);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn worker")
}

// ---------------------------------------------------------------------------
// Workers (no-op unless TALMINAL_WORKER matches)
// ---------------------------------------------------------------------------

#[test]
fn hold_lock_until_release() {
    if !is_worker("hold_lock_until_release") {
        return;
    }
    let state = PathBuf::from(std::env::var("TALMINAL_STATE_DIR").unwrap());
    let slug = std::env::var("TALMINAL_SLUG").unwrap();
    let ready = PathBuf::from(std::env::var("TALMINAL_READY_FILE").unwrap());
    let release = PathBuf::from(std::env::var("TALMINAL_RELEASE_FILE").unwrap());

    let lock = match acquire_instance_lock(&state, &slug) {
        AcquireOutcome::Acquired(lock) => lock,
        other => panic!("worker A must acquire, got {}", outcome_tag(&other)),
    };
    fs::write(&ready, b"ready").unwrap();
    wait_for_file(&release, Duration::from_secs(30));
    drop(lock);
}

#[test]
fn try_acquire_expect_none() {
    if !is_worker("try_acquire_expect_none") {
        return;
    }
    let state = PathBuf::from(std::env::var("TALMINAL_STATE_DIR").unwrap());
    let slug = std::env::var("TALMINAL_SLUG").unwrap();
    // Exit-koderne er nu tre, ikke to (M11): 18 fanger den grøn-til-rød-fælde
    // hvor en I/O-fejl ville have set ud som en aegte dublet.
    match acquire_instance_lock(&state, &slug) {
        AcquireOutcome::AlreadyRunning => std::process::exit(17),
        AcquireOutcome::Acquired(_lock) => std::process::exit(1),
        AcquireOutcome::Failed(e) => {
            eprintln!("uventet I/O-fejl i dublet-workeren: {e}");
            std::process::exit(18)
        }
    }
}

// ---------------------------------------------------------------------------
// Named tests
// ---------------------------------------------------------------------------

#[test]
fn mutex_dublet_afvises() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    fs::create_dir_all(&state).unwrap();
    let ready = tmp.path().join("ready");
    let release = tmp.path().join("release");
    let slug = "mutex-dup-test-slug";

    let state_s = state.to_string_lossy().into_owned();
    let ready_s = ready.to_string_lossy().into_owned();
    let release_s = release.to_string_lossy().into_owned();

    let exe = std::env::current_exe().unwrap();
    let mut child_a = Command::new(&exe)
        .arg("--exact")
        .arg("hold_lock_until_release")
        .arg("--test-threads=1")
        .arg("--nocapture")
        .env("TALMINAL_WORKER", "hold_lock_until_release")
        .env("TALMINAL_STATE_DIR", &state_s)
        .env("TALMINAL_SLUG", slug)
        .env("TALMINAL_READY_FILE", &ready_s)
        .env("TALMINAL_RELEASE_FILE", &release_s)
        .spawn()
        .expect("spawn A");

    wait_for_file(&ready, Duration::from_secs(15));

    let out_b = spawn_worker(
        "try_acquire_expect_none",
        &[("TALMINAL_STATE_DIR", &state_s), ("TALMINAL_SLUG", slug)],
    );
    assert_eq!(
        out_b.status.code(),
        Some(17),
        "B must see AlreadyRunning (exit 17; 18 = I/O-fejl)\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&out_b.stdout),
        String::from_utf8_lossy(&out_b.stderr)
    );

    fs::write(&release, b"go").unwrap();
    let status_a = child_a.wait().expect("wait A");
    assert!(
        status_a.success(),
        "worker A should exit cleanly: {status_a}"
    );
}

#[test]
fn lock_fil_alene_blokerer() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path();
    fs::create_dir_all(state).unwrap();
    let lock_path = state.join(".lock");
    let _held = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .share_mode(0)
        .open(&lock_path)
        .expect("hold exclusive .lock");

    let outcome = acquire_instance_lock(state, "lockfile-only-slug");
    let tag = outcome_tag(&outcome);
    assert!(
        matches!(outcome, AcquireOutcome::AlreadyRunning),
        "layer-2 exclusive .lock alone must classify as AlreadyRunning, got {tag}"
    );
}

/// M11-regression: `.lock` kan ikke aabnes af en grund der IKKE er en anden
/// proces. Foer klassifikationen blev enhver `Err(_)` til `None`, og begge
/// kaldere afsluttede med exit-kode 0 som var det en helt normal dublet.
#[test]
fn uaabnelig_lock_er_failed_ikke_dublet() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path();
    // `.lock` som MAPPE: CreateFileW afviser den uden FILE_FLAG_BACKUP_SEMANTICS,
    // og fejlkoden er hverken ERROR_SHARING_VIOLATION (32) eller
    // ERROR_LOCK_VIOLATION (33).
    fs::create_dir_all(state.join(".lock")).unwrap();

    let outcome = acquire_instance_lock(state, "lock-is-a-dir-slug");
    let tag = outcome_tag(&outcome);
    assert!(
        matches!(outcome, AcquireOutcome::Failed(_)),
        "en uaabnelig .lock maa rapporteres som Failed, got {tag}"
    );
}

/// M11-regression: `create_dir_all`-fejlen smed sin `io::Error` bogstaveligt
/// vaek (`let _ = e;`) og blev til `None`.
#[test]
fn state_dir_der_er_en_fil_er_failed_ikke_dublet() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state-som-fil");
    fs::write(&state, b"ikke en mappe").unwrap();

    let outcome = acquire_instance_lock(&state, "state-dir-is-a-file-slug");
    let tag = outcome_tag(&outcome);
    assert!(
        matches!(outcome, AcquireOutcome::Failed(_)),
        "state_dir der ikke kan oprettes maa rapporteres som Failed, got {tag}"
    );
}

#[test]
fn mutex_frigives_ved_drop() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path();
    let slug = "drop-reacquire-slug";
    let first = match acquire_instance_lock(state, slug) {
        AcquireOutcome::Acquired(lock) => lock,
        other => panic!("first acquire, got {}", outcome_tag(&other)),
    };
    drop(first);
    let second = match acquire_instance_lock(state, slug) {
        AcquireOutcome::Acquired(lock) => lock,
        other => panic!("re-acquire after drop, got {}", outcome_tag(&other)),
    };
    drop(second);
}

#[test]
fn instance_info_stale_pid_gir_false() {
    let tmp = tempfile::tempdir().unwrap();
    // High unused pid — OpenProcess should fail on Windows.
    write_instance_info(tmp.path(), 4_294_000_000, 0x1234).unwrap();
    let start = Instant::now();
    assert!(!focus_existing_instance(tmp.path()));
    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_millis(800),
        "expected ~1s retry window, got {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "retry window too long: {elapsed:?}"
    );
}

#[test]
fn instance_info_hwnd_ejerskabs_mismatch_gir_false() {
    let tmp = tempfile::tempdir().unwrap();
    let live_pid = std::process::id();
    let foreign_hwnd = unsafe { GetDesktopWindow() } as isize;
    write_instance_info(tmp.path(), live_pid, foreign_hwnd).unwrap();
    assert!(read_instance_info(tmp.path()).is_some());

    let start = Instant::now();
    assert!(!focus_existing_instance(tmp.path()));
    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_millis(800),
        "expected ~1s retry window, got {elapsed:?}"
    );
}

#[test]
fn resolve_startup_home_env_vinder() {
    let _guard = env_lock();
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("env-home");
    fs::create_dir_all(&home).unwrap();
    let global = tmp.path().join("global");
    fs::create_dir_all(&global).unwrap();

    std::env::set_var("TALMINAL_HOME", &home);
    std::env::set_var("TALMINAL_GLOBAL_HOME", &global);
    let got = resolve_startup_home();
    std::env::remove_var("TALMINAL_HOME");
    std::env::remove_var("TALMINAL_GLOBAL_HOME");
    assert_eq!(got.unwrap(), home);
}

#[test]
#[allow(non_snake_case)] // plan-mandated name (empty env = absent)
fn resolve_startup_home_TOM_env_er_fravaerende() {
    let _guard = env_lock();
    let tmp = tempfile::tempdir().unwrap();
    let global = tmp.path().join("global");
    fs::create_dir_all(&global).unwrap();

    std::env::set_var("TALMINAL_HOME", "");
    std::env::set_var("TALMINAL_GLOBAL_HOME", &global);
    // Ensure no last_project hint.
    let _ = fs::remove_file(global.join("last_project"));
    let got = resolve_startup_home().expect("default home");
    std::env::remove_var("TALMINAL_HOME");
    std::env::remove_var("TALMINAL_GLOBAL_HOME");

    assert_eq!(got, global.join("projects").join("default"));
    let meta = project::read_project_meta(&got)
        .unwrap()
        .expect("default meta written");
    assert_eq!(meta.name, "default");
}

#[test]
fn resolve_startup_home_last_project() {
    let _guard = env_lock();
    let tmp = tempfile::tempdir().unwrap();
    let global = tmp.path().join("global");
    fs::create_dir_all(&global).unwrap();

    std::env::remove_var("TALMINAL_HOME");
    std::env::set_var("TALMINAL_GLOBAL_HOME", &global);

    let root_tmp = tempfile::tempdir().unwrap();
    let root = normalize_path(root_tmp.path()).unwrap();
    write_last_project(&root).unwrap();
    let state = project_state_dir(&root).unwrap();
    fs::create_dir_all(&state).unwrap();
    write_project_meta(
        &state,
        &ProjectMeta {
            root: root.clone(),
            name: "from-last".into(),
            added_at: None,
        },
    )
    .unwrap();

    let got = resolve_startup_home();
    std::env::remove_var("TALMINAL_GLOBAL_HOME");
    assert_eq!(got.unwrap(), state);
}

#[test]
fn resolve_startup_home_default_uden_historik() {
    let _guard = env_lock();
    let tmp = tempfile::tempdir().unwrap();
    let global = tmp.path().join("global");
    fs::create_dir_all(&global).unwrap();

    std::env::remove_var("TALMINAL_HOME");
    std::env::set_var("TALMINAL_GLOBAL_HOME", &global);
    let _ = fs::remove_file(global.join("last_project"));

    let got = resolve_startup_home().unwrap();
    std::env::remove_var("TALMINAL_GLOBAL_HOME");
    assert_eq!(got, global.join("projects").join("default"));
    assert!(got.join("project.json").is_file());
}

#[test]
fn resolve_startup_home_korrupt_meta_er_err() {
    let _guard = env_lock();
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("corrupt-home");
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("project.json"), b"not-json{{{").unwrap();
    let global = tmp.path().join("global");
    fs::create_dir_all(&global).unwrap();

    std::env::set_var("TALMINAL_HOME", &home);
    std::env::set_var("TALMINAL_GLOBAL_HOME", &global);
    let got = resolve_startup_home();
    std::env::remove_var("TALMINAL_HOME");
    std::env::remove_var("TALMINAL_GLOBAL_HOME");
    assert!(got.is_err(), "corrupt project.json must be fatal Err");
}
