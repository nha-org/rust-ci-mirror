//! A page's HTML form printed to PDF, a file to download: the Notion document's Export PDF (task notion-page-menu-export;
//! design::document_pdf). Headless Chrome prints it, under print media, so components.css's [data-print] rule keeps the
//! document alone. Chrome: chrome(), the browser tests' too. It
//! reads no other module of the crate, so no module loop runs through it.
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const CHROMES: [&str; 4] = [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
];
/// A print that takes longer, its wait for a browser slot included, is stopped: its whole process group is killed (the slot's
/// wait, Chrome and its helpers) and the answer says why, with Chrome's own lines. Under nextest's 30 s, so a test reads that
/// answer: one Chrome that started and never printed on the CI mirror ran the_notion_document_downloads_as_its_printed_pdf to
/// that cap under the old 60 s, « (test timed out) » its only words (PR 1472, run 43122582126, 2026-11-11). No second try:
/// it would turn the next stall green with its cause unread (/code-review).
/// ponytail: 20 s is also all a print waits for a browser slot; the slot's wait outside this clock if dash serve prints at the cap.
const LIMIT: Duration = Duration::from_secs(20);

/// The Chrome to start: $CHROME if that file is there (a bare name looked up on $PATH), else the newest chrome-headless-shell
/// puppeteer or playwright installed, else the first of CHROMES there is. The browser tests start the same one. The shell
/// first: it is no app bundle, so a test binary (<crate>-<hash>) starting it raises no macOS « prevented from modifying
/// apps » notice, as starting Chrome.app did (@nha, 2026-11-11: « I had a privacy an security notice "<hash> was prevented
/// to modify something on your mac" »; task rust-chrome-headless-shell).
pub fn chrome() -> Option<String> {
    chrome_from(
        std::env::var("CHROME").ok(),
        std::env::var_os("HOME").map(Into::into),
        &CHROMES,
    )
}

fn chrome_from(
    env: Option<String>,
    home: Option<std::path::PathBuf>,
    chromes: &[&str],
) -> Option<String> {
    env.and_then(on_disk)
        .or_else(|| home.and_then(|h| headless_shell(&h)))
        .or_else(|| chromes.iter().map(|c| c.to_string()).find_map(on_disk))
}

/// `c` when it names a file there is: a path as given, a bare name (CHROME=chromium) looked up on $PATH as a shell would.
fn on_disk(c: String) -> Option<String> {
    match c.contains('/') {
        true => runnable(Path::new(&c)).then_some(c),
        false => std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|d| d.join(&c))
            .find(|b| runnable(b))
            .map(|b| b.to_string_lossy().into_owned()),
    }
}

