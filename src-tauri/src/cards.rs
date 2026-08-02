//! cards.toml-parsing: kort-definitioner for canvas-gridet (ejer-beslutning 4).
//!
//! Format (kanonisk, se planens skelet):
//!   [master]  — SUPERVISION-ONLY (Task 3, låst ejer-beslutning: master-kortet
//!               er ude af MVP-pathen). Default-state accepterer sektionen men
//!               IGNORERER den med logline; med `--features supervision`
//!               parses den som master-kort (Task 11), name = MASTER_NAME.
//!   [[card]]  — 0..n worker-kort; command/resume_command har claude-defaults

use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Reserveret navn: [master]-sektionens CardConfig.name (supervision-sporet;
/// i default-state er "master" et almindeligt tilladt kortnavn — Task 3).
#[cfg(feature = "supervision")]
pub const MASTER_NAME: &str = "master";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardConfig {
    pub name: String,
    pub cwd: PathBuf,
    pub command: Vec<String>,
    pub resume_command: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CardsFile {
    /// Supervision-sporet (Task 3): master-kortet er ude af MVP-pathen —
    /// feltet findes slet ikke i default-state.
    #[cfg(feature = "supervision")]
    pub master: Option<CardConfig>,
    pub cards: Vec<CardConfig>,
}

#[derive(Debug)]
pub enum CardsError {
    /// Filen findes ikke: appen viser stien og starter med tomt grid (Task 8).
    NotFound(PathBuf),
    Io(std::io::Error),
    Parse(String),
    Invalid(String),
}

impl fmt::Display for CardsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CardsError::NotFound(p) => write!(f, "cards.toml not found: {}", p.display()),
            CardsError::Io(e) => write!(f, "cards.toml io error: {e}"),
            CardsError::Parse(msg) => write!(f, "cards.toml parse error: {msg}"),
            CardsError::Invalid(msg) => write!(f, "cards.toml invalid: {msg}"),
        }
    }
}

impl std::error::Error for CardsError {}

fn default_command() -> Vec<String> {
    vec!["claude".to_string()]
}

fn default_resume_command() -> Vec<String> {
    vec!["claude".to_string(), "--continue".to_string()]
}

/// Raa serde-spejle af TOML-formatet. deny_unknown_fields fanger tastefejl
/// i en haandredigeret fil som Parse-fejl i stedet for tavs ignorering.
#[cfg(feature = "supervision")]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMaster {
    cwd: PathBuf,
    /// Fix F21: BEVIDST ingen default — [master] uden command er en
    /// Invalid-fejl (se load_cards). Master er en read-only feed
    /// (ejer-beslutning 1, Task 11); en claude-default ville spawne en
    /// fuld interaktiv claude-session i master-pladsen.
    command: Option<Vec<String>>,
    #[serde(default = "default_resume_command")]
    #[allow(dead_code)] // accepteret men ignoreret: master's resume er altid command (Task 11)
    resume_command: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCard {
    name: String,
    cwd: PathBuf,
    #[serde(default = "default_command")]
    command: Vec<String>,
    #[serde(default = "default_resume_command")]
    resume_command: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCardsFile {
    /// Supervision: fuld [master]-parse. Default-state (Task 3): sektionen
    /// accepteres som raa TOML-værdi men IGNORERES wholesale (logline i
    /// load_cards) — en [master] i en eksisterende cards.toml må ALDRIG
    /// knække MVP-loadet.
    #[cfg(feature = "supervision")]
    master: Option<RawMaster>,
    #[cfg(not(feature = "supervision"))]
    master: Option<toml::Value>,
    #[serde(default, rename = "card")]
    cards: Vec<RawCard>,
}

/// Indlaeser og validerer cards.toml. Manglende fil er en SAERSKILT gren
/// (CardsError::NotFound) - Task 8 oversaetter den til tomt grid + vist sti.
pub fn load_cards(path: &Path) -> Result<CardsFile, CardsError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(CardsError::NotFound(path.to_path_buf()));
        }
        Err(e) => return Err(CardsError::Io(e)),
    };
    let raw: RawCardsFile = toml::from_str(&text).map_err(|e| CardsError::Parse(e.to_string()))?;

    // Task 3 (låst ejer-beslutning: master-kortet ude af MVP-pathen):
    // default-state ignorerer [master] med logline — fejler ALDRIG på den
    // (heller ikke på en [master] der ville være Invalid under supervision).
    #[cfg(not(feature = "supervision"))]
    {
        if raw.master.is_some() {
            eprintln!(
                "cards.toml: [master] section ignored - master card is off the MVP path (supervision parked)"
            );
        }
    }

    // Fix F21: master har INGEN claude-default — command er OBLIGATORISK
    // (Invalid ved fravær). Task 11 (ejer-beslutning 1): en feed-tail har
    // ingen --continue — master's resume_command er ALTID command. Et
    // eksplicit resume_command i [master] accepteres af parseren, men
    // ignoreres (testet i cards_master.rs).
    #[cfg(feature = "supervision")]
    let master = match raw.master {
        Some(m) => {
            let command = m.command.ok_or_else(|| {
                CardsError::Invalid(
                    "master: command is required - master-kortet er en read-only feed, ingen claude-default"
                        .to_string(),
                )
            })?;
            Some(CardConfig {
                name: MASTER_NAME.to_string(),
                cwd: m.cwd,
                resume_command: command.clone(),
                command,
            })
        }
        None => None,
    };
    let cards = raw
        .cards
        .into_iter()
        .map(|c| CardConfig {
            name: c.name,
            cwd: c.cwd,
            command: c.command,
            resume_command: c.resume_command,
        })
        .collect();

    #[cfg(feature = "supervision")]
    let file = CardsFile { master, cards };
    #[cfg(not(feature = "supervision"))]
    let file = CardsFile { cards };
    validate(&file)?;
    Ok(file)
}

