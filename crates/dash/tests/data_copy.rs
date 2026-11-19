//! `dash data copy <dir>`, the real binary, on a live folder a task's sandbox reads and never writes (task rust-live-copy).
use dash_meter::TestDir;
use dash_meter::dash_test;
use dash_store::Connection;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn mode(path: &std::path::Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

/// The live folder made writable again, the test failed or not, so its TestDir can go.
struct Writable(std::path::PathBuf);

impl Drop for Writable {
    fn drop(&mut self) {
        mode(&self.0, 0o755);
    }
}

fn copy_into(dir: &TestDir, to: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(["data", "copy"])
        .arg(to)
        .env("DASH_REPO", dir.0.join("repo"))
        .output()
        .expect("dash data copy")
}

/// Done-when 1: the copy holds a write still in the writer's WAL, read from a folder made read-only as a task's sandbox makes
/// the live store; journals, locks and the telemetry switch stay behind, a plain file comes along, and the live folder itself is refused.
#[dash_test]
fn a_task_copies_the_live_data_it_cannot_write() {
    let dir = TestDir::new("data-copy");
    let live = dir.0.join("repo/private/live/rust/data");
    std::fs::create_dir_all(&live).expect("live");
    let writer = Connection::open(live.join("tasks.sqlite")).expect("the app's writer");
    writer
        .execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;
             CREATE TABLE t(id TEXT); INSERT INTO t VALUES ('in the wal');",
        )
        .expect("a write");
    // a WAL database no process holds: no -wal, no -shm, which a read-only open could not create here
    let closed = Connection::open(live.join("drafts.sqlite")).expect("closed");
    closed
        .execute_batch(
            "PRAGMA journal_mode=WAL; CREATE TABLE d(id TEXT); INSERT INTO d VALUES ('closed');",
        )
        .expect("a write");
    drop(closed);
    assert!(
        !live.join("drafts.sqlite-shm").exists(),
        "closed, its journals gone"
    );
    std::fs::write(live.join("codemap.json"), "{}").expect("a plain file");
    std::fs::write(live.join("orch.lock"), "").expect("a lock");
    std::fs::write(live.join("telemetry"), "127.0.0.1:8125").expect("the live switch");
    for f in std::fs::read_dir(&live).expect("live") {
        mode(&f.expect("a file").path(), 0o444);
    }
    mode(&live, 0o555);
    let _writable = Writable(live.clone());

    let to = dir.0.join("copy");
    let first = copy_into(&dir, &to);
    assert!(first.status.success(), "{first:?}");
    // a second copy over the first, its read-only live file copied before
    let ran = copy_into(&dir, &to);
    let said = String::from_utf8_lossy(&ran.stdout);
    assert!(
        ran.status.success(),
        "{said} {}",
        String::from_utf8_lossy(&ran.stderr)
    );
    assert!(
        said.contains(": codemap.json drafts.sqlite tasks.sqlite\nnext: dash <command> --data "),
        "{said}"
    );
    let mut copied: Vec<String> = std::fs::read_dir(&to)
        .expect("the copy")
        .map(|f| {
            f.expect("a file")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    copied.sort();
    assert_eq!(copied, ["codemap.json", "drafts.sqlite", "tasks.sqlite"]);
    let copy = Connection::open(to.join("tasks.sqlite")).expect("the copy");
    let row: String = copy
        .query_row("SELECT id FROM t", [], |r| r.get(0))
        .expect("the row");
    assert_eq!(row, "in the wal");
    let drafts = Connection::open(to.join("drafts.sqlite")).expect("the copy");
    let row: String = drafts
        .query_row("SELECT id FROM d", [], |r| r.get(0))
        .expect("the row");
    assert_eq!(row, "closed");
    let check: String = copy
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .expect("checked");
    assert_eq!(check, "ok");

    let onto_live = copy_into(&dir, &live);
    assert!(!onto_live.status.success());
    assert!(
        String::from_utf8_lossy(&onto_live.stderr).contains("the live folder itself, never a copy")
    );
    // a flag is never taken for the folder (run-fields-at-add's `dash data copy --help` made ./--help)
    let flag = Command::new(env!("CARGO_BIN_EXE_dash"))
        .args(["data", "copy", "--help"])
        .current_dir(&dir.0)
        .env("DASH_REPO", dir.0.join("repo"))
        .output()
        .expect("dash data copy --help");
    assert!(!flag.status.success());
    assert!(
        String::from_utf8_lossy(&flag.stderr).contains("--help: a flag, never a folder"),
        "{flag:?}"
    );
    assert!(!dir.0.join("--help").exists());
    drop(writer);
}