/// A file with an execute bit: a shell whose unzip was cut short before its chmod is passed over.
fn runnable(b: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    b.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The folder each install holds for this machine's platform, puppeteer's and playwright's alike: a cache holding several
/// platforms (an x64 install made under Rosetta) never hands this one a binary it cannot run.
const SHELL_FOLDER: &str = match (cfg!(target_os = "macos"), cfg!(target_arch = "aarch64")) {
    (true, true) => "chrome-headless-shell-mac-arm64",
    (true, false) => "chrome-headless-shell-mac-x64",
    (false, true) => "chrome-headless-shell-linux-arm64",
    (false, false) => "chrome-headless-shell-linux64",
};

/// The newest chrome-headless-shell under `home`: puppeteer's (~/.cache/puppeteer/chrome-headless-shell/<platform>-<version>/
/// chrome-headless-shell-<platform>/chrome-headless-shell) by its Chrome version, else playwright's
/// (ms-playwright/chromium_headless_shell-<revision>/chrome-headless-shell-<platform>/chrome-headless-shell, in a Mac's or
/// Linux's cache folder) by its revision. Puppeteer's first: playwright's folders name a revision, not a Chrome version, so
/// the two only compare by starting each.
/// ponytail: $PUPPETEER_CACHE_DIR and $PLAYWRIGHT_BROWSERS_PATH are not read; $CHROME names a shell kept elsewhere.
fn headless_shell(home: &Path) -> Option<String> {
    let puppeteer = [home.join(".cache/puppeteer/chrome-headless-shell")];
    let playwright = [
        home.join("Library/Caches/ms-playwright"),
        home.join(".cache/ms-playwright"),
    ];
    newest(&puppeteer, |name| name.rsplit_once('-').map(|(_, v)| v)).or_else(|| {
        newest(&playwright, |name| {
            name.strip_prefix("chromium_headless_shell-")
        })
    })
}

/// The runnable chrome-headless-shell, this platform's, in the folders of `caches` whose `version` (dotted numbers) is highest.
fn newest(caches: &[std::path::PathBuf], version: impl Fn(&str) -> Option<&str>) -> Option<String> {
    caches
        .iter()
        .filter_map(|cache| std::fs::read_dir(cache).ok())
        .flat_map(|installs| installs.flatten())
        .filter_map(|install| {
            let name = install.file_name().to_string_lossy().into_owned();
            let rank: Vec<u64> = version(&name)?
                .split('.')
                .map(|n| n.parse().ok())
                .collect::<Option<_>>()?;
            let bin = install
                .path()
                .join(SHELL_FOLDER)
                .join("chrome-headless-shell");
            runnable(&bin).then_some((rank, bin))
        })
        .max()
        .map(|(_, bin)| bin.to_string_lossy().into_owned())
}

/// bin/browser-slot, the cap on the browsers every site_perso process starts; the browser tests take theirs through it too.
pub fn slot_script() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../bin/browser-slot")
}

/// `html` printed to PDF by headless Chrome, no header or footer.
pub fn print(html: &str) -> Result<Vec<u8>, String> {
    static RUN: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "dash-pdf-{}-{}",
        std::process::id(),
        RUN.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let printed = chrome()
        .ok_or_else(|| "no Chrome to print with: install Google Chrome or chrome-headless-shell (npx @puppeteer/browsers install chrome-headless-shell), or set CHROME".to_string())
        .and_then(|chrome| printed_in(&dir, html, &chrome, &slot_script(), LIMIT));
    let _ = std::fs::remove_dir_all(&dir);
    printed
}

