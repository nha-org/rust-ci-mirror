//! The page-comments extension end to end (task page-comments-extension, rfc-dashboard 3.3), the port of
//! test/scripts/extension-check (task chrome-mock-keychain-all; @nha, 2026-11-14: « I get a bunch of popups chrome for
//! testing wants confidential storage information. Why ? »). Chrome for Testing, which still loads an unpacked extension,
//! starts with --use-mock-keychain and --password-store=basic, so it never asks his keychain. A copy of
//! private/dashboard/extension posts to a stand-in app here, allowed on every page; on a page that stand-in serves, the
//! picker picks an element, a comment is typed and sent, then the page is captured: both reach the stand-in and the outbox
//! empties. Then, on demo.test (the stand-in under a name his capture policy gives a HAR), « Send HAR » in the DevTools
//! panel sends nothing with the switch off, then posts the page's HAR gzipped with its bodies (task extension-har-panel).
//! Then « Save a login »'s form posts its site, user and password at once, and nothing of it stays in the extension's storage
//! (task extension-login-form). Then the app serves another version, and the extension reloads itself (task extension-self-reload). The CI mirror carries rust/ alone, no extension: there dash_meter::monorepo() stops the test.
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

/// The posts the stand-in took, and the version it serves.
type Shared = (Arc<Mutex<Vec<Vec<u8>>>>, Arc<Mutex<String>>);

/// The stand-in app: GET /extension/version answers its version, any other GET the demo page, POST records its body and
/// answers 200.
fn stand_in() -> (u16, Shared) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let posts: Arc<Mutex<Vec<Vec<u8>>>> = Arc::default();
    let version = Arc::new(Mutex::new("v1".to_string()));
    let (seen, served) = (posts.clone(), version.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (seen, served) = (seen.clone(), served.clone());
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
                        seen.lock().unwrap().push(body);
                        String::new()
                    }
                    // his policy, a HAR on the stand-in's own name (demo.test, mapped to it by Chrome's resolver)
                    false if head.starts_with("GET /capture/policy ") => json!({"every_page":
                        ["comment", "screenshot", "html"], "har": ["demo.test"], "unset": []})
                    .to_string(),
                    false if head.starts_with("GET /extension/version ") => {
                        json!({"version": *served.lock().unwrap()}).to_string()
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
    (port, (posts, version))
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
        let answer = self.ask(session, method, params);
        answer.unwrap_or_else(|e| panic!("{method}: {e}"))
    }

    /// `send`, its error returned: a target that goes away (a worker reloading) answers one.
    fn ask(&mut self, session: Option<&str>, method: &str, params: Value) -> Result<Value, Value> {
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
                return match answer.get("error") {
                    Some(e) => Err(e.clone()),
                    None => Ok(answer["result"].clone()),
                };
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

/// A session on the extension's service worker at `url` once `holds` is true there, asked every 100 ms for 10 s at most.
/// The worker as Chrome first lists it may run before its extension APIs are bound, and one reloading goes away under
/// its session: either is attached again, so its errors are tried again too. `skip`, a worker going away, is never asked:
/// it never answers an evaluate.
fn worker(c: &mut Chrome, url: &str, holds: &str, skip: Option<&Value>) -> (Value, String) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        let t = c.send(None, "Target.getTargets", json!({}));
        let found = t["targetInfos"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["url"] == json!(url))
            .filter(|t| Some(&t["targetId"]) != skip)
            .cloned()
            .collect::<Vec<_>>();
        for w in found {
            let Ok(a) = c.ask(
                None,
                "Target.attachToTarget",
                json!({"targetId": w["targetId"], "flatten": true}),
            ) else {
                continue;
            };
            let session = a["sessionId"].as_str().unwrap().to_string();
            let asked = c.ask(
                Some(&session),
                "Runtime.evaluate",
                json!({"expression": holds, "returnByValue": true, "awaitPromise": true}),
            );
            if asked.is_ok_and(|r| r["result"]["value"] == json!(true)) {
                return (w["targetId"].clone(), session);
            }
        }
        assert!(Instant::now() < end, "{url} never held {holds}: {t}");
        std::thread::sleep(Duration::from_millis(100));
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
        if dash_comments::extension::shipped(&name) {
            std::fs::copy(&p, ext.join(name)).unwrap();
        }
    }
    let mut m: Value =
        serde_json::from_str(&std::fs::read_to_string(src.join("manifest.json")).unwrap()).unwrap();
    let granted = m["host_permissions"].to_string();
    m["host_permissions"] = json!(["<all_urls>"]);
    std::fs::write(ext.join("manifest.json"), m.to_string()).unwrap();
    let outbox = ext.join("outbox.js");
    let text = std::fs::read_to_string(&outbox).unwrap();
    assert!(
        text.contains("http://100.72.158.115:7788"),
        "outbox.js's endpoint moved"
    );
    std::fs::write(&outbox, text.replace("http://100.72.158.115:7788", app)).unwrap();
    // the login form posts to Tailscale Serve's address, here the stand-in too
    let form = ext.join("login.js");
    let text = std::fs::read_to_string(&form).unwrap();
    // read from login.js, never named under rust/, which the public mirror publishes (/code-review)
    let serve = (text
        .split_once("export const LOGIN = \"")
        .and_then(|(_, r)| r.split_once("/extension/login\"")))
    .map(|(origin, _)| origin.to_string())
    .expect("login.js's LOGIN moved");
    // the manifest grants the address login.js posts to: renamed in one alone, the post fails (/code-review of #1676)
    assert!(
        granted.contains(&format!("{serve}/*")),
        "manifest.json's host_permissions lacks {serve}"
    );
    std::fs::write(&form, text.replace(&serve, app)).unwrap();
    let bg = ext.join("background.js");
    let text = std::fs::read_to_string(&bg).unwrap();
    std::fs::write(
        &bg,
        format!("{text}\nself.capture = capture;\nself.update = update;\n"),
    )
    .unwrap();
    ext
}

