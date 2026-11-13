//! `dash ci affected`'s crates (task rust-affected-tests): a scratch repository holding a stub workspace, read by the real git
//! and cargo metadata, as site_perso holds it (in rust/) and as the mirror does (at its top, with no shared history).
use dash_gate::affected::{cargo_args, not_as_pushed, of_repo};
use dash_meter::TestDir;
use dash_meter::dash_test;
use std::path::Path;
use std::process::Command;

/// ocr <- ingest <- dash -> mail: the shape of the real workspace, where dash depends on every crate.
const CRATES: [(&str, &str, &str); 5] = [
    ("ocr", "dash-ocr", ""),
    ("ingest", "dash-ingest", "dash-ocr = { path = \"../ocr\" }"),
    ("mail", "dash-mail", ""),
    (
        "dash",
        "dash",
        "dash-ingest = { path = \"../ingest\" }\ndash-mail = { path = \"../mail\" }",
    ),
    ("gate", "dash-gate", ""),
];

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().expect("folder")).expect("mkdir");
    std::fs::write(path, text).expect("write");
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(root)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repository at `root` holding the stub workspace in `folder`, committed.
fn repo(root: &Path, folder: &str) {
    // a hook running this test (the pre-commit's rust-gate) sets GIT_DIR, which would aim every git here at the real repository
    for (key, _) in std::env::vars_os().filter(|(k, _)| k.to_string_lossy().starts_with("GIT_")) {
        // SAFETY: nextest runs each test in its own process, this thread its only one
        unsafe { std::env::remove_var(key) };
    }
    let at = |rel: &str| {
        if folder.is_empty() {
            rel.to_string()
        } else {
            format!("{folder}/{rel}")
        }
    };
    write(
        root,
        &at("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"3\"\n",
    );
    for (dir, name, deps) in CRATES {
        write(
            root,
            &at(&format!("crates/{dir}/Cargo.toml")),
            &format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n{deps}\n"
            ),
        );
        write(root, &at(&format!("crates/{dir}/src/lib.rs")), "");
    }
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "base"]);
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

#[dash_test]
fn a_change_to_ocr_affects_its_dependents_and_never_mail() {
    let dir = TestDir::new("affected-site");
    repo(&dir.0, "rust");
    git(&dir.0, &["checkout", "-q", "-b", "task"]);
    write(&dir.0, "rust/crates/ocr/src/lib.rs", "pub fn read() {}\n");
    git(&dir.0, &["commit", "-qam", "ocr"]);
    let crates = of_repo(&dir.0, "main", &cargo()).expect("affected");
    assert_eq!(crates, ["dash", "dash-gate", "dash-ingest", "dash-ocr"]);
    // the run the CI and the pre-push make: no mail test among them
    let args = cargo_args(
        &crates,
        &["nextest".into(), "run".into(), "--locked".into()],
    );
    assert_eq!(
        args.join(" "),
        "nextest run --locked -p dash -p dash-gate -p dash-ingest -p dash-ocr"
    );
    assert!(!args.contains(&"dash-mail".to_string()), "Found: {args:?}");
    // clippy's own flags stay after its `--`
    let lint = cargo_args(
        &crates[..1],
        &["clippy".into(), "--".into(), "-D".into(), "warnings".into()],
    );
    assert_eq!(lint.join(" "), "clippy -p dash -- -D warnings");
}

#[dash_test]
fn a_file_outside_the_workspace_affects_none_and_one_outside_any_crate_all() {
    let dir = TestDir::new("affected-edges");
    repo(&dir.0, "rust");
    // uncommitted counts too: the pre-push tests the working tree
    write(&dir.0, "site/index.html", "hi");
    git(&dir.0, &["add", "."]);
    assert_eq!(
        of_repo(&dir.0, "HEAD", &cargo()).expect("affected"),
        Vec::<String>::new(),
        "a file outside rust/ is never on the mirror: no crate tests it"
    );
    write(
        &dir.0,
        "rust/Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"3\"\n# moved\n",
    );
    assert_eq!(
        of_repo(&dir.0, "HEAD", &cargo()).expect("affected"),
        ["dash", "dash-gate", "dash-ingest", "dash-mail", "dash-ocr"]
    );
}

#[dash_test]
fn the_mirror_compares_with_its_main_though_they_share_no_history() {
    let dir = TestDir::new("affected-mirror");
    repo(&dir.0, "");
    // the mirror's branch commit has no parent, as Dashboard.CiMirror builds it
    git(&dir.0, &["checkout", "-q", "--orphan", "task"]);
    write(&dir.0, "crates/mail/src/lib.rs", "pub fn send() {}\n");
    git(&dir.0, &["commit", "-qam", "mail"]);
    assert_eq!(
        of_repo(&dir.0, "main", &cargo()).expect("affected"),
        ["dash", "dash-gate", "dash-mail"]
    );
}

#[dash_test]
fn the_pre_push_tests_only_a_checkout_that_is_the_pushed_commit() {
    let dir = TestDir::new("affected-pushed");
    repo(&dir.0, "rust");
    let first = git(&dir.0, &["rev-parse", "HEAD"]);
    assert_eq!(
        not_as_pushed(&dir.0, &first).expect("pushed"),
        None,
        "HEAD as pushed, nothing changed"
    );
    // an edit not committed would be tested in place of what leaves
    write(&dir.0, "rust/crates/mail/src/lib.rs", "pub fn send() {}\n");
    let said = not_as_pushed(&dir.0, &first).expect("pushed");
    assert!(
        said.as_deref()
            .is_some_and(|s| s.contains("rust/ has changes")),
        "Found: {said:?}"
    );
    // a change outside the workspace is not what the crates test
    git(&dir.0, &["checkout", "--", "."]);
    write(&dir.0, "site/index.html", "hi");
    assert_eq!(
        not_as_pushed(&dir.0, &first).expect("pushed"),
        None,
        "a file outside rust/ holds no test"
    );
    // another ref pushed than the one checked out
    git(&dir.0, &["add", "."]);
    git(&dir.0, &["commit", "-qm", "site"]);
    let said = not_as_pushed(&dir.0, &first).expect("pushed");
    assert!(
        said.as_deref().is_some_and(|s| s.contains("not ")),
        "Found: {said:?}"
    );
}
