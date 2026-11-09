//! /design in a real browser (task rust-design-fixes; @nha, 2026-11-09: « /design the menu is cut. », « /design?tab=notion
//! => interactions are not functional. »): the page as dash serves it, menu included, in headless Chrome, driven with real
//! mouse and key events over the DevTools pipe. One test per interaction: drag, comment, toggle, slash menu, and the menu's fit;
//! and the compass's pick on any element of a page, its sidebar included (task rust-compass-any-element).
//! A mail's frame on /mail/t/<thread> runs no script and loads nothing remote (task rust-mail-thread), each against a control that
//! does reach the same port.
//! Chrome: $CHROME, else Chrome.app on a Mac or google-chrome on the Linux runner; none fails the test, never skips it.
use dash_meter::memory::TestBudget;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

const CHROMES: [&str; 4] = [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
];

struct Browser {
    child: Child,
    to: ChildStdin,
    from: BufReader<ChildStdout>,
    id: u64,
    session: Option<String>,
    dir: PathBuf,
}

impl Browser {
    /// /design?tab=notion as served, open at 1280 by 900.
    fn notion(name: &str) -> Browser {
        let page = dash_surface::forms::with_menu(
            &dash_surface::design::page("notion", None),
            "/design",
            "tab=notion",
        );
        Browser::open(name, &page, "!!document.getElementById('no-block')")
    }

