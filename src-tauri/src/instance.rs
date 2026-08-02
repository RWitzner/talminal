//! App-instance lock, instance.json, bare-start home resolution (B-light Task 2).

use crate::project::{
    global_base, project_state_dir, read_last_project, read_project_meta, write_project_meta,
    ProjectMeta,
};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, SetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetWindowThreadProcessId, IsIconic, IsWindow, SetForegroundWindow, ShowWindow,
    SW_RESTORE,
};

/// Holds the named mutex HANDLE and exclusive `.lock` file for process lifetime.
pub struct InstanceLock {
    mutex: HANDLE,
    _lock_file: File,
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        if !self.mutex.is_null() {
            unsafe {
                CloseHandle(self.mutex);
            }
            self.mutex = std::ptr::null_mut();
        }
        // `_lock_file` drop releases the exclusive share-mode(0) lock.
    }
}

/// Win32-koderne der BETYDER "en anden proces holder filen aabent eksklusivt".
/// De staves lokalt som `i32` fordi `io::Error::raw_os_error()` giver `i32`,
/// mens windows-sys' konstanter er `u32` — en lokal konstant sparer en cast i
/// hvert eneste moenster.
const ERROR_SHARING_VIOLATION: i32 = 32;
const ERROR_LOCK_VIOLATION: i32 = 33;

/// Udfaldet af [`acquire_instance_lock`] (fund M11).
///
/// Foer havde funktionen én `Option`, og FEM forskellige udfald kollapsede til
/// `None`: mutex-oprettelsen fejlede, mutexen fandtes allerede (aegte dublet),
/// `create_dir_all` fejlede, `.lock` var laast af en anden proces (aegte
/// dublet), og `.lock`-aabningen fejlede af en HVILKEN SOM HELST anden grund —
/// ACCESS_DENIED, fuld disk, read-only mappe, en for lang sti. Begge kaldere
/// laeste `None` som "koerer allerede" og afsluttede med exit-kode 0, saa en
/// reel I/O-fejl saa ud som en helt normal start. Paa ejerens egen maskine er
/// det sjaeldent; efter open-source-launch er det praecis den slags der giver
/// ubrugelige fejlrapporter ("appen starter bare ikke"). Derfor tre udfald, ikke
/// to — og fejlen baeres med ud.
///
/// GRAENSEN for klassifikationen: holder en TREDJEPART `.lock` eksklusivt (en
/// AV-/backup-scanner, en editor), giver aabningen ERROR_SHARING_VIOLATION —
/// byte for byte det samme svar som en aegte dublet. Det er derfor
/// `AlreadyRunning`, ikke `Failed`, og det er ikke til at goere bedre herfra:
/// Win32 fortaeller ikke HVEM der holder filen.
pub enum AcquireOutcome {
    /// Begge lag taget; laasen holdes til `InstanceLock` droppes.
    Acquired(InstanceLock),
    /// En anden instans for samme slug koerer: navngiven mutex fandtes, eller
    /// `.lock` var holdt eksklusivt af en anden proces.
    AlreadyRunning,
    /// Vi ved det IKKE — laasen kunne ikke afgoeres pga. en I/O-/OS-fejl.
    /// Kalderen skal fejle synligt, ikke afslutte tavst som dublet.
    Failed(io::Error),
}

