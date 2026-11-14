use super::*;
use dash_meter::{TestDir, dash_test};

/// Done-when 1: a fresh binary passes when its first run costs under 0.1 s more than its second, stand-in timings.
#[dash_test]
fn a_first_run_passes_under_a_tenth_of_a_second_over_the_second() {
    let ms = Duration::from_millis;
    assert_eq!(first_run(ms(40), ms(5)), None, "35 ms of scan passes");
    assert_eq!(first_run(ms(5), ms(9)), None, "a faster first run passes");
    let slow = first_run(ms(320), ms(20)).expect("300 ms of scan is left");
    assert!(slow.contains("costs 300 ms more"), "{slow}");
    assert!(
        first_run(ms(105), ms(5)).is_some(),
        "exactly 0.1 s over is left"
    );
}

/// Done-when 3: an app granted after it started is named « relaunch <app> », one not granted names the Developer Tools
/// step, one granted before it started is fine, and an unread TCC.db says make setup reads it.
#[dash_test]
fn each_app_is_told_relaunch_or_the_developer_tools_step() {
    let ghostty = App {
        name: "Ghostty".into(),
        client: "com.mitchellh.ghostty".into(),
        started: 1_000,
    };
    let grants = Ok(parse_grants(
        "com.mitchellh.ghostty\t2000\ncom.apple.Terminal\t500\nbroken line\n",
    ));
    assert_eq!(
        grant_step(&ghostty, &grants, false).as_deref(),
        Some("relaunch Ghostty: it started before its Developer Tools grant")
    );
    let terminal = App {
        name: "Terminal".into(),
        client: "com.apple.Terminal".into(),
        started: 1_000,
    };
    assert_eq!(
        grant_step(&terminal, &grants, false),
        None,
        "granted at 500, started at 1000"
    );
    let zed = App {
        name: "Zed".into(),
        client: "dev.zed.Zed".into(),
        started: 1_000,
    };
    assert_eq!(
        grant_step(&zed, &grants, false).as_deref(),
        Some("System Settings > Privacy & Security > Developer Tools: add and turn on Zed")
    );
    let plain =
        grant_step(&zed, &Err(unread("authorization denied", false)), false).expect("a step");
    assert!(plain.contains("make setup reads it by sudo"), "{plain}");
    // make setup's own read failing never sends him back to make setup alone: root needs Full Disk Access too
    let by_sudo =
        grant_step(&zed, &Err(unread("authorization denied", true)), false).expect("a step");
    assert!(
        by_sudo.contains("Full Disk Access, then make setup"),
        "{by_sudo}"
    );
    // the probe ran under Ghostty and passed: a row rewritten after Ghostty started asks no relaunch
    assert_eq!(grant_step(&ghostty, &grants, true), None);
}

/// Done-when 2's naming half: the app is the innermost `.app` its executable sits in.
#[dash_test]
fn the_app_is_the_bundle_its_executable_sits_in() {
    let exe = Path::new("/Applications/Ghostty.app/Contents/MacOS/ghostty");
    assert_eq!(bundle(exe), Some(Path::new("/Applications/Ghostty.app")));
    assert_eq!(bundle(Path::new("/opt/homebrew/bin/herdr")), None);
    // this process's responsible app is found, and started before now
    let me = app(std::process::id() as libc::pid_t);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("now")
        .as_secs() as i64;
    match cfg!(target_os = "macos") {
        true => assert!(me.is_some_and(|a| a.started > 0 && a.started <= now)),
        false => assert_eq!(me, None, "no responsible app off macOS"),
    }
}

/// Done-when 4: every target folder lacking `.metadata_never_index` gets one, each listed, sccache's folder too; one that
/// has it, a folder that is no Cargo target, and one under node_modules are left alone.
#[dash_test]
fn spotlight_markers_go_in_each_build_cache_lacking_one() {
    let dir = TestDir::new("doctor-spotlight");
    let mk = |rel: &str, tag: bool| {
        let p = dir.0.join(rel);
        std::fs::create_dir_all(&p).expect("a folder");
        if tag {
            std::fs::write(
                p.join("CACHEDIR.TAG"),
                "Signature: 8a477f597d28d172789f06886806bc55",
            )
            .expect("tag");
        }
        p
    };
    let a = mk("repo/site_perso/rust/target", true);
    let b = mk("repo/klar/target", true);
    let done = mk("repo/graal/target", true);
    std::fs::write(done.join(MARKER), "").expect("a marker");
    mk("repo/notes/target", false);
    mk("repo/web/node_modules/x/target", true);
    let sccache = mk("sccache", false);
    let mut dirs = vec![sccache.clone()];
    dirs.extend(targets(&dir.0.join("repo"), 5));
    assert_eq!(
        dirs[1..],
        [done.clone(), b.clone(), a.clone()],
        "the tagged targets, sorted"
    );
    let written: Vec<PathBuf> = unindex(&dirs)
        .into_iter()
        .map(|(p, r)| r.map(|()| p).expect("written"))
        .collect();
    assert_eq!(
        written,
        [sccache.join(MARKER), b.join(MARKER), a.join(MARKER)]
    );
    assert!(unindex(&dirs).is_empty(), "a second run writes nothing");
}

