//! `dash crate new`'s writes, on a copy of the workspace's own files (task rust-task-names-crate).
use dash_gate::new_crate::write;
use dash_meter::{TestDir, dash_test};
use std::path::Path;

/// Done-when 3 of rust-task-names-crate, its writes: on a copy of the workspace's own files, a planned plugin made with
/// `dash crate new` (the first of PLANNED_PLUGINS) gets its crate, tool and ui modules, joins PLUGINS (counted) and leaves PLANNED_PLUGINS, and gets
/// its workspace line, the binary's dependency and link, and its name in section 3's plugin row; a ring gets a row of
/// its own. A taken name, a bad one and an unknown kind write nothing.
#[dash_test]
fn a_new_crate_is_written_with_its_list_entry_and_link() {
    let real = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = TestDir::new("new-crate");
    let rust = dir.0.join("rust");
    for p in [
        "Cargo.toml",
        "crates/dash/Cargo.toml",
        "crates/dash/src/main.rs",
        "crates/gate/src/lib.rs",
    ] {
        std::fs::create_dir_all(rust.join(p).parent().expect("a parent")).expect("folders");
        std::fs::copy(real.join(p), rust.join(p)).expect("copied");
    }
    // the lists as this checkout holds them: its first planned plugin, the one after it, the last plugin and ring
    // ponytail: needs two planned plugins; give the test its own lists once the moves leave fewer
    let (planned, next) = (dash_gate::PLANNED_PLUGINS[0], dash_gate::PLANNED_PLUGINS[1]);
    let (plugins, last_ring) = (
        dash_gate::PLUGINS.join(", "),
        dash_gate::RINGS[dash_gate::RINGS.len() - 1],
    );
    let rfc = format!(
        "<h2>3. Architecture</h2>\n<tr><td>core</td><td>Core</td><td>a</td><td>b</td></tr>\n\
         <tr><td>{last_ring}</td><td>Extension</td><td>Its loader (<a href=\"#x\">section 12</a>)</td><td>b</td></tr>\n\
         <tr><td>{plugins}</td><td>Plugin</td></tr>\n"
    );
    std::fs::create_dir_all(dir.0.join("private/docs")).expect("docs");
    std::fs::write(dir.0.join("private/docs/rfc-rust-unified.html"), rfc).expect("rfc");
    let read = |p: &str| std::fs::read_to_string(rust.join(p)).expect(p);

    let wrote = write(&rust, planned, "plugin").expect("written");
    assert!(
        wrote.contains(&format!("crates/{planned}/src/tool.rs")),
        "{wrote:?}"
    );
    let lib = read(&format!("crates/{planned}/src/lib.rs"));
    assert!(lib.contains("pub mod tool;\npub mod ui;\n"), "{lib}");
    let manifest = read(&format!("crates/{planned}/Cargo.toml"));
    assert!(
        manifest.contains(&format!("name = \"dash-{planned}\"\n")),
        "{manifest}"
    );
    assert!(
        manifest.contains("[dependencies]\ndash-core.workspace = true\n"),
        "{manifest}"
    );
    let ui = read(&format!("crates/{planned}/src/ui.rs"));
    assert!(
        ui.starts_with(&format!("//! dash-{planned}'s UI components")),
        "{ui}"
    );
    let gate = read("crates/gate/src/lib.rs");
    let counted = format!(
        "pub const PLUGINS: [&str; {}] = [",
        dash_gate::PLUGINS.len() + 1
    );
    assert!(gate.contains(&counted), "PLUGINS counted again");
    let last = dash_gate::PLUGINS[dash_gate::PLUGINS.len() - 1];
    assert!(
        gate.contains(&format!("\"{last}\", \"{planned}\"];")),
        "{planned} ends PLUGINS"
    );
    let left = dash_gate::PLANNED_PLUGINS.len() - 1;
    let left = format!("pub const PLANNED_PLUGINS: [&str; {left}] = [\"{next}\"");
    assert!(gate.contains(&left), "{planned} left PLANNED_PLUGINS");
    let workspace = format!(
        "dash-core = {{ path = \"crates/core\" }}\ndash-{planned} = {{ path = \"crates/{planned}\" }}\n"
    );
    assert!(read("Cargo.toml").contains(&workspace));
    let binary = format!("dash-core.workspace = true\ndash-{planned}.workspace = true\n");
    assert!(read("crates/dash/Cargo.toml").contains(&binary));
    let link = format!("use dash_pdf as _;\nuse dash_{planned} as _;\n");
    assert!(read("crates/dash/src/main.rs").contains(&link));
    let html =
        || std::fs::read_to_string(dir.0.join("private/docs/rfc-rust-unified.html")).expect("rfc");
    let row = format!("<tr><td>{plugins}, {planned}</td><td>Plugin</td></tr>");
    assert!(html().contains(&row), "{}", html());

    write(&rust, "fan-out", "ring").expect("a ring");
    assert!(read("crates/gate/src/lib.rs").contains(&format!("\"{last_ring}\", \"fan-out\"];")));
    assert!(read("crates/dash/src/main.rs").contains("use dash_fan_out as _;\n"));
    let row = "</td><td>b</td></tr>\n<tr><td>fan-out</td><td>Adapter</td><td></td><td></td></tr>\n<tr><td>";
    assert!(html().contains(row), "{}", html());

    let gate = read("crates/gate/src/lib.rs");
    let solo = dash_gate::new_crate::listed(&gate, "TASK_LAYERS", "solo", true).expect("listed");
    std::fs::write(rust.join("crates/gate/src/lib.rs"), solo).expect("written");
    let before = read("crates/gate/src/lib.rs");
    for (name, kind, why) in [
        (
            planned,
            "plugin",
            format!("{planned}: a crate of that name exists").as_str(),
        ),
        ("mail", "layer", "mail: a crate of that name exists"),
        (
            "Bad",
            "plugin",
            "Bad: a crate name is lowercase letters, digits and inner dashes",
        ),
        (
            "bad-",
            "plugin",
            "bad-: a crate name is lowercase letters, digits and inner dashes",
        ),
        ("ok", "pane", "--kind pane: plugin, layer or ring"),
        (
            next,
            "ring",
            format!("{next}: a planned plugin crate, made with --kind plugin").as_str(),
        ),
        // in the file's list though not in the binary's, and no folder yet (/code-review)
        ("solo", "layer", "solo: a crate of that name exists"),
    ] {
        assert_eq!(write(&rust, name, kind), Err(why.to_string()));
    }
    assert_eq!(
        read("crates/gate/src/lib.rs"),
        before,
        "a refusal writes nothing"
    );
    assert!(!rust.join("crates/ok").exists());
}
