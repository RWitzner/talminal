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

// ---------- gate 8: configfilen overlever ikke kortet ----------

/// Filen BAERER tokenet, saa den hoerer sammen med `mcp::clear_card_token`.
/// Foer gate 8 fandtes der ingen sletning overhovedet: en `card-1.json` med et
/// gyldigt Bearer overlevede baade kort-luk OG app-exit, og laa 16 timer paa
/// disken hvor ethvert andet agent-kort — som pr. definition har shell —
/// kunne laese den.
///
/// Testen maaler EFFEKTEN paa disken, ikke at funktionen blev kaldt. Det er
/// projektets egen laere fra T2 i fase 2: en test der asserterer paa et kald
/// bestaar ogsaa naar kaldet ikke virker.
#[test]
fn remove_config_deletes_the_file_from_disk() {
    let _g = common::serial();
    let _home = common::temp_home();
    let path = worker_mcp::write_config("card-gate8", 4311, 4322, "tok-gate8").expect("write");
    assert!(path.is_file(), "forudsaetning: configen skal vaere skrevet");
    // Bekraeft at det faktisk ER en credential-baerende fil — ellers maaler
    // testen paa noget harmloest og beviser ikke gatens praemis.
    let text = std::fs::read_to_string(&path).expect("read");
    assert!(
        text.contains("Bearer tok-gate8"),
        "forudsaetning: filen skal baere tokenet, ellers er der intet at rydde"
    );

    worker_mcp::remove_config("card-gate8");

    assert!(
        !path.exists(),
        "configfilen skal vaere vaek fra disken efter en afslutningsvej: {}",
        path.display()
    );
}

/// Idempotens er ikke pedanteri her: ikke alle kort faar en config (browser-
/// kort skriver ingen, og codex-vejen leverer identiteten via env), og de seks
/// afslutningsveje kan ramme samme kort mere end én gang. Ville et manglende
/// fjern-kald panikke eller stoeje, ville oprydningen blive fjernet igen.
#[test]
fn remove_config_is_idempotent_and_silent_on_a_missing_file() {
    let _g = common::serial();
    let _home = common::temp_home();
    worker_mcp::remove_config("card-har-aldrig-eksisteret");
    let path = worker_mcp::write_config("card-twice", 4411, 4422, "tok-twice").expect("write");
    worker_mcp::remove_config("card-twice");
    worker_mcp::remove_config("card-twice");
    assert!(!path.exists());
}

/// Opstarts-sweepet daekker det afslutningsvejene ikke kan naa: et haardt
/// exit, et crash eller et stroemsvigt efterlader filer som ingen afslutning
/// koerte for. Uden det ville ét crash goere token-filen permanent.
#[test]
fn sweep_configs_clears_files_left_by_a_hard_exit() {
    let _g = common::serial();
    let _home = common::temp_home();
    let a = worker_mcp::write_config("card-crash-1", 4511, 4522, "tok-a").expect("write a");
    let b = worker_mcp::write_config("card-crash-2", 4611, 4622, "tok-b").expect("write b");
    assert!(a.is_file() && b.is_file());

    // Ingen afslutningsvej koerte — det er praecis et crash.
    worker_mcp::sweep_configs();

    assert!(
        !a.exists(),
        "sweep skal rydde efterladte configs: {}",
        a.display()
    );
    assert!(
        !b.exists(),
        "sweep skal rydde efterladte configs: {}",
        b.display()
    );
    // Og en efterfoelgende skrivning skal stadig virke (mappen genskabes).
    let c = worker_mcp::write_config("card-after", 4711, 4722, "tok-c").expect("write efter sweep");
    assert!(
        c.is_file(),
        "write_config skal genskabe mappen efter et sweep"
    );
}