fn validate(file: &CardsFile) -> Result<(), CardsError> {
    // Case-insensitivt dedup: kortnavne bliver filnavne (signals\<name>.pause.json)
    // og Windows-filsystemet er case-insensitivt.
    let mut seen: HashSet<String> = HashSet::new();
    for card in &file.cards {
        validate_name(&card.name)?;
        if !seen.insert(card.name.to_ascii_lowercase()) {
            return Err(CardsError::Invalid(format!(
                "duplicate card name: {}",
                card.name
            )));
        }
        validate_commands(card)?;
    }
    #[cfg(feature = "supervision")]
    {
        if let Some(master) = &file.master {
            validate_commands(master)?;
        }
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), CardsError> {
    if name.is_empty() {
        return Err(CardsError::Invalid(
            "card name must not be empty".to_string(),
        ));
    }
    // Task 3: reservationen af navnet "master" er supervision-only — i
    // default-state findes master-kortet ikke, og navnet er almindeligt.
    #[cfg(feature = "supervision")]
    {
        if name.eq_ignore_ascii_case(MASTER_NAME) {
            return Err(CardsError::Invalid(format!(
                "card name '{name}' is reserved for the master card"
            )));
        }
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CardsError::Invalid(format!(
            "card name '{name}' has invalid characters (allowed: A-Za-z0-9_-)"
        )));
    }
    Ok(())
}

fn validate_commands(card: &CardConfig) -> Result<(), CardsError> {
    if card.command.is_empty() {
        return Err(CardsError::Invalid(format!(
            "card '{}': command must not be empty",
            card.name
        )));
    }
    if card.resume_command.is_empty() {
        return Err(CardsError::Invalid(format!(
            "card '{}': resume_command must not be empty",
            card.name
        )));
    }
    Ok(())
}

/// Basemappe: TALMINAL_HOME-env (test-override), ellers %LOCALAPPDATA%\Talminal
/// — samme opløsning som controllerens config.default_paths().
pub fn talminal_base() -> PathBuf {
    resolve_base(env::var_os("TALMINAL_HOME"), env::var_os("LOCALAPPDATA"))
}

/// `<base>\cards.toml` (ejer-beslutning 4).
pub fn default_cards_path() -> PathBuf {
    talminal_base().join("cards.toml")
}

