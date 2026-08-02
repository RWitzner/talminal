//! D0-spike (codex-adapter, spec §5.2c): standalone MCP-server-probe.
//! Starter mcp.rs' rigtige server med stub-ops og venter, så en ekstern
//! MCP-klient (codex) kan handshake mod den ægte dialekt. Throwaway dev-bin —
//! ingen produktion afhænger af den.

use std::sync::Arc;

use talminal_canvas_lib::mcp::{
    start_mcp_server, BrowserCardOps, BrowserCardRow, McpOps, OpenResult,
};
use talminal_canvas_lib::threads::ops::LiveThreadOps;

struct StubOps;

impl BrowserCardOps for StubOps {
    fn open(&self, session: Option<&str>, url: Option<&str>) -> Result<OpenResult, String> {
        eprintln!("PROBE open session={session:?} url={url:?}");
        Ok(OpenResult {
            card_number: 42,
            target_id: "probe-target".into(),
            cdp_endpoint: "http://127.0.0.1:0/probe".into(),
        })
    }

    fn close(&self, session: Option<&str>, card_number: u32) -> Result<(), String> {
        eprintln!("PROBE close session={session:?} card={card_number}");
        Ok(())
    }

    fn list(&self) -> Result<Vec<BrowserCardRow>, String> {
        eprintln!("PROBE list");
        Ok(vec![BrowserCardRow {
            number: 42,
            opened_by: Some("probe".into()),
            url: "https://example.com".into(),
            title: "PROBE-SENTINEL-TITLE".into(),
            target_id: "probe-target".into(),
        }])
    }

    fn focus(&self, card_number: u32) -> Result<(), String> {
        eprintln!("PROBE focus card={card_number}");
        Ok(())
    }
}

fn main() {
    let port = start_mcp_server(McpOps {
        browser: Arc::new(StubOps),
        threads: Arc::new(LiveThreadOps),
    })
    .expect("mcp probe start");
    println!("MCP_PROBE_PORT={port}");
    std::thread::sleep(std::time::Duration::from_secs(120));
}
