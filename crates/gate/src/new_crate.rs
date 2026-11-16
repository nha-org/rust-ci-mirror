//! `dash crate new <name> --kind plugin|layer|ring` (task rust-task-names-crate; @nha, 2026-11-14: « sounds good, but maybe
//! hint at this ? how do we create new crates ? »): a crate was four hand steps. It writes the crate with its `tool` and `ui`
//! modules (ARCHITECTURE.md section 2 point 5), its entry in this crate's list for its kind (a planned plugin leaving
//! [`crate::PLANNED_PLUGINS`]), the workspace's dependency line, the binary's dependency and link, and section 3's row of
//! rfc-rust-unified when the RFC sits beside rust/. Then rustfmt lays out what it edited and cargo writes Cargo.lock.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const USAGE: &str = "dash crate new <name> --kind plugin|layer|ring  a crate with its tool and ui modules, its dash-gate \
                         list entry and its link in the binary, in this checkout";

dash_core::submit! {
    dash_core::Command { usage: USAGE, words: &["crate"], run: Some(|_, args, out| command(args, out)) }
}

/// `dash crate new` from the checkout holding the current folder: each file it wrote, or why nothing was.
pub fn command(args: &[&str], out: &mut dyn Write) {
    let said = match args {
        ["new", name, "--kind", kind] => {
            checkout().and_then(|rust| write(&rust, name, kind).map(|w| (rust, w)))
        }
        _ => Err(format!("usage: {USAGE}")),
    };
    // each file written is said before settling, so a rustfmt or cargo that fails leaves them named (/code-review)
    let settled = said.and_then(|(rust, files)| {
        files.iter().for_each(|f| _ = writeln!(out, "wrote {f}"));
        settle(&rust, name_of(args))
    });
    match settled {
        Ok(()) => {}
        Err(why) => {
            _ = writeln!(out, "{why}");
            std::process::exit(1);
        }
    }
}

/// The crate's name in `dash crate new <name> ..`.
fn name_of<'a>(args: &[&'a str]) -> &'a str {
    args.get(1).copied().unwrap_or_default()
}

/// The rust/ folder at or above the current one: site_perso's rust/, or the mirror's root.
fn checkout() -> Result<PathBuf, String> {
    let here = std::env::current_dir().map_err(|e| format!("this folder: {e}"))?;
    let gate = |d: &Path| d.join("crates/gate/src/lib.rs").is_file();
    (here.ancestors())
        .flat_map(|d| [d.to_path_buf(), d.join("rust")])
        .find(|d| gate(d))
        .ok_or_else(|| format!("{}: in no checkout of the Rust workspace", here.display()))
}

/// The list a kind's crate joins.
fn list(kind: &str) -> Result<&'static str, String> {
    match kind {
        "plugin" => Ok("PLUGINS"),
        "layer" => Ok("TASK_LAYERS"),
        "ring" => Ok("RINGS"),
        _ => Err(format!("--kind {kind}: plugin, layer or ring")),
    }
}