fn resolve_base(talminal_home: Option<OsString>, localappdata: Option<OsString>) -> PathBuf {
    if let Some(home) = talminal_home {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    let localappdata = localappdata
        .filter(|v| !v.is_empty())
        .expect("LOCALAPPDATA must be set (Windows-only app, spec section 20.1)");
    PathBuf::from(localappdata).join("Talminal")
}

/// Slaar et kort op ved navn. Master skygger ALTID et evt. [[card]] med
/// navnet "master". Defense-in-depth: load_cards' validering (Task 7) afviser
/// navnet allerede ved parse, og runtime-opslaget (Task 8) gaar via
/// HashMap'en, som populeres fra samme parsede CardsFile — skyggen her
/// daekker manuelt konstruerede CardsFile-vaerdier. Findes ingen [master],
/// falder "master" igennem til cards-listen.
pub fn resolve_card<'a>(file: &'a CardsFile, name: &str) -> Option<&'a CardConfig> {
    // Task 3: master-skyggen er supervision-only (default-state har intet
    // master-felt) — default-state slår altid op i cards-listen alene.
    #[cfg(feature = "supervision")]
    {
        if name == MASTER_NAME {
            if let Some(master) = file.master.as_ref() {
                return Some(master);
            }
        }
    }
    file.cards.iter().find(|c| c.name == name)
}

/// Task 11 (ejer-beslutning 1): mekanisk backend-guard — master er et
/// read-only feed-viewport. Kaldes som FOERSTE linje i write_pty (main.rs),
/// saa en frontend-fejl aldrig kan skrive bytes eller udloese
/// pause-signalfiler for master.
#[cfg(feature = "supervision")]
pub fn guard_write_pty(name: &str) -> Result<(), String> {
    if name == MASTER_NAME {
        return Err("master_readonly".to_string());
    }
    Ok(())
}

/// Default-state (Task 3): master-kortet er ude af MVP-pathen — ingen kort er
/// read-only, guarden er en bevidst no-op. Kald-stedet i main.rs er dermed
/// ens i begge feature-states (testet i tests/master_off.rs).
#[cfg(not(feature = "supervision"))]
pub fn guard_write_pty(_name: &str) -> Result<(), String> {
    Ok(())
}

/// Task 11: master har ingen pause-semantik at genoptage (ingen epoch-bump,
/// ingen signalfiler). Kaldes som FOERSTE linje i resume_card_control (main.rs).
#[cfg(feature = "supervision")]
pub fn guard_resume_control(name: &str) -> Result<(), String> {
    if name == MASTER_NAME {
        return Err("master_has_no_pause".to_string());
    }
    Ok(())
}

