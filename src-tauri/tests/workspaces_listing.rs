use talminal_canvas_lib::workspaces::listing::{self, unique_suffixes};

fn skriv_projekt(base: &std::path::Path, slug: &str, navn: &str, root: &str, added: Option<&str>) {
    let dir = base.join("projects").join(slug);
    std::fs::create_dir_all(&dir).unwrap();
    let added_felt = added
        .map(|a| format!(r#","added_at":"{a}""#))
        .unwrap_or_default();
    std::fs::write(
        dir.join("project.json"),
        format!(
            r#"{{"root":"{}","name":"{navn}"{added_felt}}}"#,
            root.replace('\\', "\\\\")
        ),
    )
    .unwrap();
}

#[test]
fn korrupt_project_json_vaelter_ikke_listen() {
    let base = tempfile::tempdir().unwrap();
    skriv_projekt(
        base.path(),
        "god-1111",
        "god",
        r"C:\repos\god",
        Some("2026-07-01T00:00:00.000Z"),
    );
    let defekt = base.path().join("projects").join("defekt-2222");
    std::fs::create_dir_all(&defekt).unwrap();
    std::fs::write(defekt.join("project.json"), b"{ikke json").unwrap();

    let liste = listing::list(base.path());
    assert_eq!(
        liste.len(),
        2,
        "den defekte post skal vises, ikke skjules eller fælde listen"
    );
    let d = liste.iter().find(|e| e.slug == "defekt-2222").unwrap();
    assert!(d.defect, "posten skal være markeret defekt");
    assert_eq!(
        d.name, "defekt-2222",
        "slug bruges som navn når filen ikke kan læses"
    );
}

#[test]
fn hidden_er_en_sidecar_og_virker_paa_en_korrupt_post() {
    let base = tempfile::tempdir().unwrap();
    let defekt = base.path().join("projects").join("defekt-2222");
    std::fs::create_dir_all(&defekt).unwrap();
    std::fs::write(defekt.join("project.json"), b"{ikke json").unwrap();

    listing::hide(base.path(), "defekt-2222")
        .expect("skjul må ikke kræve at project.json kan parses");
    assert!(defekt.join(".hidden").exists());
    assert!(
        listing::list(base.path())
            .iter()
            .find(|e| e.slug == "defekt-2222")
            .unwrap()
            .hidden
    );

    listing::unhide(base.path(), "defekt-2222").unwrap();
    assert!(!defekt.join(".hidden").exists());
}

#[test]
fn sortering_er_total_added_at_saa_navn_saa_slug() {
    let base = tempfile::tempdir().unwrap();
    skriv_projekt(
        base.path(),
        "c-3333",
        "c",
        r"C:\r\c",
        Some("2026-07-03T00:00:00.000Z"),
    );
    skriv_projekt(
        base.path(),
        "a-1111",
        "a",
        r"C:\r\a",
        Some("2026-07-01T00:00:00.000Z"),
    );
    // Uden added_at: sorteres efter navn, og lægges sidst
    skriv_projekt(base.path(), "z-9999", "zeta", r"C:\r\z", None);
    skriv_projekt(base.path(), "m-5555", "mu", r"C:\r\m", None);

    let slugs: Vec<_> = listing::list(base.path())
        .into_iter()
        .map(|e| e.slug)
        .collect();
    assert_eq!(slugs, vec!["a-1111", "c-3333", "m-5555", "z-9999"]);
}

#[test]
fn unikke_stisuffikser_gaar_saa_dybt_som_noedvendigt() {
    // Ét niveau er ikke altid nok — begge hedder bridgemind OG ligger i en repos-mappe.
    let roots = vec![
        r"C:\work\alpha\repos\bridgemind".to_string(),
        r"C:\work\beta\repos\bridgemind".to_string(),
        r"C:\work\demo".to_string(),
    ];
    let suffikser = unique_suffixes(&roots);
    assert_eq!(suffikser[0], r"alpha\repos\bridgemind");
    assert_eq!(suffikser[1], r"beta\repos\bridgemind");
    assert_eq!(
        suffikser[2], "demo",
        "en post uden kollision får intet suffiks-tillæg"
    );
}
