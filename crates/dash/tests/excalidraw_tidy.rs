//! `excalidraw tidy` on a scene the sandbox will not let it write (a replicator worktree, task replicator-review-writes):
//! the tidied scene lands in a temp file and a sandbox candidate is printed, instead of an EPERM traceback.
use dash_meter::{TestDir, dash_test, monorepo};
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[dash_test]
fn tidy_prints_a_candidate_when_the_scene_is_read_only() {
    let Some(top) = monorepo() else { return };
    let dir = TestDir::new("excalidraw-tidy-ro");
    let scene = dir.0.join("ro.excalidraw");
    std::fs::write(&scene, r#"{"type":"excalidraw","elements":[]}"#).unwrap();
    std::fs::set_permissions(&scene, std::fs::Permissions::from_mode(0o444)).unwrap();
    let out = Command::new("uv")
        .args(["run", "--quiet", "--script"])
        .arg(top.join("private/dashboard/priv/commands/excalidraw"))
        .arg("tidy")
        .arg(&scene)
        .output()
        .expect("uv runs");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("sandbox candidate"),
        "stdout: {text} stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("(allow file-write*"), "{text}");
}