/// Done-when 5's record: the steps left, each with the file that holds it, replace the last run's.
struct At(i64);

impl WallClock for At {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

#[dash_test]
fn the_steps_left_replace_the_last_run_s() {
    let dir = TestDir::new("doctor-record");
    let step = |s: &str, f: Option<&str>| Step {
        says: s.into(),
        file: f.map(PathBuf::from),
    };
    record(
        &dir.0,
        &At(7_400),
        &[
            step("relaunch Ghostty", None),
            step(
                "touch /x/.metadata_never_index",
                Some("/x/.metadata_never_index"),
            ),
        ],
    )
    .expect("recorded");
    let read = || -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir.0.join(RECORD)).expect("the record"))
            .expect("json")
    };
    assert_eq!(
        read()["steps"][0],
        json!({"says": "relaunch Ghostty", "file": null})
    );
    assert_eq!(read()["at"], 7, "the clock's seconds");
    assert_eq!(read()["steps"][1]["file"], "/x/.metadata_never_index");
    record(&dir.0, &At(8_000), &[]).expect("recorded");
    assert_eq!(
        read(),
        json!({"at": 8, "steps": []}),
        "a run with nothing left clears the handoff"
    );
}

/// Done-when 5's make setup: it runs dash doctor --sudo, and no other recipe asks sudo of it.
#[dash_test]
fn make_setup_runs_the_doctor_with_sudo_and_nothing_else_does() {
    let Some(top) = dash_meter::monorepo() else {
        return;
    };
    let make = std::fs::read_to_string(top.join("Makefile")).expect("the Makefile");
    let setup = make.split("\nsetup:").nth(1).expect("a setup recipe");
    let recipe: String = setup
        .lines()
        .skip(1)
        .take_while(|l| l.starts_with('\t'))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        recipe.contains("dash\" doctor --sudo"),
        "setup's recipe:\n{recipe}"
    );
    assert_eq!(
        make.matches("doctor --sudo").count(),
        1,
        "sudo is asked by make setup only"
    );
}

/// sudo as make setup runs it: each call counted in DOCTOR_ASKS, then the program it names run.
const SUDO: &str = "#!/bin/sh\necho \"sudo $*\" >> \"$DOCTOR_ASKS\"\nexec \"$@\"\n";
/// sqlite3 answering TCC.db's Developer Tools rows: Ghostty granted at 2000, Terminal at 500; each read counted.
const SQLITE3: &str = "#!/bin/sh\necho \"sqlite3 $*\" >> \"$DOCTOR_READS\"\nprintf 'com.mitchellh.ghostty\\t2000\\ncom.apple.Terminal\\t500\\n'\n";

/// Done-when 2's grant half: every builder's app judged from one read of TCC.db, by sudo when asked (one ask for three
/// apps), the database and query named; without --sudo no sudo runs at all.
#[dash_test]
fn the_grants_come_from_one_sudo_read_of_tcc_db_for_every_app() {
    let dir = TestDir::new("doctor-grants");
    let (asks, reads) = (dir.0.join("asks"), dir.0.join("reads"));
    let r = Readers {
        sudo: dash_meter::stand_in(SUDO),
        sqlite3: dash_meter::stand_in(SQLITE3),
        env: vec![
            ("DOCTOR_ASKS".into(), asks.display().to_string()),
            ("DOCTOR_READS".into(), reads.display().to_string()),
        ],
    };
    let app = |name: &str, client: &str| App {
        name: name.into(),
        client: client.into(),
        started: 1_000,
    };
    let apps = [
        app("Ghostty", "com.mitchellh.ghostty"),
        app("Terminal", "com.apple.Terminal"),
        app("Zed", "dev.zed.Zed"),
    ];
    let lines = |p: &Path| std::fs::read_to_string(p).unwrap_or_default();
    let (read, steps) = judge(&apps, None, true, &r);
    assert!(read, "the stand-in answered");
    assert_eq!(
        steps,
        [
            Some("relaunch Ghostty: it started before its Developer Tools grant".to_string()),
            None,
            Some(format!("{DEV_TOOLS} Zed")),
        ]
    );
    let asked = lines(&asks);
    assert_eq!(
        asked.lines().count(),
        1,
        "one sudo for three apps:\n{asked}"
    );
    assert!(
        asked.starts_with(&format!("sudo {} -separator", r.sqlite3.display())),
        "{asked}"
    );
    assert!(
        asked.contains("/Library/Application Support/com.apple.TCC/TCC.db"),
        "{asked}"
    );
    assert!(
        asked.contains("service = 'kTCCServiceDeveloperTool' and auth_value = 2"),
        "{asked}"
    );
    assert_eq!(lines(&reads).lines().count(), 1, "one read");
    let (read, _) = judge(&apps, None, false, &r);
    assert!(read);
    assert_eq!(
        lines(&asks).lines().count(),
        1,
        "plain dash doctor asks no sudo"
    );
    assert_eq!(lines(&reads).lines().count(), 2, "it reads TCC.db itself");
}
