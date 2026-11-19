//! What a heal reads of a box before its reinstall wipes it (task ci-heal-evidence): the wipe destroys the only proof of
//! why the box hung (ci-2, server 9639793, healed at 03:43:09 on 2026-11-19 before its journal was read). A bounded
//! capture: each step gets the smaller of `STEP_S` and what is left of `WHOLE_S`, a step whose own bound cannot shrink to
//! that is skipped, so the whole never passes `WHOLE_S`; a failed step is written as such, and the heal runs whatever the
//! capture got.
use std::path::Path;

/// The whole capture's bound.
pub const WHOLE_S: u64 = 90;
/// Each step's bound.
pub const STEP_S: u64 = 20;
/// What ssh runs on the box: its kernel's last words, the runner's journal, the pressure on its disk and memory, its RAM,
/// disk and load.
pub const ON_THE_BOX: &str = "dmesg -T | tail -200; journalctl -u 'actions.runner*' --since -2h --no-pager | tail -200; \
                              cat /proc/pressure/io /proc/pressure/memory; free -m; df -h; uptime";

/// One step of the capture: its name; the bound its call holds whatever it is given (the hostman tool's curl, 20 s), 0
/// for one that takes the limit it is given (ssh); and what reads it, given its limit in seconds.
pub type Read<'a> = (
    &'a str,
    u64,
    Box<dyn FnOnce(u64) -> Result<String, String> + 'a>,
);

/// Each step's answer under its name, in order: what it read, « failed: » and why, or « skipped » when what is left of
/// `WHOLE_S` (by `elapsed_s`, the seconds since the capture began) is spent, or is under the step's own fixed bound. Each
/// step's limit is the smaller of `STEP_S` and what is left.
pub fn gather(steps: Vec<Read>, elapsed_s: &dyn Fn() -> u64) -> String {
    let mut text = String::new();
    for (name, fixed, read) in steps {
        let left = WHOLE_S.saturating_sub(elapsed_s());
        let limit = STEP_S.min(left);
        let said = match () {
            _ if limit == 0 => format!("skipped: the capture's {WHOLE_S} s are spent"),
            _ if fixed > limit => {
                format!("skipped: {limit} s left of the capture's {WHOLE_S}, under its {fixed}")
            }
            _ => match read(limit) {
                Ok(out) => out.trim_end().to_string(),
                Err(why) => format!("failed: {why}"),
            },
        };
        text.push_str(&format!("== {name} ==\n{said}\n\n"));
    }
    text
}

/// The capture written to `<dir>/<server>-<utc>.txt`, the folder made first: its path, or why it was not written.
pub fn kept(dir: &Path, server: &str, utc: &str, text: &str) -> String {
    let file = dir.join(format!("{server}-{}.txt", utc.replace(':', "")));
    let written = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&file, text));
    match written {
        Ok(()) => file.display().to_string(),
        Err(e) => format!("not written ({}: {e})", file.display()),
    }
}

/// The reconciler's own ssh key in `folder` (0700), keyed off its private key: none, `keygen` makes both (ssh-keygen -t
/// ed25519, no passphrase, the key 0600); the private key alone, `derive` gives its public key (ssh-keygen -y), written
/// beside it. Its public key back, or a named error: the capture uses this key alone, never his personal key, and the
/// boxes take it beside his in their cloud-init, so a key that cannot be made is said, never dropped.
pub fn own_key(
    folder: &Path,
    keygen: &dyn Fn(&Path) -> Result<(), String>,
    derive: &dyn Fn(&Path) -> Result<String, String>,
) -> Result<String, String> {
    use std::os::unix::fs::PermissionsExt;
    let key = folder.join("id_ed25519");
    let public = key.with_extension("pub");
    let made = || -> Result<String, String> {
        std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        if !key.exists() {
            keygen(&key)?;
        } else if !public.exists() {
            let derived = derive(&key)?;
            std::fs::write(&public, format!("{}\n", derived.trim()))
                .map_err(|e| format!("{}: {e}", public.display()))?;
        }
        let text =
            std::fs::read_to_string(&public).map_err(|e| format!("{}: {e}", public.display()))?;
        Ok(text.trim().to_string())
    };
    made().map_err(|e| format!("the reconciler's key: {e}"))
}

