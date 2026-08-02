//! Project identity: root discovery, slug, state-dir layout, project.json + last_project.
//!
//! B-light Task 1 — see spec §3 and plan Task 1.

use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ProjectMeta {
    pub root: PathBuf,
    pub name: String,
    /// ISO-Z. Sættes ved førstegangs-registrering; ældre filer har den ikke.
    #[serde(default)]
    pub added_at: Option<String>,
}

/// `fs::canonicalize` + strip of Windows extended-length prefix.
/// Persisted paths must always be this form — never `\\?\…`.
pub fn normalize_path(p: &Path) -> Result<PathBuf, String> {
    let canon =
        fs::canonicalize(p).map_err(|e| format!("canonicalize failed for {}: {e}", p.display()))?;
    Ok(strip_extended_prefix(&canon))
}

/// Walk up to nearest `.git` entry (directory OR file); if none, the start dir itself.
pub fn find_project_root(start: &Path) -> Result<PathBuf, String> {
    let normalized = normalize_path(start)?;
    let mut cur = normalized.clone();
    loop {
        let git = cur.join(".git");
        if git.exists() {
            return Ok(cur);
        }
        match cur.parent() {
            Some(parent) if parent != cur.as_path() => cur = parent.to_path_buf(),
            _ => break,
        }
    }
    Ok(normalized)
}

/// `normalize_path` → `to_string_lossy` → lowercase.
pub fn canonical_key(root: &Path) -> Result<String, String> {
    Ok(normalize_path(root)?.to_string_lossy().to_ascii_lowercase())
}

/// `"<basename>-<hex8(SHA-256(canonical_key))>"`.
pub fn project_slug(root: &Path) -> Result<String, String> {
    let normalized = normalize_path(root)?;
    let basename = normalized
        .file_name()
        .ok_or_else(|| format!("project root has no basename: {}", normalized.display()))?
        .to_string_lossy();
    let key = canonical_key(&normalized)?;
    let digest = Sha256::digest(key.as_bytes());
    let hex8 = digest
        .iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    Ok(format!("{basename}-{hex8}"))
}

/// `TALMINAL_GLOBAL_HOME` override (empty = absent), else `%LOCALAPPDATA%\Talminal`.
pub fn global_base() -> PathBuf {
    if let Some(home) = std::env::var_os("TALMINAL_GLOBAL_HOME") {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    let localappdata = std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .expect("LOCALAPPDATA must be set (Windows-only app)");
    PathBuf::from(localappdata).join("Talminal")
}

/// `global_base()/projects/<slug>`.
pub fn project_state_dir(root: &Path) -> Result<PathBuf, String> {
    Ok(global_base().join("projects").join(project_slug(root)?))
}

/// `Ok(None)` = missing file; `Err` = present but corrupt/unparseable.
pub fn read_project_meta(state_dir: &Path) -> Result<Option<ProjectMeta>, String> {
    let path = state_dir.join("project.json");
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("project.json read failed: {e}")),
    };
    let meta: ProjectMeta =
        serde_json::from_str(&text).map_err(|e| format!("project.json parse failed: {e}"))?;
    Ok(Some(meta))
}

/// `create_dir_all` + unique-temp + rename. Persists `root` in normalize_path form.
pub fn write_project_meta(state_dir: &Path, meta: &ProjectMeta) -> io::Result<()> {
    fs::create_dir_all(state_dir)?;
    let to_write = ProjectMeta {
        root: normalize_path(&meta.root).map_err(io::Error::other)?,
        name: meta.name.clone(),
        added_at: meta.added_at.clone(),
    };
    let mut body = serde_json::to_string_pretty(&to_write).map_err(io::Error::other)?;
    body.push('\n');
    crate::atomic::write(&state_dir.join("project.json"), body.as_bytes())
}

