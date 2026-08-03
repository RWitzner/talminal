//! Per-worker MCP-config (browser-cards plan Task 6): skriver en
//! `--mcp-config`-fil pr. profil-spawnet CC-worker, saa den faar tools mod
//! talminal-MCP'en (Task 4, session-header-scopet) og sit scopes
//! Playwright-CDP-endpoint (Task 3, pinnet version). Ren fil-IO — ingen
//! tauri-typer; `spawn_into` (main.rs) er den eneste kalder.
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::browser::PLAYWRIGHT_MCP_PIN;
use crate::cards::talminal_base;

/// Claude Code's built-in Chrome integration competes with the Talminal-owned
/// talminal-MCP/Playwright tools for ordinary browser prompts. Keep the user's
/// other configured MCP servers, but disable that one competing browser
/// surface whenever this Talminal config is injected.
pub const DISABLE_CLAUDE_IN_CHROME_ARG: &str = "--no-chrome";

/// Env-var-navnet codex-workerens Bearer-token leveres via (spec §1.2,
/// GPT-review B3/B4): kort-identiteten er per-invocation, ikke en fil —
/// `main.rs` saetter denne env-variabel, mcp.rs slaar tokenet op mod
/// kort-nøglede registryet (kort-nøglet, se `mcp::set_card_token`).
pub const MCP_TOKEN_ENV: &str = "TALMINAL_MCP_TOKEN";

fn worker_mcp_dir() -> PathBuf {
    talminal_base().join("worker-mcp")
}

/// CLI arguments paired with a successfully written Talminal MCP config.
///
/// This intentionally does not use `--strict-mcp-config`: strict mode would
/// also remove unrelated user/project MCP servers. `--no-chrome` is the
/// narrow Claude Code switch for ensuring web tasks stay in Talminal.
pub fn launch_args(config_path: &Path) -> Vec<String> {
    vec![
        "--mcp-config".to_string(),
        config_path.display().to_string(),
        DISABLE_CLAUDE_IN_CHROME_ARG.to_string(),
    ]
}

/// Skriver `{worker-mcp-dir}/{card_name}.json`. Idempotent/overskrivende —
/// et respawn med samme porte OG samme token giver byte-identisk indhold (spec
/// §5-uforanderligheden). Atomisk skrivning via den delte `crate::atomic::write`.
pub fn write_config(
    card_name: &str,
    mcp_port: u16,
    cdp_port: u16,
    token: &str,
) -> Result<PathBuf, String> {
    let config = json!({
        "mcpServers": {
            "talminal": {
                "type": "http",
                "url": format!("http://127.0.0.1:{mcp_port}/mcp"),
                "headers": {
                    // Browser-tools' scope-noegle (uaendret, uverificeret header).
                    "x-talminal-session": card_name,
                    // Beslutning 12: de tre traad-tools er Bearer-only, saa CC
                    // skal baere det samme canvas-udstedte token som codex.
                    "Authorization": format!("Bearer {token}")
                }
            },
            "playwright": {
                "command": "npx",
                "args": [
                    "-y",
                    PLAYWRIGHT_MCP_PIN,
                    "--cdp-endpoint",
                    format!("http://127.0.0.1:{cdp_port}")
                ]
            }
        }
    });
    let path = worker_mcp_dir().join(format!("{card_name}.json"));
    crate::atomic::write_json_pretty(&path, &config)
        .map_err(|e| format!("worker-mcp write failed: {e}"))?;
    Ok(path)
}

/// Fjerner `{worker-mcp-dir}/{card_name}.json`.
///
/// Filen BAERER tokenet (`"Authorization": "Bearer …"`), saa den hoerer sammen
/// med `mcp::clear_card_token`: et ryddet token uden en fjernet fil efterlader
/// en credential-formet fil paa disken, som enhver proces under samme bruger
/// kan laese — inklusive de ANDRE agent-kort, der pr. definition har shell.
/// Maalt foer dette blev skrevet: en `card-1.json` overlevede baade kort-luk
/// OG app-exit i 16 timer.
///
/// Idempotent: en manglende fil er ikke en fejl. Ikke alle kort faar en config
/// (browser-kort skriver ingen, og codex-vejen leverer identiteten via env),
/// og afslutningsvejene kan ramme samme kort mere end én gang.
///
/// **Kaldes kun fra afslutningsveje.** Se `main::release_card_identity`.
pub fn remove_config(card_name: &str) {
    let path = worker_mcp_dir().join(format!("{card_name}.json"));
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!(
            "[canvas] worker-mcp: kunne ikke fjerne {}: {e}",
            path.display()
        ),
    }
}

/// App-start-sweep: hele worker-mcp-mappen ryddes.
///
/// Daekker det `remove_config` paa afslutningsvejene ikke kan naa — et haardt
/// exit, et crash eller et strømsvigt efterlader filer som ingen afslutning
/// koerte for. Uden den ville et enkelt crash goere token-filen permanent.
/// Best-effort, samme kontrakt som `browser::sweep_profiles`: laaste filer
/// efterlades til naeste sweep.
///
/// Sikker fordi configen skrives pr. spawn (`write_config` i `spawn_into`s
/// fase 2), ikke laeses fra en tidligere session — og tokenet i en gammel fil
/// er alligevel doedt, da `mcp`'s registry er in-proces.
pub fn sweep_configs() {
    let _ = std::fs::remove_dir_all(worker_mcp_dir());
}

/// Codex-varianten af MCP-injektionen (spike-vej i, end-to-end-verificeret):
/// per-invocation `-c`-overrides — INGEN config.toml-mutation, INGEN config-fil.
/// Kort-identiteten leveres som Bearer-token via env (bearer_token_env_var).
pub fn codex_launch_args(mcp_port: u16, cdp_port: u16) -> Vec<String> {
    let playwright_args = format!(
        r#"mcp_servers.playwright.args=["-y","{PLAYWRIGHT_MCP_PIN}","--cdp-endpoint","http://127.0.0.1:{cdp_port}"]"#
    );
    vec![
        "-c".into(),
        format!(r#"mcp_servers.talminal.url="http://127.0.0.1:{mcp_port}/mcp""#),
        "-c".into(),
        format!(r#"mcp_servers.talminal.bearer_token_env_var="{MCP_TOKEN_ENV}""#),
        "-c".into(),
        r#"mcp_servers.playwright.command="npx""#.into(),
        "-c".into(),
        playwright_args,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_launch_args_are_per_invocation_config_overrides() {
        let args = codex_launch_args(50001, 50002);
        // Spike §5.2: -c muterer IKKE config.toml; value-delen er TOML.
        assert_eq!(args[0], "-c");
        assert_eq!(
            args[1],
            r#"mcp_servers.talminal.url="http://127.0.0.1:50001/mcp""#
        );
        assert_eq!(
            args[3],
            format!(r#"mcp_servers.talminal.bearer_token_env_var="{MCP_TOKEN_ENV}""#)
        );
        let joined = args.join(" ");
        assert!(joined.contains(r#"mcp_servers.playwright.command="npx""#));
        assert!(joined.contains("--cdp-endpoint"));
        assert!(joined.contains("http://127.0.0.1:50002"));
        assert!(
            args.iter().all(|a| a == "-c" || a.contains('=')),
            "kun -c key=value-par"
        );
    }
}
