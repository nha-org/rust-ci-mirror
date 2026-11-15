//! The page-comments extension end to end (task page-comments-extension, rfc-dashboard 3.3), the port of
//! test/scripts/extension-check (task chrome-mock-keychain-all; @nha, 2026-11-14: « I get a bunch of popups chrome for
//! testing wants confidential storage information. Why ? »). Chrome for Testing, which still loads an unpacked extension,
//! starts with --use-mock-keychain and --password-store=basic, so it never asks his keychain. A copy of
//! private/dashboard/extension posts to a stand-in app here, allowed on every page; on a page that stand-in serves, the
//! picker picks an element, a comment is typed and sent, then the page is captured: both reach the stand-in and the outbox
//! empties. The CI mirror carries rust/ alone, no extension: there dash_meter::monorepo() stops the test.
use dash_meter::dash_test;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The flags that keep Chrome off his keychain: dash-gate's no-keychain rule refuses a headless launch without the first.
const KEYCHAIN_OFF: [&str; 2] = ["--use-mock-keychain", "--password-store=basic"];

/// The newest Chrome for Testing: $CHROME_FOR_TESTING, else puppeteer's (~/.cache/puppeteer/chrome/<platform>-<version>),
/// else playwright's (ms-playwright/chromium-<revision>), each by its number.
fn chrome_for_testing() -> Option<PathBuf> {
    if let Some(c) = std::env::var_os("CHROME_FOR_TESTING") {
        return Some(c.into());
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let app = "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing";
    let newest = |dir: PathBuf, prefix: &str, inside: &str| {
        let mut found: Vec<(Vec<u64>, PathBuf)> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let n = name.strip_prefix(prefix)?;
                let key = n
                    .split('.')
                    .map(|p| p.parse().ok())
                    .collect::<Option<_>>()?;
                let bin = e.path().join(inside).join(app);
                bin.is_file().then_some((key, bin))
            })
            .collect();
        found.sort();
        found.pop().map(|(_, p)| p)
    };
    newest(
        home.join(".cache/puppeteer/chrome"),
        "mac_arm-",
        "chrome-mac-arm64",
    )
    .or_else(|| {
        newest(
            home.join("Library/Caches/ms-playwright"),
            "chromium-",
            "chrome-mac-arm64",
        )
    })
}

/// The stand-in app: GET answers the demo page, POST records its body and answers 200.
fn stand_in() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let posts: Arc<Mutex<Vec<String>>> = Arc::default();
    let seen = posts.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let seen = seen.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(&stream);
                let (mut head, mut len) = (String::new(), 0usize);
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                    head.push_str(&line);
                }
                let mut body = vec![0; len];
                let _ = r.read_exact(&mut body);
                let page = "<!doctype html><title>Demo app</title><main id=main><p>First</p>\
                            <p class=target>Second paragraph</p></main>";
                let answer = match head.starts_with("POST") {
                    true => {
                        seen.lock()
                            .unwrap()
                            .push(String::from_utf8_lossy(&body).into());
                        String::new()
                    }
                    false => page.to_string(),
                };
                let _ = (&stream).write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                        answer.len()
                    )
                    .as_bytes(),
                );
            });
        }
    });
    (port, posts)
}

/// Chrome over its DevTools pipe (fds 3 and 4), as notion_browser.rs and apply.rs drive one, a session per target.
struct Chrome {
    child: Child,
    to: ChildStdin,
    from: BufReader<ChildStdout>,
    id: u64,
}

impl Chrome {
    fn send(&mut self, session: Option<&str>, method: &str, params: Value) -> Value {
        self.id += 1;
        let mut msg = json!({"id": self.id, "method": method, "params": params});
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        self.to.write_all(msg.to_string().as_bytes()).unwrap();
        self.to.write_all(b"\0").unwrap();
        loop {
            let mut raw = Vec::new();
            assert!(
                self.from.read_until(0, &mut raw).unwrap() > 0,
                "Chrome ended during {method}"
            );
            raw.pop();
            let answer: Value = serde_json::from_slice(&raw).unwrap();
            if answer["id"] == json!(self.id) {
                assert!(answer.get("error").is_none(), "{method}: {answer}");
                return answer["result"].clone();
            }
        }
    }

    fn eval(&mut self, session: &str, js: &str) -> Value {
        let r = self.send(
            Some(session),
            "Runtime.evaluate",
            json!({"expression": js, "returnByValue": true, "awaitPromise": true}),
        );
        assert!(r.get("exceptionDetails").is_none(), "{js}: {r}");
        r["result"]["value"].clone()
    }

