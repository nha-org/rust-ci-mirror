//! A page's HTML form printed to PDF, a file to download: the Notion document's Export PDF (task notion-page-menu-export;
//! design::document_pdf). Headless Chrome prints it, under print media, so components.css's [data-print] rule keeps the
//! document alone. Chrome: $CHROME, else Chrome.app on a Mac or google-chrome on Linux, as the browser tests find it. It
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
/// wait, Chrome and its helpers) and the answer says why.
const LIMIT: Duration = Duration::from_secs(60);

/// The Chrome to start: $CHROME, else the first of CHROMES there is. The browser tests start the same one.
pub fn chrome() -> Option<String> {
    std::env::var("CHROME").ok().or_else(|| {
        CHROMES
            .iter()
            .find(|c| Path::new(c).exists())
            .map(|c| c.to_string())
    })
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
    let printed = printed_in(&dir, html);
    let _ = std::fs::remove_dir_all(&dir);
    printed
}

fn printed_in(dir: &Path, html: &str) -> Result<Vec<u8>, String> {
    let page = dir.join("page.html");
    let out = dir.join("page.pdf");
    std::fs::write(&page, html).map_err(|e| format!("{}: {e}", page.display()))?;
    let chrome = chrome().ok_or("no Chrome to print with: install Google Chrome or set CHROME")?;
    // One of the machine's browser slots (bin/browser-slot, the cap every browser site_perso starts shares), taken for the
    // shell's pid, which exec makes Chrome's: freed when Chrome ends. The CI mirror carries rust/ alone: no bin/ there.
    // ponytail: the path is the build's checkout, so a binary built in a worktree since removed prints with no slot; a
    // lookup from the data folder if dash ever runs away from its checkout.
    let slot = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../bin/browser-slot");
    let script = match slot.exists() {
        true => {
            r#"slot=$1; shift; "$slot" $$ >/dev/null || { echo "bin/browser-slot failed" >&2; exit 1; }; exec "$@""#
        }
        false => r#"shift; exec "$@""#,
    };
    let log = dir.join("chrome.log");
    let errors = std::fs::File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    let mut child = Command::new("/bin/sh")
        .args(["-c", script, "sh"])
        .arg(&slot)
        .arg(&chrome)
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
    let end = Instant::now() + LIMIT;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() > end {
            let _ = Command::new("kill")
                .args(["-KILL", &format!("-{}", child.id())])
                .status();
            let _ = child.wait();
            return Err(format!(
                "Chrome did not print in {} s, a browser slot's wait included{}",
                LIMIT.as_secs(),
                said(&log)
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let pdf = std::fs::read(&out)
        .map_err(|e| format!("Chrome ({status}) wrote no PDF: {e}{}", said(&log)))?;
    match pdf.starts_with(b"%PDF-") {
        true => Ok(pdf),
        false => Err(format!("Chrome ({status}) wrote no PDF{}", said(&log))),
    }
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
