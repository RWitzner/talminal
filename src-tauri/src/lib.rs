//! Talminal core library.
//!
//! MVP-moduler (canvas-pivot):
//! - `pty`     PtyHost — spawn/read/write/resize/kill, DSR auto-reply, teardown
//! - `cards`   cards.toml parsing (CardConfig / CardsFile / load_cards)
//! - `control` supervisions-facade: frossen IPC-kontrakt i begge feature-states (Task 1)
//!
//! Task 1 Del B-sockets (doede indtil deres task udfylder dem — se planen):
//! - `profiles`    agent-profiler + env-politik-split (Task 2)
//! - `registry`    muterbart kort-registry: create/close/list (Task 5)
//! - `workspace`   workspace.json-persistens + viewport + settings (Task 6/14)
//! - `restore`     restore-on-launch, en-pr.-cwd-reglen (Task 11)
//! - `secrets`     keyring-secrets, Windows Credential Manager (Task 14)
//! - `submit`      submit-koreografi: tekst og \r som separate writes (Task 16b)
//! - `transcripts` transcript-tail-laesning (Task 17)
//!
//! Supervision (PARKERET bag `--features supervision`, slettes ikke —
//! laast ejer-beslutning; spec v0.6 forbliver sporets sandhedskilde):
//! - `epoch`    EpochGate — per-card epoch gate + owner
//! - `signals`  pause/resume signal files (atomic write)
//! - `presence` fs-poll of presence files -> Tauri events

pub mod atomic;
pub mod browser;
pub mod browser_host;
pub mod cards;
pub mod context_hud;
pub mod control;
pub mod instance;
pub mod mcp;
pub mod perf_trace;
pub mod profiles;
pub mod project;
pub mod prompt_readiness;
pub mod providers;
pub mod pty;
pub mod registry;
pub mod restore;
pub mod secrets;
pub mod submit;
pub mod threads;
pub mod transcripts;
pub mod usage_hud;
pub mod voice_capture;
pub mod wake_hotkey;
pub mod webview_permissions;
pub mod worker_mcp;
pub mod workspace;
pub mod workspaces;

#[cfg(feature = "supervision")]
pub mod epoch;
#[cfg(feature = "supervision")]
pub mod presence;
#[cfg(feature = "supervision")]
pub mod signals;