    /// `page` from a file, open at 1280 by 900 once loaded and `ready` holds.
    fn open(name: &str, page: &str, ready: &str) -> Browser {
        let dir =
            std::env::temp_dir().join(format!("notion-browser-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("page.html");
        std::fs::write(&file, page).unwrap();
        let bin = std::env::var("CHROME")
            .ok()
            .or_else(|| {
                CHROMES
                    .iter()
                    .find(|c| std::path::Path::new(c).exists())
                    .map(|c| c.to_string())
            })
            .expect("no Chrome: install Google Chrome or set CHROME");
        // One of the machine's browser slots, held until this test's process ends (bin/browser-slot, the cap every browser
        // site_perso starts shares). The CI mirror carries rust/ alone: no bin/, and a runner of its own.
        let slot =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../bin/browser-slot");
        if slot.exists() {
            let taken = Command::new(&slot)
                .arg(std::process::id().to_string())
                .status();
            assert!(taken.is_ok_and(|s| s.success()), "no browser slot");
        }
        // The pipe is fds 3 (its commands) and 4 (its answers): the shell hands it this process's two pipes there.
        let mut child = Command::new("/bin/sh")
            .args([
                "-c",
                r#"exec "$0" "$@" 3<&0 4>&1 </dev/null >/dev/null 2>/dev/null"#,
                &bin,
            ])
            .args([
                "--headless",
                "--remote-debugging-pipe",
                "--no-sandbox",
                "--disable-gpu",
                "--use-mock-keychain",
                "--password-store=basic",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-background-networking",
                "--disable-component-update",
                "--disable-sync",
                "--disable-crash-reporter",
                "--hide-scrollbars",
            ])
            .arg(format!("--user-data-dir={}", dir.join("profile").display()))
            .arg("about:blank")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let to = child.stdin.take().unwrap();
        let from = BufReader::new(child.stdout.take().unwrap());
        let mut b = Browser {
            child,
            to,
            from,
            id: 0,
            session: None,
            dir,
        };
        let target = b.send("Target.createTarget", json!({"url": "about:blank"}));
        let attached = b.send(
            "Target.attachToTarget",
            json!({"targetId": target["targetId"], "flatten": true}),
        );
        b.session = attached["sessionId"].as_str().map(str::to_string);
        b.send(
            "Emulation.setDeviceMetricsOverride",
            json!({"width": 1280, "height": 900, "deviceScaleFactor": 1, "mobile": false}),
        );
        b.send(
            "Page.navigate",
            json!({"url": format!("file://{}", file.display())}),
        );
        b.until(&format!("document.readyState === 'complete' && {ready}"));
        b
    }

    /// One command's result; the events in between are dropped.
    fn send(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let mut msg = json!({"id": self.id, "method": method, "params": params});
        if let Some(s) = &self.session {
            msg["sessionId"] = json!(s);
        }
        self.to.write_all(msg.to_string().as_bytes()).unwrap();
        self.to.write_all(b"\0").unwrap();
        self.to.flush().unwrap();
        loop {
            let mut raw = Vec::new();
            assert!(
                self.from.read_until(0, &mut raw).unwrap() > 0,
                "Chrome closed its pipe"
            );
            raw.pop();
            let answer: Value = serde_json::from_slice(&raw).unwrap();
            if answer["id"] == json!(self.id) {
                assert!(answer.get("error").is_none(), "{method}: {answer}");
                return answer["result"].clone();
            }
        }
    }

    fn eval(&mut self, js: &str) -> Value {
        let r = self.send(
            "Runtime.evaluate",
            json!({"expression": js, "returnByValue": true}),
        );
        assert!(r.get("exceptionDetails").is_none(), "{js}: {r}");
        r["result"]["value"].clone()
    }

    fn until(&mut self, js: &str) {
        let end = Instant::now() + Duration::from_secs(10);
        while self.eval(js) != json!(true) {
            assert!(Instant::now() < end, "never true: {js}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn text(&mut self, sel: &str) -> String {
        let js = format!("document.querySelector({sel:?})?.textContent ?? '<none>'");
        self.eval(&js).as_str().unwrap().to_string()
    }

    fn shown(&mut self, sel: &str) -> bool {
        // checkVisibility: a closed <details> keeps its content's boxes, under content-visibility: hidden.
        self.eval(&format!(
            "!!document.querySelector({sel:?})?.checkVisibility()"
        )) == json!(true)
    }

    fn mouse(&mut self, kind: &str, (x, y): (f64, f64)) {
        let buttons = if kind == "mouseReleased" { 0 } else { 1 };
        self.send(
            "Input.dispatchMouseEvent",
            json!({"type": kind, "x": x, "y": y, "button": "left", "buttons": buttons, "clickCount": 1}),
        );
    }

    fn click(&mut self, sel: &str) {
        let at = self.centre(sel);
        self.mouse("mouseMoved", at);
        self.mouse("mousePressed", at);
        self.mouse("mouseReleased", at);
    }

    fn centre(&mut self, sel: &str) -> (f64, f64) {
        let js = format!(
            // scrolled to first, as a hand does: the editor sits atop the Documents tab, its menus may open past the fold
            "(e => e && (e.scrollIntoView({{block: 'nearest'}}), (r => [r.x + r.width / 2, r.y + r.height / 2])(e.getBoundingClientRect())))(document.querySelector({sel:?}))"
        );
        let at = self.eval(&js);
        (
            at[0].as_f64().unwrap_or_else(|| panic!("no {sel}")),
            at[1].as_f64().unwrap(),
        )
    }

    /// A drag across `words` of line `line`, from its first letter to its last, as a hand does it.
    fn drag(&mut self, line: usize, words: &str) {
        let js = format!(
            "(() => {{ const t = document.getElementById('no-text-{line}'), n = t.firstChild.firstChild, i = n.data.indexOf({words:?});
               const box = k => {{ const r = document.createRange(); r.setStart(n, k); r.setEnd(n, k + 1); return r.getBoundingClientRect(); }};
               const a = box(i), b = box(i + {len} - 1); return [a.left + 1, a.top + a.height / 2, b.right - 1]; }})()",
            len = words.len()
        );
        let p = self.eval(&js);
        let (x0, y, x1) = (
            p[0].as_f64().unwrap(),
            p[1].as_f64().unwrap(),
            p[2].as_f64().unwrap(),
        );
        self.mouse("mouseMoved", (x0, y));
        self.mouse("mousePressed", (x0, y));
        for step in 1..=8 {
            self.mouse("mouseMoved", (x0 + (x1 - x0) * step as f64 / 8.0, y));
        }
        self.mouse("mouseReleased", (x1, y));
    }

    /// The compass tapped and its pick armed: both picks add their listener on a timer, and a timer set after them runs after.
    fn arm(&mut self) {
        self.click("#dash-compass");
        self.eval("window.armed = false, setTimeout(() => armed = true), true");
        self.until("armed");
    }

    fn typed(&mut self, text: &str) {
        self.send("Input.insertText", json!({"text": text}));
    }

    fn key(&mut self, key: &str, code: u32) {
        for kind in ["keyDown", "keyUp"] {
            self.send(
                "Input.dispatchKeyEvent",
                json!({"type": kind, "key": key, "code": key, "windowsVirtualKeyCode": code}),
            );
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const LINE: usize = 4;
const WORDS: &str = "checked against";

#[test]
fn the_menu_and_every_tab_show_whole_at_1280() {
    let _budget = TestBudget::new();
    let mut b = Browser::notion("menu");
    let fit = b.eval(
        "(() => { const nav = document.getElementById('dash-nav').getBoundingClientRect(), bar = document.querySelector('.ds-bar');
           const r = bar.getBoundingClientRect(), tabs = [...bar.children].map(t => t.getBoundingClientRect());
           return { navRight: nav.right, barLeft: r.left, barRight: r.right, overflow: bar.scrollWidth - bar.clientWidth,
             cut: tabs.filter(t => t.width && (t.left < nav.right || t.right > innerWidth)).length,
             page: document.documentElement.scrollWidth - innerWidth }; })()",
    );
    assert!(
        fit["barLeft"].as_f64() >= fit["navRight"].as_f64(),
        "the bar starts under the menu: {fit}"
    );
    assert_eq!(fit["barRight"], json!(1280), "{fit}");
    assert_eq!(
        (
            fit["overflow"].clone(),
            fit["cut"].clone(),
            fit["page"].clone()
        ),
        (json!(0), json!(0), json!(0)),
        "{fit}"
    );
}

#[test]
fn a_drag_across_words_picks_them_and_opens_the_toolbar() {
    let _budget = TestBudget::new();
    let mut b = Browser::notion("drag");
    b.drag(LINE, WORDS);
    assert_eq!(b.text(".no-pick"), WORDS);
    assert!(
        b.shown(&format!("#no-line-{LINE} #no-tool")),
        "the toolbar opens on the line"
    );
    // A click elsewhere closes it and drops the pick.
    b.click(".no-title");
    assert!(!b.shown("#no-tool"));
    assert_eq!(b.text(".no-pick"), "<none>");
}

#[test]
fn a_comment_is_posted_on_the_words_and_on_the_page() {
    let _budget = TestBudget::new();
    let mut b = Browser::notion("comment");
    b.drag(LINE, WORDS);
    b.click("#no-comment");
    assert!(!b.shown("#no-tool"), "Comment closes the toolbar");
    assert!(
        b.shown("#no-new #no-input"),
        "a thread opens with its field"
    );
    assert_eq!(b.text("#no-new .no-quote"), WORDS);
    b.typed("Looks right");
    assert_eq!(
        b.eval("document.getElementById('no-send').classList.contains('off')"),
        json!(false)
    );
    b.click("#no-send");
    assert_eq!(b.text("#no-said-0-0 .no-words"), "Looks right");
    assert_eq!(b.text(".no-mark"), WORDS, "the words stay marked");
    // The reply field stays open and Enter posts.
    b.typed("And a reply");
    b.key("Enter", 13);
    assert_eq!(b.text("#no-said-0-1 .no-words"), "And a reply");
    // Its « ... » shows on hover.
    let over = b.centre("#no-said-0-0");
    b.mouse("mouseMoved", over);
    b.click("#no-said-0-0-more");
    assert!(b.shown("#no-said-0-0 #no-menu"), "a comment's menu opens");
    // A draft outlives a drag elsewhere, which draws the threads again.
    b.typed("A third");
    b.key("Enter", 13);
    b.typed("unsent");
    b.drag(5, "Memory");
    assert_eq!(
        b.eval("document.getElementById('no-input').value"),
        json!("unsent")
    );
    // A comment on the page itself folds the threads, as Notion does when one loses focus.
    b.click("#no-add");
    assert_eq!(b.text("#no-unfold-0"), "Show 1 reply");
    assert!(!b.shown("#no-said-0-1"));
    b.click("#no-unfold-0");
    assert_eq!(b.text("#no-said-0-1 .no-words"), "And a reply");
    b.click("#no-add");
    assert!(b.shown("#no-page-input"));
    b.typed("On the whole page");
    b.click("#no-page-post");
    assert_eq!(b.text("#no-page-0 .no-words"), "On the whole page");
}

#[test]
fn the_toggle_opens_and_closes() {
    let _budget = TestBudget::new();
    let mut b = Browser::notion("toggle");
    assert!(!b.shown("#no-toggle .no-line"), "closed at first");
    b.click("#no-toggle-head");
    assert!(b.shown("#no-toggle .no-line"));
    b.click("#no-toggle-head");
    assert!(!b.shown("#no-toggle .no-line"));
}

#[test]
fn a_slash_opens_the_blocks_and_one_is_picked() {
    let _budget = TestBudget::new();
    let mut b = Browser::notion("slash");
    assert!(!b.shown("#no-slash"));
    b.click("#no-slash-input");
    b.typed("/");
    assert!(b.shown("#no-slash"), "a slash opens the menu");
    b.typed("head");
    let offered = b.eval("[...document.querySelectorAll('#no-slash [data-kind]')].filter(e => e.checkVisibility()).map(e => e.textContent)");
    assert_eq!(offered, json!(["Heading"]));
    b.key("Enter", 13);
    assert!(!b.shown("#no-slash"));
    assert_eq!(
        b.eval("document.getElementById('no-block').className"),
        json!("no-line no-block heading")
    );
    b.typed("/");
    b.click("#no-slash-todo");
    assert_eq!(
        b.eval("document.getElementById('no-block').className"),
        json!("no-line no-block todo")
    );
    b.typed("/");
    b.key("Escape", 27);
    assert!(!b.shown("#no-slash"), "Escape closes it");
}

/// Task rust-compass-any-element (@nha, 2026-11-09: « the comments should work on any element. It doesn't work on anything on
/// the menu »): Home's cards under its heading, the sidebar as every page wears it. The compass, then a tap on a sidebar item,
/// a card or the heading, opens the box quoting it, and Send posts a path that finds that very element again.
#[test]
fn the_compass_comments_on_the_sidebar_a_card_and_a_heading() {
    let _budget = TestBudget::new();
    let tree = r#"{"root":"home","elements":{
        "home":{"type":"stack","children":["h","cards"]},
        "h":{"type":"heading","props":{"text":"Projects"}},
        "cards":{"type":"cards","children":["c0","c1"]},
        "c0":{"type":"card","props":{"title":"site_perso","line":"3 open","href":"/tasks?project=site_perso"}},
        "c1":{"type":"card","props":{"title":"klar","line":"1 open","href":"/tasks?project=klar"}}}}"#;
    let page =
        dash_surface::forms::with_menu(&dash_surface::screen::page("Home", tree).unwrap(), "/", "");
    // a file page posts nowhere: the post is kept for the test, answered as refused so the page does not reload
    let ready = "!!document.getElementById('c1') && (window.posted = [], window.fetch = (u, o) => { \
        if (o && o.method === 'POST') posted.push(Object.fromEntries(o.body)); \
        return Promise.resolve({ ok: false, json: () => ({ agent: '', notes: [] }), text: () => '' }) }, true)";
    let mut b = Browser::open("compass", &page, ready);
    // an icon with no words, named by its button
    b.eval("document.getElementById('h').insertAdjacentHTML('afterend', '<button title=\"Delete\"><svg id=\"icon\" width=\"24\" height=\"24\"><rect width=\"24\" height=\"24\"/></svg></button>'), true");
    let picks = [
        ("#dash-menu a[href='/tasks']", "Tasks"),
        ("#dash-menu a[href='/tasks?project=klar']", "klar"),
        ("#dash-nav label[title='Fold the menu']", "\u{ab}"),
        ("#c1", "klar 1 open"),
        ("#h", "Projects"),
        ("#icon", "Delete"),
    ];
    for (n, (sel, quote)) in picks.iter().enumerate() {
        b.arm();
        assert_eq!(
            b.eval("document.getElementById('dash-compass').getAttribute('aria-pressed')"),
            json!("true"),
            "{sel}"
        );
        // its own box, not a child's: a tap inside its edge
        let at = b.eval(&format!(
            "(r => [r.left + 2, r.top + 2])(document.querySelector({sel:?}).getBoundingClientRect())"
        ));
        let at = (at[0].as_f64().unwrap(), at[1].as_f64().unwrap());
        for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
            b.mouse(kind, at);
        }
        let hit = b.eval(&format!(
            "(e => e && e.outerHTML.slice(0, 120))(document.elementFromPoint({}, {}))",
            at.0, at.1
        ));
        assert!(
            b.shown("#dash-compose"),
            "the box opens on {sel}: tapped {hit}"
        );
        assert_eq!(b.text("#dash-compose blockquote"), *quote, "{sel}");
        b.typed(&format!("note {n}"));
        b.key("Enter", 13);
        b.until(&format!("posted.length === {}", n + 1));
        let sent = b.eval(&format!("posted[{n}]"));
        // the box closed first: it sits over the page's foot
        b.eval("document.getElementById('dash-compose').remove(), true");
        assert_eq!(sent["text"], json!(format!("note {n}")), "{sent}");
        assert_eq!(sent["quote"], json!(quote), "{sent}");
        assert_eq!(
            b.eval(&format!(
                "(e => e === document.elementFromPoint({}, {}) && !!e.closest({sel:?}))(document.querySelector({:?}))",
                at.0,
                at.1,
                sent["path"].as_str().unwrap()
            )),
            json!(true),
            "{sel}: anchored at {sent}"
        );
        // the menu's own clicks did not run: no fold, the file page still there
        assert_eq!(
            b.eval("document.getElementById('dash-nav-fold').checked"),
            json!(false),
            "{sel}"
        );
    }
    // An older page's thread starts its own pick and keeps its content; a tap on this sidebar is still ours.
    b.eval(
        "window.picks = 0; const t = document.createElement('div'); t.id = 'thread'; document.body.append(t); \
        const eat = e => { e.preventDefault(); e.stopPropagation(); \
          if (e.type === 'click') for (const k of ['pointerdown', 'pointerup', 'click']) document.removeEventListener(k, eat, true) }; \
        t.addEventListener('comments:pick', () => { picks++; setTimeout(() => { \
          for (const k of ['pointerdown', 'pointerup', 'click']) document.addEventListener(k, eat, true) }) }), true",
    );
    b.arm();
    b.click("#h");
    assert_eq!(b.eval("picks"), json!(1));
    assert!(!b.shown("#dash-compose"), "the older pick's own");
    assert_eq!(
        b.eval("document.getElementById('dash-compass').getAttribute('aria-pressed')"),
        json!("false")
    );
    b.arm();
    b.click("#dash-menu a[href='/mail']");
    assert_eq!(b.eval("picks"), json!(2));
    assert_eq!(b.text("#dash-compose blockquote"), "Mail");
    // On a phone, an older page: the drawer opened, the compass in it, an item of it is ours.
    b.eval("document.getElementById('dash-compose').remove(), true");
    b.send(
        "Emulation.setDeviceMetricsOverride",
        json!({"width": 390, "height": 844, "deviceScaleFactor": 1, "mobile": true}),
    );
    b.until("!document.getElementById('dash-nav').checkVisibility()");
    b.click("#dash-nav-open");
    b.until("document.getElementById('dash-nav').checkVisibility()");
    b.arm();
    b.click("#dash-menu a[href='/tools']");
    assert_eq!(b.eval("picks"), json!(3));
    assert_eq!(b.text("#dash-compose blockquote"), "Tools");
}