    /// `js` once truthy, asked every 100 ms for 10 s at most.
    fn until(&mut self, session: &str, js: &str) -> Value {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let v = self.eval(session, js);
            if !(v.is_null() || v == json!(false) || v == json!([])) {
                return v;
            }
            assert!(Instant::now() < end, "never held: {js}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn attach(&mut self, target: &Value) -> String {
        let a = self.send(
            None,
            "Target.attachToTarget",
            json!({"targetId": target, "flatten": true}),
        );
        a["sessionId"].as_str().unwrap().to_string()
    }
}

impl Drop for Chrome {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A copy of the extension in `dir` posting to `app`, allowed on every page, its capture reachable from the test.
fn extension(top: &Path, dir: &Path, app: &str) -> PathBuf {
    let src = top.join("private/dashboard/extension");
    let ext = dir.join("ext");
    std::fs::create_dir_all(&ext).unwrap();
    // every module the extension ships, never a list: policy.js came after one and the worker failed its import
    for f in std::fs::read_dir(&src).unwrap() {
        let p = f.unwrap().path();
        let name = p.file_name().unwrap().to_str().unwrap().to_owned();
        if name.ends_with(".js") || name == "manifest.json" {
            std::fs::copy(&p, ext.join(name)).unwrap();
        }
    }
    let mut m: Value =
        serde_json::from_str(&std::fs::read_to_string(src.join("manifest.json")).unwrap()).unwrap();
    m["host_permissions"] = json!(["<all_urls>"]);
    std::fs::write(ext.join("manifest.json"), m.to_string()).unwrap();
    let outbox = ext.join("outbox.js");
    let text = std::fs::read_to_string(&outbox).unwrap();
    assert!(
        text.contains("http://100.72.158.115:7788"),
        "outbox.js's endpoint moved"
    );
    std::fs::write(&outbox, text.replace("http://100.72.158.115:7788", app)).unwrap();
    let bg = ext.join("background.js");
    let text = std::fs::read_to_string(&bg).unwrap();
    std::fs::write(&bg, format!("{text}\nself.capture = capture;\n")).unwrap();
    ext
}

// Browser needed: an unpacked extension's service worker, its picker and its outbox run only in a real Chrome.
#[dash_test]
fn a_comment_and_a_capture_leave_the_extension_s_outbox_in_chrome_for_testing_off_his_keychain() {
    let Some(top) = dash_meter::monorepo() else {
        return;
    };
    let bin = chrome_for_testing()
        .expect("no Chrome for Testing: npx @puppeteer/browsers install chrome@stable, or set CHROME_FOR_TESTING");
    // one of the machine's browser slots, held until this test's process ends (bin/browser-slot)
    let slot = top.join("bin/browser-slot");
    let taken = Command::new(&slot)
        .arg(std::process::id().to_string())
        .status()
        .expect("bin/browser-slot");
    assert!(taken.success(), "bin/browser-slot: {taken}");
    let dir = dash_meter::TestDir::new("extension-browser");
    let (port, posts) = stand_in();
    let ext = extension(&top, &dir.0, &format!("http://127.0.0.1:{port}"));
    let page_url = format!("http://localhost:{port}/index.html");

    let mut child = Command::new("/bin/sh")
        .args([
            "-c",
            r#"exec "$0" "$@" 3<&0 4>&1 </dev/null >/dev/null 2>/dev/null"#,
        ])
        .arg(&bin)
        .args([
            "--headless",
            "--remote-debugging-pipe",
            "--no-sandbox",
            // without it the picker's screenshot (captureVisibleTab) took 9.5 s in headless Chrome for Testing 152
            "--disable-gpu",
            // Chrome answers Browser.getBrowserCommandLine only with it
            "--enable-automation",
        ])
        .args(KEYCHAIN_OFF)
        .args([
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-sync",
            "--disable-crash-reporter",
        ])
        // Chrome 137 on ignores --load-extension: the extension loads over the pipe (Extensions.loadUnpacked) with this
        .arg("--enable-unsafe-extension-debugging")
        .arg(format!(
            "--user-data-dir={}",
            dir.0.join("profile").display()
        ))
        .arg("about:blank")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    // past 25 s Chrome is killed, so a hung read ends and names its command inside nextest's 30 s
    let pid = child.id().to_string();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(25));
        let _ = Command::new("kill").args(["-KILL", &pid]).status();
    });
    let mut c = Chrome {
        to: child.stdin.take().unwrap(),
        from: BufReader::new(child.stdout.take().unwrap()),
        child,
        id: 0,
    };
    // Chrome itself runs with both flags: its own command line, read back
    let argv = c.send(None, "Browser.getBrowserCommandLine", json!({}));
    for flag in KEYCHAIN_OFF {
        assert!(
            argv["arguments"].as_array().unwrap().contains(&json!(flag)),
            "{flag} not in {argv}"
        );
    }

    let target = c.send(None, "Target.createTarget", json!({"url": page_url}));
    let page = c.attach(&target["targetId"]);
    c.until(
        &page,
        "document.readyState === 'complete' && !!document.querySelector('p.target')",
    );
    let id = c.send(None, "Extensions.loadUnpacked", json!({"path": ext}))["id"].clone();
    let worker_url = format!("chrome-extension://{}/background.js", id.as_str().unwrap());
    let end = Instant::now() + Duration::from_secs(10);
    // the worker as Chrome first lists it may run before its extension APIs are bound: attached again until they are
    let sw = loop {
        let t = c.send(None, "Target.getTargets", json!({}));
        let found = t["targetInfos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["url"] == json!(worker_url))
            .cloned();
        if let Some(w) = found {
            let session = c.attach(&w["targetId"]);
            if c.eval(&session, "!!globalThis.chrome?.scripting") == json!(true) {
                break session;
            }
        }
        assert!(Instant::now() < end, "{worker_url} never started: {t}");
        std::thread::sleep(Duration::from_millis(100));
    };
    let tab = format!(
        "(async () => (await chrome.tabs.query({{}})).find(t => t.url === {}))()",
        json!(page_url)
    );
    c.eval(
        &sw,
        &format!(
            "(async () => {{ const t = await {tab}; \
             await chrome.scripting.executeScript({{target: {{tabId: t.id}}, files: ['picker.js']}}); }})()"
        ),
    );
    c.until(&page, "!!document.getElementById('dash-pick-ring')");
    let at = c.eval(
        &page,
        "(() => { const r = document.querySelector('p.target').getBoundingClientRect(); \
         return [r.x + r.width / 2, r.y + r.height / 2]; })()",
    );
    let (x, y) = (at[0].as_f64().unwrap(), at[1].as_f64().unwrap());
    c.send(
        Some(&page),
        "Input.dispatchMouseEvent",
        json!({"type": "mouseMoved", "x": x, "y": y}),
    );
    for kind in ["mousePressed", "mouseReleased"] {
        c.send(
            Some(&page),
            "Input.dispatchMouseEvent",
            json!({"type": kind, "x": x, "y": y, "button": "left", "clickCount": 1}),
        );
    }
    c.until(
        &page,
        "document.activeElement === document.querySelector('#dash-pick-panel textarea')",
    );
    c.send(
        Some(&page),
        "Input.insertText",
        json!({"text": "This paragraph is too long"}),
    );
    c.eval(
        &page,
        "document.querySelector('#dash-pick-panel button[data-a=send]').click()",
    );
    c.eval(&sw, &format!("(async () => self.capture(await {tab}))()"));

    // both queued, then flushed: an outbox read before the capture's enqueue is no verdict, so the posts are waited too
    let left = c.until(
        &sw,
        "(async () => { const s = await chrome.storage.local.get(['outbox', 'refused']); \
         return s.outbox && s.outbox.length === 0 ? s : null; })()",
    );
    let end = Instant::now() + Duration::from_secs(10);
    while posts.lock().unwrap().len() < 2 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(100));
    }
    let posts = posts.lock().unwrap().clone();
    assert_eq!(
        left["refused"],
        json!([]),
        "the stand-in refused one: {left}"
    );
    assert_eq!(
        posts.len(),
        2,
        "two posts, a comment and a capture: {posts:?}"
    );
    let comment = posts
        .iter()
        .find(|p| p.contains("This paragraph is too long"));
    let comment = comment.unwrap_or_else(|| panic!("no comment posted: {posts:?}"));
    assert!(
        comment.contains("Second paragraph"),
        "the comment quotes its element: {comment}"
    );
    assert!(
        posts
            .iter()
            .any(|p| p.contains("capture") && p.contains("page.html")),
        "no capture posted: {posts:?}"
    );
}
