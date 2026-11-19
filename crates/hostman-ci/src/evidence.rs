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

/// What a box's reinstall says when the tick's capture time was spent before its capture began: it is reinstalled all the
/// same.
pub const SPENT: &str = "not captured: the tick's capture time was spent";

/// The tick's capture time: `WHOLE_S` shared by every box the tick reinstalls, never 90 s each, so a tick that wipes three
/// boxes still ends within it. Started again as each tick starts; `clock` counts seconds.
pub struct Budget {
    clock: Box<dyn Fn() -> u64>,
    began: std::cell::Cell<u64>,
}

impl Budget {
    pub fn new(clock: Box<dyn Fn() -> u64>) -> Self {
        let began = std::cell::Cell::new(clock());
        Self { clock, began }
    }

    /// A new tick: its `WHOLE_S` begin now.
    pub fn start(&self) {
        self.began.set((self.clock)());
    }

    /// The box's capture within what is left of the tick's time (`gather`), or `None` when nothing is left.
    pub fn capture(&self, steps: Vec<Read>) -> Option<String> {
        let elapsed = || (self.clock)().saturating_sub(self.began.get());
        (elapsed() < WHOLE_S).then(|| gather(steps, &elapsed))
    }
}

/// The capture written to `<dir>/<server>-<utc>.txt`, the folder made first: its path, or why it was not written.
pub fn kept(dir: &Path, server: &str, utc: &str, text: &str) -> Result<String, String> {
    let file = dir.join(format!("{server}-{}.txt", utc.replace(':', "")));
    let written = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&file, text));
    match written {
        Ok(()) => Ok(file.display().to_string()),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

/// How long after a box's capture none is taken again: a box whose reinstall is refused tick after tick (the old ci box's
/// vds_is_busy, a file a minute from 05:19:49 on 2026-11-19) is captured once an hour, its status naming that file.
pub const ONCE_S: i64 = 3600;
/// The captures kept per box, the newest.
pub const KEEP: usize = 20;

/// A box's captures in `dir`, oldest first: each `<server>-<utc>.txt` (utc as `kept` writes it, 2026-11-19T051949Z) and
/// its time in seconds. A name whose rest is no such time is another box's (ci's never ci-2's).
fn captures(dir: &Path, server: &str) -> Vec<(std::path::PathBuf, i64)> {
    let prefix = format!("{server}-");
    let time = |utc: &str| -> Option<i64> {
        let (day, rest) = (utc.get(..10)?, utc.get(10..)?);
        let hms = rest.strip_prefix('T')?.strip_suffix('Z')?;
        let n = |r: std::ops::Range<usize>| hms.get(r)?.parse::<i64>().ok();
        let ok = hms.len() == 6 && hms.bytes().all(|b| b.is_ascii_digit());
        ok.then_some(())?;
        Some(dash_core::days(day)? * 86_400 + n(0..2)? * 3600 + n(2..4)? * 60 + n(4..6)?)
    };
    let mut all: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let utc = name.strip_prefix(&prefix)?.strip_suffix(".txt")?;
            Some((e.path(), time(utc)?))
        })
        .collect();
    all.sort_by_key(|(_, at)| *at);
    all
}

/// The box's capture by `capture`, unless its newest file in `dir` is younger than `ONCE_S` at `now_s`: then that file,
/// named, and nothing captured. Read from the folder, so a restart keeps the hour. Its newest `KEEP` kept after.
pub fn once(dir: &Path, server: &str, now_s: i64, capture: impl FnOnce() -> String) -> String {
    if let Some((last, at)) = captures(dir, server).pop()
        && now_s - at < ONCE_S
    {
        return format!(
            "evidence in {}, the box's capture of the hour",
            last.display()
        );
    }
    let said = capture();
    prune(dir, server);
    said
}