/// Two-layer duplicate guard (M7): named mutex `Talminal-<slug>` + exclusive `state_dir\.lock`.
/// Both must succeed; partial success rolls back (mutex-handlen lukkes).
///
/// Klassifikationen er bindende (M11): kun `ERROR_ALREADY_EXISTS` paa mutexen og
/// ERROR_SHARING_VIOLATION/ERROR_LOCK_VIOLATION paa `.lock` er en aegte dublet.
/// Alt andet er `Failed(e)` — se [`AcquireOutcome`].
pub fn acquire_instance_lock(state_dir: &Path, slug: &str) -> AcquireOutcome {
    let mutex_name = format!("Talminal-{slug}");
    let wide = to_wide(&mutex_name);
    let mutex = unsafe {
        SetLastError(0);
        CreateMutexW(std::ptr::null(), 0, wide.as_ptr())
    };
    if mutex.is_null() {
        // Fejlkoden hentes FOER noget andet kald naar at overskrive
        // thread-last-error.
        return AcquireOutcome::Failed(io::Error::last_os_error());
    }
    let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if already {
        unsafe {
            CloseHandle(mutex);
        }
        return AcquireOutcome::AlreadyRunning;
    }

    if let Err(e) = fs::create_dir_all(state_dir) {
        unsafe {
            CloseHandle(mutex);
        }
        // En state-dir der ikke kan oprettes er en driftsfejl, aldrig en dublet:
        // en KOERENDE instans har pr. konstruktion allerede sin mappe.
        return AcquireOutcome::Failed(e);
    }
    let lock_path = state_dir.join(".lock");
    let lock_file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .share_mode(0)
        .open(&lock_path);
    match lock_file {
        Ok(file) => AcquireOutcome::Acquired(InstanceLock {
            mutex,
            _lock_file: file,
        }),
        Err(e) => {
            unsafe {
                CloseHandle(mutex);
            }
            match e.raw_os_error() {
                Some(ERROR_SHARING_VIOLATION) | Some(ERROR_LOCK_VIOLATION) => {
                    AcquireOutcome::AlreadyRunning
                }
                _ => AcquireOutcome::Failed(e),
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstanceInfo {
    pid: u32,
    hwnd: isize,
}

pub fn write_instance_info(state_dir: &Path, pid: u32, hwnd: isize) -> io::Result<()> {
    fs::create_dir_all(state_dir)?;
    let info = InstanceInfo { pid, hwnd };
    let mut body = serde_json::to_string_pretty(&info).map_err(io::Error::other)?;
    body.push('\n');
    crate::atomic::write(&state_dir.join("instance.json"), body.as_bytes())
}

pub fn read_instance_info(state_dir: &Path) -> Option<(u32, isize)> {
    let text = fs::read_to_string(state_dir.join("instance.json")).ok()?;
    let info: InstanceInfo = serde_json::from_str(&text).ok()?;
    Some((info.pid, info.hwnd))
}

/// Bounded retry focus of an already-running instance. Returns `true` only if
/// `SetForegroundWindow` reported success.
pub fn focus_existing_instance(state_dir: &Path) -> bool {
    for _ in 0..5 {
        if let Some((pid, hwnd)) = read_instance_info(state_dir) {
            if pid_is_live(pid) && window_owned_by_pid(hwnd, pid) {
                if focus_hwnd(hwnd) {
                    return true;
                }
            } else if let Some(fallback) = find_window_for_project(state_dir) {
                // Stale/mismatched instance.json — title fallback.
                if focus_hwnd(fallback) {
                    return true;
                }
            }
        } else if let Some(fallback) = find_window_for_project(state_dir) {
            if focus_hwnd(fallback) {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

/// Bare-start / env home resolution. Corrupt `project.json` in the chosen state-dir is `Err`.
pub fn resolve_startup_home() -> Result<PathBuf, String> {
    if let Some(home) = env_nonempty("TALMINAL_HOME") {
        let home = PathBuf::from(home);
        validate_project_meta(&home)?;
        return Ok(home);
    }

    if let Some(root) = read_last_project() {
        if root.is_dir() {
            let state = project_state_dir(&root)?;
            // "Fjern fra listen" (listing::hide) skal også gælde NÆSTE opstart.
            // Uden gaten er handlingen en der fortryder sig selv: projektet
            // forsvinder fra rail'en og kommer tilbage som opstarts-workspace
            // næste gang appen startes bart.
            //
            // Et skjult hint er ikke en FEJL — det falder blot igennem til
            // default-grenen nedenfor, præcis som de tre eksisterende
            // fald-igennem-veje (intet hint, roden findes ikke, tomt hint).
            // Kun `validate_project_meta` må fælde opstarten her.
            //
            // Rækkefølgen er bindende: gaten ligger EFTER `project_state_dir`
            // (som kan fejle på canonicalisering) og FØR `validate_project_meta`,
            // så et skjult projekt med korrupt project.json heller ikke fælder
            // opstarten.
            if !state.join(".hidden").exists() {
                validate_project_meta(&state)?;
                return Ok(state);
            }
        }
    }

    let state = global_base().join("projects").join("default");
    fs::create_dir_all(&state).map_err(|e| format!("default state dir create failed: {e}"))?;
    match read_project_meta(&state)? {
        Some(_) => {}
        None => {
            let meta = ProjectMeta {
                root: user_home_dir()?,
                name: "default".into(),
                added_at: None,
            };
            write_project_meta(&state, &meta)
                .map_err(|e| format!("default project.json write failed: {e}"))?;
        }
    }
    Ok(state)
}

/// User profile directory (`USERPROFILE`, else `HOME`).
pub fn user_home_dir() -> Result<PathBuf, String> {
    std::env::var_os("USERPROFILE")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("HOME").filter(|v| !v.is_empty()))
        .map(PathBuf::from)
        .ok_or_else(|| "USERPROFILE/HOME must be set".into())
}

fn validate_project_meta(state_dir: &Path) -> Result<(), String> {
    // Ok(None)/Ok(Some) both fine; Err (corrupt) propagates — M13.
    let _ = read_project_meta(state_dir)?;
    Ok(())
}

fn env_nonempty(key: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(key).filter(|v| !v.is_empty())
}

fn pid_is_live(pid: u32) -> bool {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        CloseHandle(handle);
        true
    }
}

fn window_owned_by_pid(hwnd: isize, pid: u32) -> bool {
    if hwnd == 0 {
        return false;
    }
    let hwnd = hwnd as HWND;
    unsafe {
        if IsWindow(hwnd) == 0 {
            return false;
        }
        let mut win_pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut win_pid);
        win_pid == pid
    }
}

fn focus_hwnd(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    let hwnd = hwnd as HWND;
    unsafe {
        if IsWindow(hwnd) == 0 {
            return false;
        }
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd) != 0
    }
}

fn find_window_for_project(state_dir: &Path) -> Option<isize> {
    let meta = read_project_meta(state_dir).ok().flatten()?;
    let wide = to_wide(&meta.name);
    let hwnd = unsafe { FindWindowW(std::ptr::null(), wide.as_ptr()) };
    if hwnd.is_null() {
        None
    } else {
        Some(hwnd as isize)
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