/// Default-state (Task 3): no-op — se guard_write_pty ovenfor.
#[cfg(not(feature = "supervision"))]
pub fn guard_resume_control(_name: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Skriver `contents` til en frisk tempdir og kalder load_cards på den.
    fn load_str(contents: &str) -> Result<CardsFile, CardsError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cards.toml");
        std::fs::write(&path, contents).unwrap();
        load_cards(&path)
    }

    // Supervision-only (Task 3): [master]-parse findes ikke i default-state —
    // default-state-adfaerden (sektionen ignoreres) er testet i tests/master_off.rs.
    #[cfg(feature = "supervision")]
    #[test]
    fn full_file_parses_master_and_cards() {
        let file = load_str(
            r#"
[master]
command = ["uv", "run", "--directory", "C:/projekter/demo/controller", "persona", "feed", "--follow"]
cwd = "C:/projekter/demo"

[[card]]
name = "a"
cwd = "C:/code/proj-a"
command = ["claude"]
resume_command = ["claude", "--continue"]

[[card]]
name = "b"
cwd = "C:/code/proj-b"
"#,
        )
        .unwrap();

        let master = file.master.expect("master section present");
        assert_eq!(master.name, MASTER_NAME);
        assert_eq!(master.cwd, PathBuf::from("C:/projekter/demo"));
        assert_eq!(master.command.len(), 7);
        assert_eq!(master.command[0], "uv");
        // Task 11: master's resume er ALTID command (feed-tail, ingen --continue)
        assert_eq!(master.resume_command, master.command);

        assert_eq!(file.cards.len(), 2);
        assert_eq!(file.cards[0].name, "a");
        assert_eq!(file.cards[0].command, ["claude"]);
        assert_eq!(file.cards[0].resume_command, ["claude", "--continue"]);
        assert_eq!(file.cards[1].name, "b");
        assert_eq!(file.cards[1].cwd, PathBuf::from("C:/code/proj-b"));
    }

    #[test]
    fn card_defaults_applied() {
        let file = load_str("[[card]]\nname = \"a\"\ncwd = \"C:/code/proj-a\"\n").unwrap();
        assert_eq!(file.cards[0].command, ["claude"]);
        assert_eq!(file.cards[0].resume_command, ["claude", "--continue"]);
    }

    #[cfg(feature = "supervision")]
    #[test]
    fn master_without_command_is_invalid() {
        // Fix F21: master er en read-only feed (låst ejer-beslutning 1,
        // Task 11) — INGEN claude-default. [master] uden command ville
        // ellers spawne en fuld interaktiv claude-session i master-pladsen.
        // Fejlen er Invalid (med forklarende besked), ikke en rå toml-fejl.
        let res = load_str("[master]\ncwd = \"C:/projekter/demo\"\n");
        match res {
            Err(CardsError::Invalid(msg)) => {
                assert!(msg.contains("master"), "besked skal nævne master: {msg}");
                assert!(msg.contains("command is required"), "besked: {msg}");
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn missing_file_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cards.toml");
        match load_cards(&path) {
            Err(CardsError::NotFound(p)) => assert_eq!(p, path),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn invalid_toml_is_parse_error() {
        assert!(matches!(
            load_str("this is not toml ["),
            Err(CardsError::Parse(_))
        ));
    }

    #[test]
    fn unknown_key_is_parse_error() {
        // deny_unknown_fields fanger tastefejl i en haandredigeret fil
        let res = load_str("[[card]]\nname = \"a\"\ncwd = \"C:/x\"\ncomand = [\"typo\"]\n");
        assert!(matches!(res, Err(CardsError::Parse(_))));
    }

    #[test]
    fn empty_file_is_empty_config() {
        let file = load_str("").unwrap();
        #[cfg(feature = "supervision")]
        {
            assert!(file.master.is_none());
        }
        assert!(file.cards.is_empty());
    }

    #[test]
    fn duplicate_names_rejected() {
        let res = load_str(
            "[[card]]\nname = \"a\"\ncwd = \"C:/x\"\n\n[[card]]\nname = \"a\"\ncwd = \"C:/y\"\n",
        );
        assert!(matches!(res, Err(CardsError::Invalid(_))));
    }

    #[test]
    fn duplicate_names_case_insensitive_rejected() {
        // kortnavne bliver filnavne (signals\<name>.pause.json); Windows-fs er case-insensitivt
        let res = load_str(
            "[[card]]\nname = \"b\"\ncwd = \"C:/x\"\n\n[[card]]\nname = \"B\"\ncwd = \"C:/y\"\n",
        );
        assert!(matches!(res, Err(CardsError::Invalid(_))));
    }

    // Supervision-only (Task 3): i default-state er "master" et almindeligt
    // tilladt kortnavn (ingen reservation — master-kortet findes ikke).
    #[cfg(feature = "supervision")]
    #[test]
    fn master_name_reserved_for_master_card() {
        let res = load_str("[[card]]\nname = \"master\"\ncwd = \"C:/x\"\n");
        assert!(matches!(res, Err(CardsError::Invalid(_))));
    }

    #[test]
    fn bad_name_chars_rejected() {
        // navnet bruges som filnavn og TALMINAL_SESSION_ID - ingen stier/specialtegn
        let res = load_str("[[card]]\nname = \"../evil\"\ncwd = \"C:/x\"\n");
        assert!(matches!(res, Err(CardsError::Invalid(_))));
    }

    #[test]
    fn empty_command_rejected() {
        let res = load_str("[[card]]\nname = \"a\"\ncwd = \"C:/x\"\ncommand = []\n");
        assert!(matches!(res, Err(CardsError::Invalid(_))));
    }

    #[test]
    fn resolve_base_prefers_talminal_home() {
        let base = resolve_base(
            Some(OsString::from("C:/tmp/pos-test-home")),
            Some(OsString::from("C:/Users/x/AppData/Local")),
        );
        assert_eq!(base, PathBuf::from("C:/tmp/pos-test-home"));
    }

    #[test]
    fn resolve_base_falls_back_to_localappdata() {
        let base = resolve_base(None, Some(OsString::from("C:/Users/x/AppData/Local")));
        assert_eq!(
            base,
            PathBuf::from("C:/Users/x/AppData/Local").join("Talminal")
        );
    }
}