/// A box's captures past its newest `KEEP` removed; another box's untouched.
pub fn prune(dir: &Path, server: &str) {
    let all = captures(dir, server);
    for (old, _) in &all[..all.len().saturating_sub(KEEP)] {
        let _ = std::fs::remove_file(old);
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

/// What ssh runs on the box to read Caddy's certificate store before a reinstall wipes it (task ci-caddy-keeps-certs): its
/// certificates and ACME account, gzipped, base64 on one line; nothing when it holds none. pipefail: tar failing midway
/// fails the read, never a cut store passed as whole.
pub const STORE_READ: &str = "set -o pipefail; d=/var/lib/caddy/.local/share/caddy; if [ -d $d/certificates ]; then tar -C /var/lib/caddy/.local/share/caddy -czf - certificates acme | base64 -w0; fi";

/// A store read is whole when it decodes and lists to its end as a gzip tar holding a certificates/ folder, its base64 on
/// stdin: anything else keeps the last good one (`keep_store`). The listing is taken whole first, so a cut archive, whose
/// first entries list before tar fails, fails here too.
pub const STORE_CHECK: &str =
    "l=$(base64 -d | tar -tzf -) && printf '%s\\n' \"$l\" | grep -q '^certificates/'";

/// The push's first words: refused (exit 3) unless the box runs the install `id` names, which that install's cloud-init
/// wrote to `file`: a box still showing the old system, about to be wiped, takes nothing.
pub fn install_guard(id: &str, file: &str) -> String {
    format!(
        "[ \"$(cat {file} 2>/dev/null)\" = \"{id}\" ] || {{ echo not install {id} yet >&2; exit 3; }}; "
    )
}

/// What ssh runs on a freshly reinstalled box to take its kept Caddy store back, the store on its stdin (task
/// ci-caddy-keeps-certs): refused unless it runs install `id` and its cloud-init is done (Caddy and its user exist, its
/// Caddyfile written), then the store unpacked into Caddy's folder as caddy's alone, and Caddy enabled and started only
/// after.
pub fn store_push(id: &str) -> String {
    install_guard(id, crate::INSTALL_ID)
        + "[ -e /var/lib/cloud/instance/boot-finished ] || { echo its cloud-init still runs >&2; exit 3; }; \
           d=/var/lib/caddy/.local/share/caddy && mkdir -p $d && base64 -d | tar -C $d -xzf - && \
           chown -R caddy:caddy /var/lib/caddy && chmod -R go-rwx $d && systemctl enable --now caddy"
}

/// A box's Caddy store kept in `dir` (0700) as `<server>.tgz.b64` (0600), outside git: what `read` got off the box,
/// written over the last only once `whole` (`STORE_CHECK`) says it is a whole store; a read that gave nothing, failed (the
/// box hung), was cut short or holds no certificates keeps the last good one. Out: the store to give back, or none when
/// none was ever kept.
pub fn keep_store(
    dir: &Path,
    server: &str,
    read: Result<String, String>,
    whole: &dyn Fn(&str) -> bool,
) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let file = dir.join(format!("{server}.tgz.b64"));
    let fresh = read
        .ok()
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty() && whole(r));
    if let Some(store) = &fresh {
        // written beside, then renamed over: the last good one is never half replaced
        let next = dir.join(format!("{server}.tgz.b64.new"));
        let wrote = std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)))
            .and_then(|()| std::fs::write(&next, store))
            .and_then(|()| std::fs::set_permissions(&next, std::fs::Permissions::from_mode(0o600)))
            .and_then(|()| std::fs::rename(&next, &file));
        if wrote.is_ok() {
            return fresh;
        }
    }
    std::fs::read_to_string(&file)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// ssh's words to the box at `ip`: the reconciler's `key` alone (IdentitiesOnly), its own `known` hosts file (never his