/// ssh's words to the box at `ip`: the reconciler's `key` alone (IdentitiesOnly), its own `known` hosts file (never his
/// ~/.ssh/known_hosts), a new box's host key taken, then `ON_THE_BOX`.
pub fn ssh_args(key: &Path, known: &Path, ip: &str) -> Vec<String> {
    let o = |s: String| ["-o".to_string(), s];
    let mut args = vec!["-i".to_string(), key.display().to_string()];
    for opt in [
        "IdentitiesOnly=yes".to_string(),
        "BatchMode=yes".into(),
        "ConnectTimeout=10".into(),
        format!("UserKnownHostsFile={}", known.display()),
        "StrictHostKeyChecking=accept-new".into(),
    ] {
        args.extend(o(opt));
    }
    args.extend([format!("root@{ip}"), ON_THE_BOX.to_string()]);
    args
}

/// What ssh's run says: its output, or why none; a box made before the reconciler's key (it refuses that key) said as such.
pub fn ssh_said(code: i32, stderr: &str, stdout: &str) -> Result<String, String> {
    match code {
        0 => Ok(stdout.to_string()),
        _ if stderr.contains("Permission denied") => {
            Err("ssh: box predates the reconciler's key".into())
        }
        _ => Err(format!(
            "ssh exited {code}: {}\n{}",
            stderr.trim(),
            stdout.trim()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::cell::{Cell, RefCell};

    /// Each step is written under its name, what it read or a failure as such, given the smaller of 20 s and what is left of
    /// the 90; a step past the whole is skipped, never run.
    #[dash_test]
    fn the_capture_writes_each_step_a_failure_as_such_and_skips_past_its_bound() {
        let clock = Cell::new(0);
        let ran = Cell::new(false);
        let steps: Vec<Read> = vec![
            (
                "server",
                20,
                Box::new(|_| Ok("{\"status\":\"on\"}\n".into())),
            ),
            (
                "ssh",
                0,
                Box::new(|limit| {
                    clock.set(clock.get() + limit);
                    Err(format!("timed out after {limit} s"))
                }),
            ),
            (
                "statistics",
                0,
                Box::new(|_| {
                    clock.set(95);
                    Ok("ram".into())
                }),
            ),
            (
                "log",
                0,
                Box::new(|_| {
                    ran.set(true);
                    Ok("never".into())
                }),
            ),
        ];
        let text = gather(steps, &|| clock.get());
        assert_eq!(
            text,
            "== server ==\n{\"status\":\"on\"}\n\n\
             == ssh ==\nfailed: timed out after 20 s\n\n\
             == statistics ==\nram\n\n\
             == log ==\nskipped: the capture's 90 s are spent\n\n"
        );
        assert!(!ran.get(), "a step past the bound never runs");
        assert!(ON_THE_BOX.contains("journalctl -u 'actions.runner*' --since -2h"));
    }

    /// A slow step started near the end gets what is left, never its full 20, and one whose own bound cannot shrink to
    /// that is skipped: the whole never passes 90 s.
    #[dash_test]
    fn a_step_near_the_end_gets_what_is_left_and_the_whole_never_passes_90_s() {
        let clock = Cell::new(80);
        let given = RefCell::new(vec![]);
        let steps: Vec<Read> = vec![
            ("hostman", 20, Box::new(|_| Ok("never".into()))),
            (
                "ssh",
                0,
                Box::new(|limit| {
                    given.borrow_mut().push(limit);
                    clock.set(clock.get() + limit);
                    Err("timed out".into())
                }),
            ),
        ];
        let text = gather(steps, &|| clock.get());
        assert_eq!(*given.borrow(), [10], "10 s left of 90");
        assert!(clock.get() <= WHOLE_S, "{}", clock.get());
        assert!(
            text.contains("== hostman ==\nskipped: 10 s left of the capture's 90, under its 20"),
            "{text}"
        );
    }

    /// The file lands in its folder, made first, named by the server and the time; one that cannot be written says why.
    #[dash_test]
    fn the_capture_is_kept_in_its_file_or_says_why_not() {
        let dir = std::env::temp_dir().join(format!("ci-heal-{}", std::process::id()));
        let path = kept(
            &dir.join("ci-heal"),
            "ci-2",
            "2026-11-19T03:43:09Z",
            "== server ==\non\n",
        );
        assert!(
            path.ends_with("ci-heal/ci-2-2026-11-19T034309Z.txt"),
            "{path}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "== server ==\non\n"
        );
        let blocked = dir.join("ci-heal/ci-2-2026-11-19T034309Z.txt/below");
        assert!(kept(&blocked, "ci-2", "t", "x").starts_with("not written ("));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The reconciler's own key, keyed off its private key: made once (folder 0700, key 0600), never again; a private key
    /// left without its .pub gets its .pub derived, never a new key; one that cannot be made is a named error.
    #[dash_test]
    fn the_reconciler_s_key_is_made_once_and_a_half_made_one_completed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ci-key-{}", std::process::id()));
        let (made, derived) = (Cell::new(0), Cell::new(0));
        let keygen = |key: &Path| -> Result<(), String> {
            made.set(made.get() + 1);
            std::fs::write(key, "PRIVATE").unwrap();
            std::fs::set_permissions(key, std::fs::Permissions::from_mode(0o600)).unwrap();
            std::fs::write(key.with_extension("pub"), "ssh-ed25519 AAAA hostman-ci\n").unwrap();
            Ok(())
        };
        let derive = |_: &Path| -> Result<String, String> {
            derived.set(derived.get() + 1);
            Ok("ssh-ed25519 AAAA hostman-ci".into())
        };
        let folder = dir.join("hostman-ci-ssh");
        let own = || own_key(&folder, &keygen, &derive);
        assert_eq!(own(), Ok("ssh-ed25519 AAAA hostman-ci".into()));
        assert_eq!(own(), Ok("ssh-ed25519 AAAA hostman-ci".into()));
        assert_eq!((made.get(), derived.get()), (1, 0), "made once");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&folder), 0o700);
        assert_eq!(mode(&folder.join("id_ed25519")), 0o600);
        // half made: the private key kept, its .pub lost
        std::fs::remove_file(folder.join("id_ed25519.pub")).unwrap();
        assert_eq!(own(), Ok("ssh-ed25519 AAAA hostman-ci".into()));
        assert_eq!(
            (made.get(), derived.get()),
            (1, 1),
            "derived, never made again"
        );
        assert!(folder.join("id_ed25519.pub").exists());
        let failed = own_key(
            &dir.join("other"),
            &|_| Err("ssh-keygen: not found".into()),
            &derive,
        );
        assert_eq!(
            failed,
            Err("the reconciler's key: ssh-keygen: not found".into())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The capture's ssh uses the reconciler's key alone, its own known-hosts file; a box made before that key refuses it,
    /// said as such and the capture goes on.
    #[dash_test]
    fn the_capture_s_ssh_uses_the_reconciler_s_key_alone() {
        let args = ssh_args(
            Path::new("/d/hostman-ci-ssh/id_ed25519"),
            Path::new("/d/known"),
            "1.2.3.4",
        );
        let joined = args.join(" ");
        assert!(
            joined.contains("-i /d/hostman-ci-ssh/id_ed25519 -o IdentitiesOnly=yes"),
            "{joined}"
        );
        assert!(
            joined.contains("UserKnownHostsFile=/d/known")
                && joined.contains("StrictHostKeyChecking=accept-new")
        );
        assert!(
            !joined.contains(".ssh/known_hosts") && joined.contains("root@1.2.3.4"),
            "{joined}"
        );
        assert_eq!(args.last().map(String::as_str), Some(ON_THE_BOX));
        assert_eq!(ssh_said(0, "", "ok\n"), Ok("ok\n".into()));
        assert_eq!(
            ssh_said(
                255,
                "root@1.2.3.4: Permission denied (publickey,password).",
                ""
            ),
            Err("ssh: box predates the reconciler's key".into())
        );
        assert!(
            ssh_said(255, "Connection timed out during banner exchange", "")
                .is_err_and(|e| e.contains("banner"))
        );
    }
}