/// Everything but formatting under `rust`: each path written, from rust/, or why nothing was (all checked before any write).
pub fn write(rust: &Path, name: &str, kind: &str) -> Result<Vec<String>, String> {
    let list = list(kind)?;
    let shaped = name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !shaped || name.ends_with('-') {
        return Err(format!(
            "{name}: a crate name is lowercase letters, digits and inner dashes"
        ));
    }
    let taken = crate::CRATE_KINDS
        .iter()
        .any(|(_, names)| names.contains(&name));
    if taken || rust.join("crates").join(name).exists() {
        return Err(format!("{name}: a crate of that name exists"));
    }
    // a planned plugin is a plugin: in another list it would sit in two (/code-review)
    if kind != "plugin" && crate::PLANNED_PLUGINS.contains(&name) {
        return Err(format!(
            "{name}: a planned plugin crate, made with --kind plugin"
        ));
    }
    let read = |p: &str| std::fs::read_to_string(rust.join(p)).map_err(|e| format!("{p}: {e}"));
    let gate = "crates/gate/src/lib.rs";
    let mut lists = listed(&read(gate)?, list, name, true)?;
    if kind == "plugin" && crate::PLANNED_PLUGINS.contains(&name) {
        lists = listed(&lists, "PLANNED_PLUGINS", name, false)?;
    }
    let dep = format!("dash-{name}");
    let workspace = after_line(
        &read("Cargo.toml")?,
        "dash-core = ",
        &format!("{dep} = {{ path = \"crates/{name}\" }}"),
    )?;
    let binary = after_line(
        &read("crates/dash/Cargo.toml")?,
        "dash-core.workspace",
        &format!("{dep}.workspace = true"),
    )?;
    let link = format!("use dash_{} as _;", name.replace('-', "_"));
    let main = after_line(
        &read("crates/dash/src/main.rs")?,
        "use dash_pdf as _;",
        &link,
    )?;
    let rfc_path = rust.join("../private/docs/rfc-rust-unified.html");
    let rfc = match std::fs::read_to_string(&rfc_path) {
        Ok(html) => Some(rfc_row(&html, name, kind)?),
        Err(_) => None,
    };
    let what = match kind {
        "plugin" => "a plugin: a feature on the core's ports alone",
        "layer" => "a task layer: one core port's adapter",
        _ => "a ring of section 3",
    };
    let manifest = format!(
        "[package]\nname = \"{dep}\"\nversion.workspace = true\nedition.workspace = true\npublish.workspace = true\n\n\
         [dependencies]\ndash-core.workspace = true\n\n[dev-dependencies]\ndash-meter.workspace = true\n"
    );
    let lib = format!(
        "//! {dep}, {what}, made by `dash crate new`. Its tool and its UI components each depend on the core SDK alone; the \
         rest\n//! depends on those two and the core (rust/ARCHITECTURE.md section 2 point 5).\npub mod tool;\npub mod ui;\n"
    );
    let mut files = vec![
        (format!("crates/{name}/Cargo.toml"), manifest),
        (format!("crates/{name}/src/lib.rs"), lib),
        (
            format!("crates/{name}/src/tool.rs"),
            format!("//! {dep}'s tool: the API it wraps, on the core SDK alone.\n"),
        ),
        (
            format!("crates/{name}/src/ui.rs"),
            format!("//! {dep}'s UI components, on the core SDK alone.\n"),
        ),
        (gate.into(), lists),
        ("Cargo.toml".into(), workspace),
        ("crates/dash/Cargo.toml".into(), binary),
        ("crates/dash/src/main.rs".into(), main),
    ];
    if let Some(html) = rfc {
        files.push(("../private/docs/rfc-rust-unified.html".into(), html));
    }
    std::fs::create_dir_all(rust.join("crates").join(name).join("src"))
        .map_err(|e| format!("{name}: {e}"))?;
    for (path, text) in &files {
        std::fs::write(rust.join(path), text).map_err(|e| format!("{path}: {e}"))?;
    }
    Ok(files.into_iter().map(|(p, _)| p).collect())
}