/// ~/.ssh/known_hosts), a new box's host key taken, then `cmd` (`ON_THE_BOX`, `STORE_READ`).
pub fn ssh_args(key: &Path, known: &Path, ip: &str, cmd: &str) -> Vec<String> {
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
    args.extend([format!("root@{ip}"), cmd.to_string()]);
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

    /// A box is captured once an hour at most, read from its newest file, so a restart keeps it: the old ci box refused
    /// every PATCH with vds_is_busy, and each tick captured it again (ci-2026-11-19T051949Z.txt to 052400Z.txt, one a
    /// minute). Inside the hour the status names the last file; past it, a new capture. ci's files are never ci-2's.
    #[dash_test]
    fn a_box_is_captured_once_an_hour_and_its_newest_20_kept() {
        let dir = std::env::temp_dir().join(format!("ci-once-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in [
            "ci-2026-11-19T051949Z.txt",
            "ci-2026-11-19T052400Z.txt",
            "ci-2-2026-11-19T060000Z.txt",
        ] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        let at = |hms: &str| dash_core::days("2026-11-19").unwrap() * 86_400 + secs(hms);
        fn secs(hms: &str) -> i64 {
            let n = |r: std::ops::Range<usize>| hms[r].parse::<i64>().unwrap();
            n(0..2) * 3600 + n(2..4) * 60 + n(4..6)
        }
        let ran = Cell::new(0);
        let capture = || {
            ran.set(ran.get() + 1);
            "evidence in new".to_string()
        };
        let last = dir.join("ci-2026-11-19T052400Z.txt").display().to_string();
        let said = once(&dir, "ci", at("055000"), capture);
        assert_eq!(
            said,
            format!("evidence in {last}, the box's capture of the hour")
        );
        assert_eq!(ran.get(), 0, "inside the hour, nothing captured again");
        assert_eq!(
            once(&dir, "ci", at("062400"), capture),
            "evidence in new",
            "an hour after, captured again"
        );
        assert_eq!(
            once(&dir, "ci-3", at("055000"), capture),
            "evidence in new",
            "another box's files are not its"
        );
        // the newest 20 a box's kept, another box's untouched
        for m in 0..25 {
            std::fs::write(dir.join(format!("ci-2026-11-18T10{m:02}00Z.txt")), "x").unwrap();
        }
        prune(&dir, "ci");
        let mut left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left.iter().filter(|f| f.starts_with("ci-2026")).count(),
            KEEP
        );
        assert!(
            left.contains(&"ci-2026-11-19T052400Z.txt".into())
                && !left.contains(&"ci-2026-11-18T100400Z.txt".into())
        );
        assert!(left.contains(&"ci-2-2026-11-19T060000Z.txt".into()));
        let _ = std::fs::remove_dir_all(&dir);
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
        )
        .unwrap();
        assert!(
            path.ends_with("ci-heal/ci-2-2026-11-19T034309Z.txt"),
            "{path}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "== server ==\non\n"
        );
        let blocked = dir.join("ci-heal/ci-2-2026-11-19T034309Z.txt/below");
        assert!(kept(&blocked, "ci-2", "t", "x").is_err_and(|e| e.contains("ci-2-t.txt")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One tick's captures share its 90 s, never 90 each: three boxes, the first spends 80, the second gets the 10 left,
    /// the third none, said as such; started again, the next tick has its 90.
    #[dash_test]
    fn a_tick_s_captures_share_its_90_s_and_a_box_past_them_says_so() {
        let clock = std::rc::Rc::new(Cell::new(1000));
        let c = std::rc::Rc::clone(&clock);
        let budget = Budget::new(Box::new(move || c.get()));
        let given = RefCell::new(vec![]);
        // a box's capture: four steps, each taking all it is given
        let steps = || -> Vec<Read> {
            (0..4)
                .map(|_| -> Read {
                    let (clock, given) = (&clock, &given);
                    (
                        "ssh",
                        0,
                        Box::new(move |limit| {
                            given.borrow_mut().push(limit);
                            clock.set(clock.get() + limit);
                            Err("timed out".into())
                        }),
                    )
                })
                .collect()
        };
        let boxes: Vec<_> = (0..3).map(|_| budget.capture(steps())).collect();
        assert_eq!(
            *given.borrow(),
            [20, 20, 20, 20, 10],
            "80 s, then the 10 left, then nothing"
        );
        assert_eq!(
            clock.get() - 1000,
            WHOLE_S,
            "the three within the tick's 90 s"
        );
        assert!(
            boxes[1]
                .as_deref()
                .is_some_and(|t| t.ends_with("skipped: the capture's 90 s are spent\n\n"))
        );
        assert_eq!(boxes[2], None, "the third: {SPENT}");
        budget.start();
        assert!(
            budget.capture(steps()).is_some(),
            "the next tick has its 90"
        );
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

    /// `STORE_CHECK` run here as the reconciler runs it: the store on its stdin.
    fn checked(store: &str) -> bool {
        use std::io::Write;
        let mut sh = std::process::Command::new("sh")
            .args(["-c", STORE_CHECK])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        sh.stdin
            .take()
            .unwrap()
            .write_all(store.as_bytes())
            .unwrap();
        sh.wait().unwrap().success()
    }

    /// A store as STORE_READ makes it: Caddy's folders gzipped, base64 on one line.
    fn archive(root: &Path, with_certs: bool) -> String {
        let _ = std::fs::remove_dir_all(root);
        std::fs::create_dir_all(root.join("acme")).unwrap();
        if with_certs {
            std::fs::create_dir_all(root.join("certificates/le/whatsnr.eu")).unwrap();
            std::fs::write(
                root.join("certificates/le/whatsnr.eu/whatsnr.eu.crt"),
                "-----BEGIN CERTIFICATE-----\n",
            )
            .unwrap();
        }
        let dirs = if with_certs {
            "certificates acme"
        } else {
            "acme"
        };
        let made = std::process::Command::new("sh")
            .args([
                "-c",
                &format!(
                    "tar -C {} -czf - {dirs} | base64 | tr -d '\\n'",
                    root.display()
                ),
            ])
            .output()
            .unwrap();
        String::from_utf8(made.stdout).unwrap()
    }

    /// Caddy's certificate store, read off the box before its wipe, is kept in the data folder (0600, its folder 0700) and
    /// given back: a box whose read gave nothing, failed (hung), was cut short or holds no certificates/ folder keeps the
    /// last good store; one with none gets nothing. whatsnr.eu's certificate was asked of Let's Encrypt at each reinstall
    /// until its 5 a week ran out (2026-11-19).
    #[dash_test]
    fn caddy_s_store_is_kept_off_the_box_and_given_back() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("ci-store-{}", std::process::id()));
        let dir = base.join("caddy-store");
        let good = archive(&base.join("box"), true);
        assert!(checked(&good), "a real store passes the check");
        assert_eq!(
            keep_store(&dir, "ci-2", Ok(String::new()), &checked),
            None,
            "nothing read, nothing kept"
        );
        assert_eq!(
            keep_store(&dir, "ci-2", Ok(format!("{good}\n")), &checked),
            Some(good.clone())
        );
        let file = dir.join("ci-2.tgz.b64");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&dir), mode(&file)), (0o700, 0o600));
        let last = |read: Result<String, String>| keep_store(&dir, "ci-2", read, &checked);
        assert_eq!(
            last(Err("ssh exited 255".into())),
            Some(good.clone()),
            "a hung box: the last kept"
        );
        assert_eq!(
            last(Ok(String::new())),
            Some(good.clone()),
            "an empty read never erases it"
        );
        // tar failing mid-read: the base64 cut inside its gzip stream (here bsdtar pads its output to 10240 bytes, so a cut
        // in the padding would still be whole; GNU tar on the box pads nothing)
        assert_eq!(
            last(Ok(good[..120].to_string())),
            Some(good.clone()),
            "a cut store keeps the last"
        );
        // an archive with no certificates/ folder
        let empty = archive(&base.join("bare"), false);
        assert_eq!(
            last(Ok(empty)),
            Some(good.clone()),
            "an empty archive keeps the last"
        );
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            good,
            "the file still holds the last good one"
        );
        assert_eq!(
            keep_store(&dir, "ci-3", Err("no".into()), &checked),
            None,
            "another box's is not its"
        );
        assert!(STORE_READ.starts_with("set -o pipefail; "), "{STORE_READ}");
        assert!(
            STORE_READ
                .contains("tar -C /var/lib/caddy/.local/share/caddy -czf - certificates acme"),
            "{STORE_READ}"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The capture's ssh uses the reconciler's key alone, its own known-hosts file; a box made before that key refuses it,
    /// said as such and the capture goes on.
    #[dash_test]
    fn the_capture_s_ssh_uses_the_reconciler_s_key_alone() {
        let args = ssh_args(
            Path::new("/d/hostman-ci-ssh/id_ed25519"),
            Path::new("/d/known"),
            "1.2.3.4",
            ON_THE_BOX,
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