// Browser needed: an unpacked extension's service worker, its picker and its outbox run only in a real Chrome.
#[dash_test]
fn a_comment_a_capture_a_har_and_a_login_leave_nothing_in_the_extension_in_chrome_for_testing_off_his_keychain()
 {
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
    let (port, (posts, version)) = stand_in();
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
            // the HAR panel's page: demo.test is the stand-in, a host the capture policy takes, as no IP or localhost is
            "--host-resolver-rules=MAP demo.test 127.0.0.1",
            // DevTools open on each new tab, as he opens them, so the extension's panel exists
            "--auto-open-devtools-for-tabs",
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

    // Developer mode on, as in his Chrome (rfc-chrome-extension section 5): off, Chrome disables an unpacked extension that
    // reloads itself (DISABLE_UNSUPPORTED_DEVELOPER_EXTENSION). Turned on as the page's toggle does
    let settings = c.send(
        None,
        "Target.createTarget",
        json!({"url": "chrome://extensions"}),
    );
    let settings_target = settings["targetId"].clone();
    let settings = c.attach(&settings_target);
    c.until(&settings, "!!globalThis.chrome?.developerPrivate");
    let on = c.eval(
        &settings,
        "new Promise(r => chrome.developerPrivate.updateProfileConfiguration({inDeveloperMode: true}, r))\
         .then(() => new Promise(r => chrome.developerPrivate.getProfileConfiguration(r))).then(p => p.inDeveloperMode)",
    );
    assert_eq!(on, json!(true), "Developer mode stayed off");
    c.send(
        None,
        "Target.closeTarget",
        json!({"targetId": settings_target}),
    );
    let target = c.send(None, "Target.createTarget", json!({"url": page_url}));
    let page = c.attach(&target["targetId"]);
    c.until(
        &page,
        "document.readyState === 'complete' && !!document.querySelector('p.target')",
    );
    let id = c.send(None, "Extensions.loadUnpacked", json!({"path": ext}))["id"].clone();
    let worker_url = format!("chrome-extension://{}/background.js", id.as_str().unwrap());
    let (old, sw) = worker(&mut c, &worker_url, "!!globalThis.chrome?.scripting", None);
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
    let got: Vec<String> = (posts.lock().unwrap().iter())
        .map(|b| String::from_utf8_lossy(b).into())
        .collect();
    assert_eq!(
        left["refused"],
        json!([]),
        "the stand-in refused one: {left}"
    );
    assert_eq!(got.len(), 2, "two posts, a comment and a capture: {got:?}");
    let comment = got
        .iter()
        .find(|p| p.contains("This paragraph is too long"));
    let comment = comment.unwrap_or_else(|| panic!("no comment posted: {got:?}"));
    assert!(
        comment.contains("Second paragraph"),
        "the comment quotes its element: {comment}"
    );
    assert!(
        got.iter()
            .any(|p| p.contains("capture") && p.contains("page.html")),
        "no capture posted: {got:?}"
    );
    // The HAR panel (task extension-har-panel): on demo.test, a host the stand-in's policy gives a HAR, with DevTools open,
    // « Send HAR » in the Dashboard panel sends nothing while the « Capture HAR » switch is off, then posts the page's HAR
    // gzipped, the page's own body in it.
    c.until(
        &sw,
        "(async () => (await chrome.storage.local.get('policy')).policy?.har?.includes('demo.test'))()",
    );
    let har_url = format!("http://demo.test:{port}/index.html");
    let har_tab = c.send(None, "Target.createTarget", json!({"url": har_url}))["targetId"].clone();
    let tab_id = c.until(
        &sw,
        &format!(
            "(async () => (await chrome.tabs.query({{}})).find(t => t.url === {})?.id)()",
            json!(har_url)
        ),
    );
    let base = format!("chrome-extension://{}", id.as_str().unwrap());
    // the extension's DevTools page in that tab's DevTools, then its Dashboard tab shown, as his click on it does
    let (frame, _) = worker(
        &mut c,
        &format!("{base}/devtools.html"),
        &format!("chrome.devtools.inspectedWindow.tabId === {tab_id}"),
        None,
    );
    let targets = c.send(None, "Target.getTargets", json!({}));
    let tools = (targets["targetInfos"].as_array().unwrap().iter())
        .find(|t| t["targetId"] == frame)
        .map(|t| t["parentId"].clone())
        .unwrap();
    let tools = c.attach(&tools);
    c.eval(
        &tools,
        &format!(
            "import('./ui/legacy/legacy.js').then(L => L.InspectorView.InspectorView.instance().showPanel({}))",
            json!(format!("{base}Dashboard"))
        ),
    );
    let (_, panel) = worker(
        &mut c,
        &format!("{base}/panel.html"),
        "!!document.getElementById('send')",
        None,
    );
    // what was loaded before DevTools listened may be missing: loaded again, as the panel tells him
    let tab = c.attach(&har_tab);
    c.send(Some(&tab), "Page.reload", json!({}));
    c.until(
        &tab,
        "document.readyState === 'complete' && !!document.querySelector('p.target')",
    );
    let said = "document.getElementById('said').textContent";
    c.eval(&panel, "document.getElementById('send').click()");
    let off = c.until(&panel, &format!("{said}.startsWith('Not sent') && {said}"));
    assert!(off.as_str().unwrap().contains("Capture HAR"), "{off}");
    // u-bodies: getHAR() leaves the page's body out, each entry carrying getContent() instead, which the panel calls
    // (rfc-chrome-extension's u-bodies row; Chrome for Testing 152)
    let bodies = c.eval(
        &panel,
        &format!(
            "new Promise(r => chrome.devtools.network.getHAR(r)).then(h => h.entries.filter(e => e.request.url === {}) \
             .map(e => [e.response.content.text ?? null, typeof e.getContent]))",
            json!(har_url)
        ),
    );
    assert_eq!(
        bodies,
        json!([[null, "function"]]),
        "getHAR() changed: rfc-chrome-extension's u-bodies row is to write again"
    );
    c.eval(&sw, "chrome.storage.local.set({capturing: true})");
    c.eval(&panel, "document.getElementById('send').click()");
    let queued = c.until(&panel, &format!("{said}.startsWith('Queued') && {said}"));
    let end = Instant::now() + Duration::from_secs(10);
    while posts.lock().unwrap().len() < 3 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(100));
    }
    let post = posts
        .lock()
        .unwrap()
        .get(2)
        .cloned()
        .unwrap_or_else(|| panic!("no HAR posted: {queued}"));
    let text = String::from_utf8_lossy(&post);
    assert!(
        text.contains("name=\"tag\"\r\n\r\nhar\r\n"),
        "tagged har: {text}"
    );
    let at = |needle: &[u8], from: usize| {
        (post[from..].windows(needle.len()).position(|w| w == needle)).map(|i| i + from)
    };
    let file = at(b"filename=\"page.har.gz\"", 0).expect("page.har.gz posted");
    let start = at(b"\r\n\r\n", file).expect("its head") + 4;
    // the part ends at the form's own boundary, its first line, never at bytes of the gzip that only look like one
    let boundary = &post[..at(b"\r\n", 0).expect("a boundary")];
    let stop = at(&[b"\r\n", boundary].concat(), start).expect("its end");
    let gz = &post[start..stop];
    assert_eq!(gz[..2], [0x1f, 0x8b], "gzipped");
    // gunzipped by Chrome's own DecompressionStream, read as the HAR it is
    let hex: String = gz.iter().map(|b| format!("{b:02x}")).collect();
    let read = c.eval(
        &panel,
        &format!("(async () => {{ const b = Uint8Array.from({hex:?}.match(/../g), h => parseInt(h, 16)); \
         const t = await new Response(new Blob([b]).stream().pipeThrough(new DecompressionStream('gzip'))).text(); \
         const page = JSON.parse(t).log.entries.find(e => e.request.url === {}); \
         return page && page.response.content.text; }})()", json!(har_url)),
    );
    assert!(
        read.as_str()
            .is_some_and(|b| b.contains("Second paragraph")),
        "the page's entry holds its body: {read}"
    );
    // The login form (task extension-login-form, done-when 2): the site filled from the page it was opened on, the user and
    // password typed, Save posts all three at once; then the outbox is empty and no storage of the extension holds any of it.
    let login_url = format!("{base}/login.html?site=www.linkedin.com");
    let form_tab =
        c.send(None, "Target.createTarget", json!({"url": login_url}))["targetId"].clone();
    let form = c.attach(&form_tab);
    c.until(
        &form,
        "document.readyState === 'complete' && document.getElementById('site')?.value === 'www.linkedin.com'",
    );
    const PASSWORD: &str = "hunter-2-in-the-form";
    for (field, text) in [("user", "me@example.com"), ("password", PASSWORD)] {
        c.eval(
            &form,
            &format!("document.getElementById('{field}').focus()"),
        );
        c.send(Some(&form), "Input.insertText", json!({"text": text}));
    }
    c.eval(&form, "document.getElementById('save').click()");
    let said = "document.getElementById('said').textContent";
    let saved = c.until(&form, &format!("{said}.startsWith('Saved') && {said}"));
    // the post holding a password, whatever came before it (/code-review)
    let login_post = || {
        let posts = posts.lock().unwrap();
        (posts
            .iter()
            .map(|b| String::from_utf8_lossy(b).into_owned()))
        .find(|p| p.contains("name=\"password\""))
    };
    // the stand-in records a post before it answers, so « Saved » means it is there (/code-review of #1676)
    let post = login_post().unwrap_or_else(|| panic!("no login posted: {saved}"));
    for (name, value) in [
        ("site", "www.linkedin.com"),
        ("user", "me@example.com"),
        ("password", PASSWORD),
    ] {
        assert!(
            post.contains(&format!("name=\"{name}\"\r\n\r\n{value}\r\n")),
            "{name} in the one post: {post}"
        );
    }
    assert_eq!(
        c.eval(&form, "document.getElementById('password').value"),
        json!("")
    );
    let stored = c.eval(
        &sw,
        "(async () => JSON.stringify({local: await chrome.storage.local.get(null), \
         session: await chrome.storage.session.get(null)}))()",
    );
    let stored = stored.as_str().unwrap();
    let held: Value = serde_json::from_str(stored).unwrap();
    assert_eq!(
        held["local"]["outbox"],
        json!([]),
        "the outbox is empty: {stored}"
    );
    for typed in [PASSWORD, "me@example.com"] {
        assert!(
            !stored.contains(typed),
            "the extension's storage holds {typed}: {stored}"
        );
    }
    // what the scene proved, printed for the task's report (nextest --no-capture shows it)
    println!(
        "login scene: one post with site, user and password; outbox {}; storage.local and .session hold neither the user nor \
         the password: {stored}",
        held["local"]["outbox"]
    );
    c.send(None, "Target.closeTarget", json!({"targetId": form_tab}));
    // Self-reload: the worker checks the app's version every minute; the first one it read is its own, kept in
    // storage.session, which Chrome empties on a reload. Another version served, the next check reloads the folder, and
    // the new worker keeps the new one: seen there, it proves the reload.
    let alarm = c.eval(
        &sw,
        "(async () => (await chrome.alarms.get('flush'))?.periodInMinutes)()",
    );
    assert_eq!(alarm, json!(1), "checked every minute");
    let kept = "(async () => (await chrome.storage.session.get('loaded')).loaded)()";
    assert_eq!(c.until(&sw, kept), json!("v1"));
    c.eval(&sw, "self.marker = 1");
    *version.lock().unwrap() = "v2".into();
    // the alarm's own run, a minute away at best, is the same call; never awaited, since the reload ends the worker under
    // it and its answer never comes
    c.eval(&sw, "void self.update()");
    let (_, new) = worker(
        &mut c,
        &worker_url,
        "(async () => !self.marker && (await chrome.storage.session.get('loaded')).loaded === 'v2')()",
        Some(&old),
    );
    // the same version again: no second reload
    c.eval(&new, "self.marker = 2");
    c.eval(&new, "self.update()");
    assert_eq!(
        c.eval(&new, "self.marker"),
        json!(2),
        "a reload with no change"
    );
}