fn printed_in(
    dir: &Path,
    html: &str,
    chrome: &str,
    slot: &Path,
    limit: Duration,
) -> Result<Vec<u8>, String> {
    let page = dir.join("page.html");
    let out = dir.join("page.pdf");
    std::fs::write(&page, html).map_err(|e| format!("{}: {e}", page.display()))?;
    // One of the machine's browser slots (bin/browser-slot, the cap every browser site_perso starts shares), taken for the
    // shell's pid, which exec makes Chrome's: freed when Chrome ends. The CI mirror carries rust/ alone: no bin/ there.
    // ponytail: the path is the build's checkout, so a binary built in a worktree since removed prints with no slot; a
    // lookup from the data folder if dash ever runs away from its checkout.
    let script = match slot.exists() {
        true => {
            r#"slot=$1; shift; "$slot" $$ >/dev/null || { echo "bin/browser-slot failed" >&2; exit 1; }; exec "$@""#
        }
        false => r#"shift; exec "$@""#,
    };
    let log = dir.join("chrome.log");
    let errors = std::fs::File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let mut sh = Command::new("/bin/sh");
    // A caller holding a slot already (a browser test printing as it drives a page) covers this print: asked for a second
    // slot, five such tests each held one and waited on a sixth until nextest's 30 s (task rust-notion-browser-flake). A
    // caller with none, as dash serve, still takes one per print. A parent's BROWSER_SLOT_PID is kept.
    // ponytail: a slot holder's prints run uncounted beside its own browser (bin/browser-slot's rule for a parent's scripts):
    // two Chromes a slot for a test; a count of Chromes per slot if a holder ever prints many at once.
    if std::env::var_os("BROWSER_SLOT_PID").is_none() {
        sh.env("BROWSER_SLOT_PID", std::process::id().to_string());
    }
    let mut child = sh
        .args(["-c", script, "sh"])
        .arg(slot)
        .arg(chrome)
        .args([
            "--headless",
            "--no-sandbox",
            "--disable-gpu",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-sync",
            // No keychain ask on a Mac: a fresh profile would ask for « Chrome Safe Storage », a popup or a stall.
            "--use-mock-keychain",
            "--password-store=basic",
            "--no-pdf-header-footer",
            // no crashpad handler, which double-forks out of the group end_group ends (/code-review)
            "--disable-crash-reporter",
        ])
        .arg(format!("--user-data-dir={}", dir.join("profile").display()))
        .arg(format!("--print-to-pdf={}", out.display()))
        .arg(format!("file://{}", page.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(errors)
        // its own group, so a timeout ends all of it
        .process_group(0)
        .spawn()
        .map_err(|e| format!("{chrome}: {e}"))?;
    let end = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status.to_string();
        }
        // A PDF whole on disk ends the print, whether Chrome exits or not: chrome-headless-shell 152 stayed up after writing
        // one under a full nextest run, each such print ran to nextest's 30 s and its orphan held a browser slot (2026-11-11).
        // Its group is ended there, its slot freed with it.
        if std::fs::read(&out).is_ok_and(|pdf| whole(&pdf)) {
            end_group(&mut child);
            break "ended once its PDF was whole".to_string();
        }
        if Instant::now() > end {
            end_group(&mut child);
            return Err(format!(
                "Chrome did not print in {} s, a browser slot's wait included{}",
                limit.as_secs(),
                said(&log)
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let pdf = std::fs::read(&out)
        .map_err(|e| format!("Chrome ({status}) wrote no PDF: {e}{}", said(&log)))?;
    match whole(&pdf) {
        true => Ok(pdf),
        false => Err(format!("Chrome ({status}) wrote no PDF{}", said(&log))),
    }
}

/// What `dash ci warm-chrome` says: the Chrome a print starts, its install folder read through once (warm()), before the CI
/// mirror's test run. Why Chrome printed nothing in 20 s there (job 131105878050): on a runner, a first Chrome start reads its
/// ~390 MB of files cold from the image's disk, and under the I/O of nextest's other tests that read took 8 to 20 s, CPU idle
/// and I/O pressure at 27 to 51 % (traced reruns, 2026-11-12). Its files cached, every print there took 0.75 to 1.76 s. Read
/// inside the print, the warm-up itself took 10 to 16 s under that I/O: it runs before the tests, when the disk is free.
/// An error when nothing was read (no Chrome, or none in an install folder warm() knows): the step fails, never warms
/// nothing in silence (/code-review).
pub fn warm_chrome() -> Result<String, String> {
    let chrome = chrome().ok_or("no Chrome to warm")?;
    let started = Instant::now();
    let bytes = warm(&chrome);
    if bytes == 0 {
        return Err(format!(
            "{chrome}: no install folder read (none beside it holds icudtl.dat)"
        ));
    }
    Ok(format!(
        "{chrome}: {} MB of its install folder read in {:.1} s",
        bytes / 1_000_000,
        started.elapsed().as_secs_f64()
    ))
}

/// Chrome's install folder (the one beside its real binary, a folder holding icudtl.dat, never a bin folder of the system)
/// read through once, file by file, so a later start finds its files in the page cache. Errors are ignored: Chrome reads its
/// own files anyway. The bytes read.
fn warm(chrome: &str) -> u64 {
    let Some(dir) = Path::new(chrome)
        .canonicalize()
        .ok()
        .and_then(|c| c.parent().map(Path::to_path_buf))
    else {
        return 0;
    };
    if !dir.join("icudtl.dat").is_file() {
        return 0;
    }
    std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|file| file.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|file| std::fs::File::open(file.path()).ok())
        .map(|mut f| std::io::copy(&mut f, &mut std::io::sink()).unwrap_or(0))
        .sum()
}

/// Ends Chrome's process group (its own, from process_group(0)) and reaps it. killpg, never a shelled `kill -KILL -<pgid>`:
/// Ubuntu 24.04's procps kill read that pgid by its first digit, so a pgid starting with 1 became kill(-1), every process of
/// the user, the Actions runner's agent with them: PR 1444's mirror runs hung to the job's 20 minutes, their logs lost
/// (2026-11-11; procps issue 354). dash-gate's no_group_is_ended_by_a_shelled_kill holds it.
fn end_group(child: &mut std::process::Child) {
    // SAFETY: killpg reads no memory. It reaches the group's members only: a helper that left it (setsid) is not ended here,
    // hence --disable-crash-reporter, which keeps crashpad's handler from starting at all (/code-review). It fails only on a
    // group already gone (the leader exited, so the wait below returns at once); EPERM cannot arise for our own child.
    unsafe { libc::killpg(child.id() as libc::pid_t, libc::SIGKILL) };
    let _ = child.wait();
}

/// A PDF written to its end: its header, and its %%EOF trailer last.
fn whole(pdf: &[u8]) -> bool {
    pdf.starts_with(b"%PDF-") && pdf.trim_ascii_end().ends_with(b"%%EOF")
}

/// The last lines Chrome or the slot wrote to stderr, for the answer that says why no PDF came.
fn said(log: &Path) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().rev().take(5).collect();
    match lines.is_empty() {
        true => String::new(),
        false => format!(
            ": {}",
            lines.into_iter().rev().collect::<Vec<_>>().join(" / ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A fake home folder, empty, unique to `name`.
    fn home(name: &str) -> std::path::PathBuf {
        let h = std::env::temp_dir().join(format!("dash-pdf-chrome-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&h);
        std::fs::create_dir_all(&h).unwrap();
        h
    }

    /// A fake Chrome, `script` at a path of its own kept between runs, rewritten only when its text changes, and run once on
    /// its way in (FAKE_WARM set, it exits at once): macOS checks a new executable on its first run, 3 s and past 5 under a
    /// build, which ate this file's 1 s limit before the fake said a word (2026-11-12; rust/CLAUDE.md rule 1). Written beside
    /// and renamed into place, so a test of another checkout running the old text never reads half a script (/code-review).
    fn fake(name: &str, script: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("dash-pdf-fake-chrome-{name}"));
        let script = format!("#!/bin/sh\n[ -z \"$FAKE_WARM\" ] || exit 0\n{script}");
        if std::fs::read_to_string(&path).ok() != Some(script.clone()) {
            let new = path.with_extension(std::process::id().to_string());
            std::fs::write(&new, &script).unwrap();
            std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755)).unwrap();
            std::fs::rename(&new, &path).unwrap();
        }
        let warmed = std::process::Command::new(&path)
            .env("FAKE_WARM", "1")
            .status();
        assert!(
            warmed.as_ref().is_ok_and(|s| s.success()),
            "{}: {warmed:?}",
            path.display()
        );
        path
    }

    /// An empty file at `home`/`rel`, standing for a browser binary.
    fn bin(home: &Path, rel: &str) -> String {
        let p = home.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.to_string_lossy().into_owned()
    }

    #[dash_meter::dash_test]
    fn chrome_prefers_headless_shell() {
        let h = home("prefers");
        let pup = ".cache/puppeteer/chrome-headless-shell/mac_arm";
        let pw = "Library/Caches/ms-playwright";
        let playwright = bin(
            &h,
            &format!("{pw}/chromium_headless_shell-1234/{SHELL_FOLDER}/chrome-headless-shell"),
        );
        bin(
            &h,
            &format!("{pw}/chromium_headless_shell-999/{SHELL_FOLDER}/chrome-headless-shell"),
        );
        assert_eq!(
            chrome_from(None, Some(h.clone()), &CHROMES),
            Some(playwright),
            "playwright's highest revision, with no puppeteer"
        );
        bin(
            &h,
            &format!("{pup}-99.0.1.2/{SHELL_FOLDER}/chrome-headless-shell"),
        );
        let newest = bin(
            &h,
            &format!("{pup}-152.0.7977.54/{SHELL_FOLDER}/chrome-headless-shell"),
        );
        bin(
            &h,
            &format!("{pup}-143.0.7499.192/{SHELL_FOLDER}/chrome-headless-shell"),
        );
        // an install with no binary yet (a download cut short) is passed over
        std::fs::create_dir_all(h.join(format!("{pup}-160.0.0.1/chrome-headless-shell-mac-arm64")))
            .unwrap();
        assert_eq!(
            chrome_from(None, Some(h.clone()), &CHROMES),
            Some(newest.clone()),
            "puppeteer's 152, by version not by text, before playwright"
        );
        // another platform's install, newer, and this platform's with no execute bit (an unzip cut short), are passed over
        bin(
            &h,
            &format!("{pup}-170.0.0.1/chrome-headless-shell-win64/chrome-headless-shell"),
        );
        let unrunnable = bin(
            &h,
            &format!("{pup}-165.0.0.1/{SHELL_FOLDER}/chrome-headless-shell"),
        );
        std::fs::set_permissions(&unrunnable, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            chrome_from(None, Some(h.clone()), &CHROMES),
            Some(newest),
            "this platform's runnable 152 still"
        );
        let _ = std::fs::remove_dir_all(&h);
    }

    #[dash_meter::dash_test]
    fn a_print_ends_once_its_pdf_is_whole() {
        let h = home("hangs");
        // a Chrome that writes its PDF, then never exits, as chrome-headless-shell 152 did under a full run
        let fake = fake(
            "prints",
            "for a; do case $a in --print-to-pdf=*) o=${a#--print-to-pdf=};; esac; done\n\
             printf '%%PDF-1.4 fake\\n%%%%EOF\\n' > \"$o\"; exec sleep 600\n",
        );
        let dir = h.join("print");
        std::fs::create_dir_all(&dir).unwrap();
        let started = Instant::now();
        // no slot: a fake Chrome takes none of the machine's, nor waits on one
        let pdf = printed_in(
            &dir,
            "<p>x</p>",
            &fake.to_string_lossy(),
            &h.join("no-slot"),
            LIMIT,
        );
        assert_eq!(
            pdf.as_deref(),
            Ok(&b"%PDF-1.4 fake\n%%EOF\n"[..]),
            "the PDF Chrome wrote"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "ended once written, not at the limit: {:?}",
            started.elapsed()
        );
        let _ = std::fs::remove_dir_all(&h);
    }

    #[dash_meter::dash_test]
    fn a_chrome_that_never_prints_is_ended_at_the_limit_and_the_answer_holds_its_words() {
        let h = home("stalls");
        // a Chrome that starts, says a line and never writes its PDF, as the CI mirror's did
        let fake = fake("stalls", "echo stalled >&2; exec sleep 600\n");
        let dir = h.join("print");
        std::fs::create_dir_all(&dir).unwrap();
        let started = Instant::now();
        let answer = printed_in(
            &dir,
            "<p>x</p>",
            &fake.to_string_lossy(),
            &h.join("no-slot"),
            Duration::from_secs(1),
        );
        assert_eq!(
            answer,
            Err("Chrome did not print in 1 s, a browser slot's wait included: stalled".into())
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "ended at its limit: {:?}",
            started.elapsed()
        );
        assert!(
            LIMIT <= Duration::from_secs(20),
            "the limit leaves a test 10 s of nextest's 30 to start and to read why"
        );
        let _ = std::fs::remove_dir_all(&h);
    }

    #[dash_meter::dash_test]
    fn the_warm_up_reads_a_chrome_install_folder_and_nothing_else() {
        let h = home("warm");
        let chrome = bin(&h, "install/chrome");
        std::fs::write(h.join("install/resources.pak"), "12345").unwrap();
        assert_eq!(
            warm(&chrome),
            0,
            "no icudtl.dat: a folder such as /usr/bin, never read"
        );
        std::fs::write(h.join("install/icudtl.dat"), "123").unwrap();
        std::fs::create_dir_all(h.join("install/locales")).unwrap();
        std::fs::write(h.join("install/locales/fr.pak"), "1234567").unwrap();
        assert_eq!(
            warm(&chrome),
            8,
            "the install folder's own files, not its subfolders"
        );
        // a link, as /usr/bin/google-chrome to /opt/google/chrome: the folder of the file it names
        let link = h.join("google-chrome");
        std::os::unix::fs::symlink(&chrome, &link).unwrap();
        assert_eq!(
            warm(&link.to_string_lossy()),
            8,
            "through a link, its target's folder"
        );
        assert_eq!(warm(&h.join("gone").to_string_lossy()), 0, "no such Chrome");
        let _ = std::fs::remove_dir_all(&h);
    }

    #[dash_meter::dash_test]
    fn chrome_on_linux_keeps_google_chrome() {
        // CI's Linux runner: no headless shell in its caches, google-chrome at its path, Chrome.app nowhere
        let h = home("linux");
        let app = h.join("Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
        let google = bin(&h, "usr/bin/google-chrome");
        let chromium = bin(&h, "usr/bin/chromium");
        let chromes = [app.to_str().unwrap(), &google, &chromium];
        assert_eq!(
            chrome_from(None, Some(h.clone()), &chromes),
            Some(google.clone()),
            "no shell and no Chrome.app: google-chrome, the first of CHROMES there is"
        );
        assert_eq!(
            chrome_from(Some("/no/such/chrome".into()), Some(h.clone()), &chromes),
            Some(google),
            "a $CHROME not there leaves google-chrome too"
        );
        assert_eq!(
            CHROMES[1], "/usr/bin/google-chrome",
            "CHROMES holds google-chrome's path on Linux, right after Chrome.app"
        );
        let _ = std::fs::remove_dir_all(&h);
    }

    #[dash_meter::dash_test]
    fn chrome_skips_missing_env() {
        let h = home("env");
        let shell = bin(
            &h,
            &format!(
                ".cache/puppeteer/chrome-headless-shell/linux-152.0.7977.54/{SHELL_FOLDER}/chrome-headless-shell"
            ),
        );
        let gone = h.join("no-such-chrome").to_string_lossy().into_owned();
        assert_eq!(
            chrome_from(Some(gone), Some(h.clone()), &CHROMES),
            Some(shell.clone()),
            "a $CHROME not there falls through to the shell"
        );
        let mine = bin(&h, "my-chrome");
        assert_eq!(
            chrome_from(Some(mine.clone()), Some(h.clone()), &CHROMES),
            Some(mine),
            "a $CHROME that is there wins"
        );
        let sh = chrome_from(Some("sh".into()), Some(h.clone()), &CHROMES).unwrap();
        assert!(
            sh.ends_with("/sh"),
            "a bare name is found on $PATH, as Command would: {sh}"
        );
        let empty = home("env-empty");
        let fallback = CHROMES.iter().map(|c| c.to_string()).find_map(on_disk);
        assert_eq!(
            chrome_from(
                Some("/no/such/chrome".into()),
                Some(empty.clone()),
                &CHROMES
            ),
            fallback,
            "with no shell, the first of CHROMES there is"
        );
        let _ = std::fs::remove_dir_all(&h);
        let _ = std::fs::remove_dir_all(&empty);
    }
}