/// rustfmt over what was edited, so the lists and the link lie as rustfmt_check wants, then Cargo.lock taking the crate.
fn settle(rust: &Path, name: &str) -> Result<(), String> {
    let run = |cmd: &mut Command| {
        let out = cmd
            .current_dir(rust)
            .output()
            .map_err(|e| format!("{cmd:?}: {e}"))?;
        match out.status.success() {
            true => Ok(()),
            false => Err(format!(
                "{cmd:?}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )),
        }
    };
    let lib = format!("crates/{name}/src/lib.rs");
    let files = ["crates/gate/src/lib.rs", "crates/dash/src/main.rs", &lib];
    run(Command::new("rustfmt")
        .args(["--edition", "2024"])
        .args(files))?;
    run(Command::new("cargo").args(["metadata", "--format-version", "1", "--offline"]))
}

/// `text` with `const <list>: [&str; N] = [..];` holding `name` (`add`) or no longer holding it, N counted again; rustfmt lays
/// it out after.
pub fn listed(text: &str, list: &str, name: &str, add: bool) -> Result<String, String> {
    let head = format!("pub const {list}: [&str; ");
    let start = text.find(&head).ok_or(format!("dash-gate has no {list}"))?;
    let eq = start
        + text[start..]
            .find('=')
            .ok_or(format!("{list}: no literal"))?;
    let body = eq + text[eq..].find('[').ok_or(format!("{list}: no literal"))? + 1;
    let end = body + text[body..].find("];").ok_or(format!("{list}: no end"))?;
    let mut names: Vec<&str> = (text[body..end].split(','))
        .map(|n| n.trim().trim_matches('"'))
        .filter(|n| !n.is_empty())
        .collect();
    // the file's own list, which a binary built before it may not know (/code-review)
    if add && names.contains(&name) {
        return Err(format!("{name}: a crate of that name exists"));
    }
    match add {
        true => names.push(name),
        false => names.retain(|n| *n != name),
    }
    let quoted: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
    let (before, after) = (&text[..start], &text[end + 2..]);
    Ok(format!(
        "{before}{head}{}] = [{}];{after}",
        names.len(),
        quoted.join(", ")
    ))
}

/// `text` with `line` after its first line opening on `prefix`, blanks before it ignored.
fn after_line(text: &str, prefix: &str, line: &str) -> Result<String, String> {
    let at = (text.match_indices('\n'))
        .map(|(i, _)| i + 1)
        .chain([0])
        .filter(|&i| text[i..].trim_start().starts_with(prefix))
        .min()
        .ok_or(format!("no line opening on {prefix}"))?;
    let eol = at + text[at..].find('\n').map_or(text.len() - at, |n| n + 1);
    Ok(format!("{}{line}\n{}", &text[..eol], &text[eol..]))
}

/// rfc-rust-unified with the crate in section 3's table, as dash-gate's `sections_3_and_4_are_the_gate_and_the_meter` reads
/// it, its own markup reused so no tag is written here (the_dash_crate_holds_no_html: HTML is the design system's): a plugin
/// ends the plugin row's names, which that test holds to [`crate::PLUGINS`]; a ring is a copy of the last ring's row, its
/// name and kind set and its two prose cells left for the task to write. A task layer's row, which no test reads, is the
/// task's.
fn rfc_row(html: &str, name: &str, kind: &str) -> Result<String, String> {
    match kind {
        "plugin" => {
            let row = crate::PLUGINS.join(", ");
            let at = html
                .find(&row)
                .ok_or("rfc-rust-unified: section 3's plugin row is not dash-gate's PLUGINS")?;
            // the cell's end, past a plugin this binary was built before
            let end = at + html[at..].find('<').unwrap_or(html.len() - at);
            Ok(format!("{}, {name}{}", &html[..end], &html[end..]))
        }
        "ring" => {
            let last = crate::RINGS.last().copied().unwrap_or_default();
            let cell = format!(">{last}<");
            // section 3's: the last row naming it before the plugin row, which that test holds to PLUGINS (/code-review)
            let table = html.find(&crate::PLUGINS.join(", ")).unwrap_or(html.len());
            let line = (html[..table].lines().rfind(|l| l.contains(&cell)))
                .map(str::trim_start)
                .ok_or(format!("rfc-rust-unified: no row of {last} in section 3"))?;
            // its cells in order, each the text after an opening `td>`
            let mut values = [name, "Adapter", "", ""].into_iter();
            // a link inside a cell goes with its text
            let row = ["", "tr>", "/td>", "/tr>"];
            let pieces: Vec<String> = (line.split('<'))
                .filter_map(|p| match p.strip_prefix("td>") {
                    Some(_) => Some(format!("td>{}", values.next().unwrap_or_default())),
                    None => row.contains(&p.trim_end()).then(|| p.to_string()),
                })
                .collect();
            after_line(html, line, &pieces.join("<"))
        }
        _ => Ok(html.to_string()),
    }
}