/// A port on loopback that tells whether anything reached it, and its number.
fn watched_port() -> (TcpListener, u16) {
    let port = TcpListener::bind("127.0.0.1:0").expect("a port");
    port.set_nonblocking(true).expect("non-blocking");
    let n = port.local_addr().expect("its address").port();
    (port, n)
}

/// Whether a connection reached `port` within `wait`.
fn reached(port: &TcpListener, wait: Duration) -> bool {
    let end = Instant::now() + wait;
    while Instant::now() < end {
        if port.accept().is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// /mail/t/<thread>'s page holding one open mail of `html`, as the server draws it.
fn a_mail(name: &str, html: &str) -> Browser {
    let tree = json!({"root": "page", "elements": {
        "page": {"type": "stack", "children": ["mail-0"]},
        "mail-0": {"type": "mail", "props": {"from": "Ada", "to": "nicolas@klar.im", "at": "2026-11-08T20:28:45Z",
            "text": "", "html": html, "open": true}},
    }});
    let page = dash_surface::screen::page("Mail", &tree.to_string()).expect("the page");
    Browser::open(name, &page, "!!document.querySelector('iframe.frame')")
}

/// Done-when 2 of rust-mail-thread: a script in a mail never runs. Its script would take the frame to the port; the same
/// mail in a frame that allows scripts does.
#[test]
fn a_mail_frame_runs_no_script() {
    let _budget = TestBudget::new();
    let (port, n) = watched_port();
    let script = format!("<p>Hi</p><script>location.href='http://127.0.0.1:{n}/ran'</script>");
    let mut b = a_mail("frame-script", &script);
    assert!(
        !reached(&port, Duration::from_secs(2)),
        "the mail's script ran"
    );
    let control = format!(
        "const f=document.createElement('iframe');f.sandbox='allow-scripts';f.srcdoc={script:?};document.body.append(f);true"
    );
    b.eval(&control);
    assert!(
        reached(&port, Duration::from_secs(5)),
        "the control's script never reached the port"
    );
}

/// Done-when 2 of rust-mail-thread: a mail's remote image, stylesheet and refresh load nothing; the same image on the page
/// around it does.
#[test]
fn a_mail_frame_loads_no_remote_image() {
    let _budget = TestBudget::new();
    let (port, n) = watched_port();
    let mail = format!(
        r#"<meta http-equiv="refresh" content="0;url=http://127.0.0.1:{n}/refresh"><link rel="stylesheet" href="http://127.0.0.1:{n}/s.css"><p>Hi</p><img src="http://127.0.0.1:{n}/pixel.gif"><div style="background:url(http://127.0.0.1:{n}/bg.png)">x</div>"#
    );
    let mut b = a_mail("frame-image", &mail);
    assert!(
        !reached(&port, Duration::from_secs(2)),
        "the mail loaded from the port"
    );
    b.eval(&format!("document.body.append(Object.assign(new Image(),{{src:'http://127.0.0.1:{n}/control.gif'}}));true"));
    assert!(
        reached(&port, Duration::from_secs(5)),
        "the control's image never reached the port"
    );
}