/// Hint only: corrupt/empty → `None`.
pub fn read_last_project() -> Option<PathBuf> {
    let path = global_base().join("last_project");
    let text = fs::read_to_string(path).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

/// Unique-temp + rename. Persists normalize_path form.
pub fn write_last_project(root: &Path) -> io::Result<()> {
    let normalized = normalize_path(root).map_err(io::Error::other)?;
    skriv_hint(&global_base(), &normalized)
}

/// Skriver `last_project`-hintet for det workspace der FAKTISK er blevet synligt
/// (frossen kontrakt B, CORRECTIONS.md afsnit B — ejer: T3). Pollertråden kalder
/// den i ack-grenen, så hintet følger den workspace-overdragelse der rent
/// faktisk lykkedes, i stedet for kun at blive skrevet én gang ved app-opstart.
///
/// **No-op når `slug == "default"`.** Default-workspacets root ER brugerens
/// hjemmemappe, men dets state-dir er `projects/default` — ikke
/// `project_state_dir(home)`, som er hash-baseret (project.rs:79). Skrev vi
/// hintet, ville næste opstart resolve en ANDEN mappe: et tomt tvillinge-workspace.
///
/// Roden læses fra `projects/<slug>/project.json` og skrives VERBATIM. Den er
/// allerede persisteret i `normalize_path`-form, og et hint må ikke fejle bare
/// fordi mappen er midlertidigt utilgængelig (netværksdrev, afmonteret disk) —
/// `normalize_path` ville kræve at stien kan canonicaliseres lige nu.
/// Manglende/korrupt `project.json` er `Ok(())`: der er intet hint at skrive.
pub fn write_last_project_for_active(global_base: &Path, slug: &str) -> io::Result<()> {
    if slug == "default" {
        return Ok(());
    }
    let state_dir = global_base.join("projects").join(slug);
    let Ok(Some(meta)) = read_project_meta(&state_dir) else {
        return Ok(());
    };
    skriv_hint(global_base, &meta.root)
}

fn skriv_hint(global_base: &Path, root: &Path) -> io::Result<()> {
    let path = global_base.join("last_project");
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let body = format!("{}\n", root.display());
    crate::atomic::write(&path, body.as_bytes())
}

fn strip_extended_prefix(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn find_project_root_gaar_op_til_git_mappe() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        let nested = root.join("a").join("b");
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();

        let got = find_project_root(&nested).unwrap();
        assert_eq!(got, normalize_path(&root).unwrap());
    }

    #[test]
    #[allow(non_snake_case)] // plan-mandated name (`.git` FILE form)
    fn find_project_root_git_FIL_taeller_som_rod() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("worktree");
        let nested = root.join("src");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join(".git"), "gitdir: /tmp/somewhere\n").unwrap();

        let got = find_project_root(&nested).unwrap();
        assert_eq!(got, normalize_path(&root).unwrap());
    }

    #[test]
    fn find_project_root_uden_git_er_mappen_selv() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("plain");
        fs::create_dir_all(&dir).unwrap();

        let got = find_project_root(&dir).unwrap();
        assert_eq!(got, normalize_path(&dir).unwrap());
    }

    #[test]
    fn find_project_root_ikke_eksisterende_sti_er_err() {
        let missing = PathBuf::from(r"C:\talminal-definitely-missing-path-xyz\nope");
        assert!(find_project_root(&missing).is_err());
    }

    #[test]
    fn normalize_path_stripper_extended_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let canon = fs::canonicalize(tmp.path()).unwrap();
        let normalized = normalize_path(tmp.path()).unwrap();

        assert!(
            !normalized.to_string_lossy().starts_with(r"\\?\"),
            "normalized must not keep extended prefix: {normalized:?}"
        );
        assert_eq!(normalized, strip_extended_prefix(&canon));
    }

    #[test]
    fn normalize_path_unc_helper() {
        assert_eq!(
            strip_extended_prefix(Path::new(r"\\?\UNC\srv\share\foo")),
            PathBuf::from(r"\\srv\share\foo")
        );
        assert_eq!(
            strip_extended_prefix(Path::new(r"\\?\C:\Users\x")),
            PathBuf::from(r"C:\Users\x")
        );
    }

    #[test]
    fn project_slug_format_og_determinisme() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let s1 = project_slug(root).unwrap();
        let basename = normalize_path(root)
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();

        assert!(
            s1.starts_with(&format!("{basename}-")),
            "slug {s1:?} should start with basename {basename:?}"
        );
        let suffix = &s1[basename.len() + 1..];
        assert_eq!(suffix.len(), 8);
        assert!(
            suffix.chars().all(|c| c.is_ascii_hexdigit()),
            "suffix must be hex8: {suffix}"
        );

        // Same path, alternate casing → same slug (canonical_key lowercases).
        let norm = normalize_path(root).unwrap();
        let flipped: String = norm
            .to_string_lossy()
            .chars()
            .map(|c| {
                if c.is_ascii_uppercase() {
                    c.to_ascii_lowercase()
                } else if c.is_ascii_lowercase() {
                    c.to_ascii_uppercase()
                } else {
                    c
                }
            })
            .collect();
        let s2 = project_slug(Path::new(&flipped)).unwrap();
        assert_eq!(s1, s2);
        assert_eq!(s1, project_slug(root).unwrap());
    }

    #[test]
    fn project_slug_kollision_samme_basename_forskellig_sti() {
        let tmp = tempfile::tempdir().unwrap();
        let p1 = tmp.path().join("a").join("foo");
        let p2 = tmp.path().join("b").join("foo");
        fs::create_dir_all(&p1).unwrap();
        fs::create_dir_all(&p2).unwrap();

        let s1 = project_slug(&p1).unwrap();
        let s2 = project_slug(&p2).unwrap();
        assert_ne!(s1, s2);
        assert!(s1.starts_with("foo-"));
        assert!(s2.starts_with("foo-"));
    }

    #[test]
    fn global_base_respekterer_env_override() {
        let _guard = env_lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("TALMINAL_GLOBAL_HOME", tmp.path());
        let got = global_base();
        std::env::remove_var("TALMINAL_GLOBAL_HOME");
        assert_eq!(got, tmp.path());
    }

    #[test]
    fn tom_env_er_fravaerende() {
        let _guard = env_lock();
        std::env::set_var("TALMINAL_GLOBAL_HOME", "");
        let got = global_base();
        std::env::remove_var("TALMINAL_GLOBAL_HOME");
        let expected = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap()).join("Talminal");
        assert_eq!(got, expected);
    }

    #[test]
    fn project_meta_roundtrip() {
        let state = tempfile::tempdir().unwrap();
        let root_tmp = tempfile::tempdir().unwrap();
        let meta = ProjectMeta {
            root: normalize_path(root_tmp.path()).unwrap(),
            name: "demo".into(),
            added_at: None,
        };
        write_project_meta(state.path(), &meta).unwrap();
        assert_eq!(read_project_meta(state.path()).unwrap(), Some(meta));
    }

    #[test]
    fn read_project_meta_fravaer_er_ok_none() {
        let state = tempfile::tempdir().unwrap();
        assert_eq!(read_project_meta(state.path()).unwrap(), None);
    }

    #[test]
    fn read_project_meta_korrupt_er_err() {
        let state = tempfile::tempdir().unwrap();
        fs::write(state.path().join("project.json"), b"not-json{{{").unwrap();
        assert!(read_project_meta(state.path()).is_err());
    }

    #[test]
    fn last_project_roundtrip() {
        let _guard = env_lock();
        let g = tempfile::tempdir().unwrap();
        std::env::set_var("TALMINAL_GLOBAL_HOME", g.path());

        let root_tmp = tempfile::tempdir().unwrap();
        let root = normalize_path(root_tmp.path()).unwrap();
        write_last_project(&root).unwrap();
        let got = read_last_project();

        std::env::remove_var("TALMINAL_GLOBAL_HOME");
        assert_eq!(got, Some(root));
    }

    #[test]
    fn last_project_for_active_skriver_det_synlige_workspaces_rod() {
        let g = tempfile::tempdir().unwrap();
        let state = g.path().join("projects").join("alpha-1111");
        fs::create_dir_all(&state).unwrap();
        fs::write(
            state.join("project.json"),
            br#"{"root":"C:\\r\\alpha","name":"alpha","added_at":null}"#,
        )
        .unwrap();

        write_last_project_for_active(g.path(), "alpha-1111").unwrap();
        assert_eq!(
            fs::read_to_string(g.path().join("last_project"))
                .unwrap()
                .trim(),
            r"C:\r\alpha",
            "hintet skal pege paa det workspace der blev synligt"
        );
    }

    #[test]
    fn last_project_for_active_er_et_no_op_for_default() {
        // Default-workspacets root er hjemmemappen, men dets state-dir er
        // projects/default. Et hint ville sende naeste opstart til
        // project_state_dir(home) — en hash-baseret, TOM tvillingemappe.
        let g = tempfile::tempdir().unwrap();
        let state = g.path().join("projects").join("default");
        fs::create_dir_all(&state).unwrap();
        fs::write(
            state.join("project.json"),
            br#"{"root":"C:\\Users\\r","name":"default","added_at":null}"#,
        )
        .unwrap();

        write_last_project_for_active(g.path(), "default").unwrap();
        assert!(
            !g.path().join("last_project").exists(),
            "default maa ALDRIG skrive et last_project-hint"
        );
    }

    #[test]
    fn last_project_for_active_uden_project_json_er_ok() {
        let g = tempfile::tempdir().unwrap();
        write_last_project_for_active(g.path(), "findes-ikke").unwrap();
        assert!(!g.path().join("last_project").exists());
    }

    #[test]
    fn last_project_korrupt_er_none() {
        let _guard = env_lock();
        let g = tempfile::tempdir().unwrap();
        std::env::set_var("TALMINAL_GLOBAL_HOME", g.path());
        fs::write(g.path().join("last_project"), b"\n\t  \n").unwrap();
        let got = read_last_project();
        std::env::remove_var("TALMINAL_GLOBAL_HOME");
        assert_eq!(got, None);
    }
}
