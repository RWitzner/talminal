mod common;

use talminal_canvas_lib::worker_mcp;

#[test]
fn config_carries_session_header_and_pinned_playwright() {
    // Serialiseret sammen med token-testene: de flytter TALMINAL_HOME proces-
    // bredt og sletter mappen igen naar deres TempDir droppes. Uden guarden kan
    // filen forsvinde mellem skrivningen og laesningen herunder.
    let _g = common::serial();
    let path = worker_mcp::write_config("card-777", 4111, 4222, "tok-777").expect("write");
    let text = std::fs::read_to_string(&path).expect("read");
    let json: serde_json::Value = serde_json::from_str(&text).expect("json");
    let servers = &json["mcpServers"];
    assert_eq!(servers["talminal"]["type"], "http");
    assert_eq!(servers["talminal"]["url"], "http://127.0.0.1:4111/mcp");
    assert_eq!(
        servers["talminal"]["headers"]["x-talminal-session"],
        "card-777"
    );
    assert_eq!(servers["playwright"]["command"], "npx");
    let args: Vec<String> = servers["playwright"]["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        args,
        vec![
            "-y",
            "@playwright/mcp@0.0.78",
            "--cdp-endpoint",
            "http://127.0.0.1:4222"
        ]
    );

    // Talminal browser-tools must win over a globally/default-enabled Claude in
    // Chrome integration, without strict mode hiding unrelated user MCPs.
    let launch_args = worker_mcp::launch_args(&path);
    assert_eq!(
        launch_args,
        vec![
            "--mcp-config",
            path.to_str().expect("utf-8 test path"),
            "--no-chrome",
        ]
    );
    assert!(!launch_args.iter().any(|arg| arg == "--strict-mcp-config"));
    std::fs::remove_file(path).ok();
}

#[test]
fn cc_config_carries_both_the_session_header_and_a_bearer_token() {
    let _g = common::serial();
    let _home = common::temp_home();
    let path = worker_mcp::write_config("card-4", 50001, 50002, "tok-abc").expect("write_config");
    let body = std::fs::read_to_string(path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let headers = &v["mcpServers"]["talminal"]["headers"];
    // Browser-tools afhaenger af session-headeren; den maa IKKE forsvinde.
    assert_eq!(headers["x-talminal-session"], "card-4");
    // De tre traad-tools er Bearer-only (beslutning 12).
    assert_eq!(headers["Authorization"], "Bearer tok-abc");
    assert_eq!(
        v["mcpServers"]["talminal"]["url"],
        "http://127.0.0.1:50001/mcp"
    );
}

#[test]
fn a_respawn_with_the_same_token_is_byte_identical() {
    let _g = common::serial();
    let _home = common::temp_home();
    let first = worker_mcp::write_config("card-5", 1, 2, "tok").unwrap();
    let a = std::fs::read_to_string(&first).unwrap();
    let second = worker_mcp::write_config("card-5", 1, 2, "tok").unwrap();
    let b = std::fs::read_to_string(&second).unwrap();
    assert_eq!(a, b, "spec §5-uforanderligheden");
}
