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
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
            &dash_surface::design::page("notion", None, ""),
            "/design",
            "tab=notion",
        );
        Browser::open(name, &page, "!!document.getElementById('no-block')")
    }

    /// `page` from a file, open at 1280 by 900 once loaded and `ready` holds.
    fn open(name: &str, page: &str, ready: &str) -> Browser {
        let mut b = Browser::start(name);
        let file = b.dir.join("page.html");
        std::fs::write(&file, page).unwrap();
        b.go(&format!("file://{}", file.display()), ready);
        b
    }

    /// `url` open at 1280 by 900 once loaded and `ready` holds.
    fn go(&mut self, url: &str, ready: &str) {
        self.send("Page.navigate", json!({ "url": url }));
        self.until(&format!("document.readyState === 'complete' && {ready}"));
    }

    /// Chrome on a blank page at 1280 by 900.
    fn start(name: &str) -> Browser {
        let dir =
            std::env::temp_dir().join(format!("notion-browser-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin =
            dash_surface::pdf::chrome().expect("no Chrome: install Google Chrome or set CHROME");
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

    fn mouse(&mut self, kind: &str, at: (f64, f64)) {
        self.button(kind, at, "left", 1);
    }

    /// One mouse event of `button`, the `count`th press of a double or triple click.
    fn button(&mut self, kind: &str, (x, y): (f64, f64), button: &str, count: u32) {
        let held = match (kind, button) {
            ("mouseReleased", _) | (_, "none") => 0,
            (_, "right") => 2,
            _ => 1,
        };
        self.send(
            "Input.dispatchMouseEvent",
            json!({"type": kind, "x": x, "y": y, "button": button, "buttons": held, "clickCount": count}),
        );
    }

    /// `count` presses of `button` on `word` of line `line`, as a double or triple click or a right click is.
    fn clicks(&mut self, line: usize, word: &str, button: &str, count: u32) {
        let (left, y, right) = self.words_at(line, word);
        let at = ((left + right) / 2.0, y);
        self.button("mouseMoved", at, "none", 0);
        for n in 1..=count {
            self.button("mousePressed", at, button, n);
            self.button("mouseReleased", at, button, n);
        }
    }

    /// Where `words` of line `line` sit, in whichever of its runs holds them: their left, middle and right.
    fn words_at(&mut self, line: usize, words: &str) -> (f64, f64, f64) {
        let js = format!(
            "(() => {{ const w = {words:?}, walk = document.createTreeWalker(document.getElementById('no-text-{line}'), NodeFilter.SHOW_TEXT);
               for (let n; (n = walk.nextNode());) {{ const i = n.data.indexOf(w); if (i < 0) continue;
                 const r = document.createRange(); r.setStart(n, i); r.setEnd(n, i + w.length); const b = r.getBoundingClientRect();
                 return [b.left, b.top + b.height / 2, b.right]; }} }})()"
        );
        let p = self.eval(&js);
        let at = |i: usize| {
            p[i].as_f64()
                .unwrap_or_else(|| panic!("no {words:?} in line {line}"))
        };
        (at(0), at(1), at(2))
    }

    fn click(&mut self, sel: &str) {
        let at = self.centre(sel);
        self.mouse("mouseMoved", at);
        self.mouse("mousePressed", at);
        self.mouse("mouseReleased", at);
    }

    /// A click just past the last word of `sel`, as a hand clicks after a line's end.
    fn click_end(&mut self, sel: &str) {
        let at = self.eval(&format!(
            "(e => {{ const r = document.createRange(); r.selectNodeContents(e); const w = r.getBoundingClientRect(), b = e.getBoundingClientRect();
               return [Math.min(w.right + 12, b.right - 2), b.y + b.height / 2]; }})(document.querySelector({sel:?}))"
        ));
        let at = (at[0].as_f64().unwrap(), at[1].as_f64().unwrap());
        let hit = format!(
            "document.querySelector({sel:?}).contains(document.elementFromPoint({}, {}))",
            at.0, at.1
        );
        assert_eq!(self.eval(&hit), json!(true), "nothing covers {sel}'s end");
        for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
            self.mouse(kind, at);
        }
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
        let (left, y, right) = self.words_at(line, words);
        let (x0, x1) = (left + 1.0, right - 1.0);
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

/// A whole sentence selected, then a right click on it: the toolbar opens with its skills, the sentence picked (task
/// rust-notion-right-click; @nha, 2026-11-10: « I cannot do this on sentences for some reason only double-click on words. This
/// needs to be a right click imo. »).
#[test]
fn a_right_click_on_a_selected_sentence_opens_the_toolbar() {
    let _budget = TestBudget::new();
    let mut b = Browser::notion("right-click");
    let sentence = b.text("#no-text-3");
    b.clicks(3, "generated", "left", 3);
    assert_eq!(b.eval("getSelection().toString().trim()"), json!(sentence));
    assert_eq!(b.text(".no-pick"), sentence);
    b.click(".no-title");
    assert!(!b.shown("#no-tool"));
    // From « generated » on and into the next line's start, as a desktop triple click or an overshooting drag ends, with no
    // release, as the keyboard selects: only the right click opens the toolbar.
    b.eval(
        "(n => getSelection().setBaseAndExtent(n, n.data.indexOf('generated'), document.getElementById('no-text-4'), 0))
           (document.getElementById('no-text-3').firstChild.firstChild)",
    );
    assert!(!b.shown("#no-tool"));
    let tail = &sentence[sentence.find("generated").unwrap()..];
    // Twice: the line cut anew for the pick keeps the same words selected.
    for _ in 0..2 {
        b.clicks(3, "shared", "right", 1);
        assert!(
            b.shown("#no-line-3 #no-tool"),
            "the right click opens the toolbar on the line"
        );
        assert!(
            b.text("#no-tool").contains("Proofread"),
            "{}",
            b.text("#no-tool")
        );
        assert_eq!(b.text(".no-pick"), tail);
        assert_eq!(b.eval("getSelection().toString()"), json!(tail));
    }
}

/// A double click on a word picks it and opens the toolbar, as before the right click.
#[test]
fn a_double_click_picks_a_word_and_opens_the_toolbar() {
    let _budget = TestBudget::new();
    let mut b = Browser::notion("double-click");
    b.clicks(8, "face", "left", 2);
    assert_eq!(b.eval("getSelection().toString().trim()"), json!("face"));
    assert_eq!(b.text(".no-pick"), "face");
    assert!(b.shown("#no-line-8 #no-tool"));
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

/// Task notion-page-menu-export (@nha, 2026-11-09: « the menu should work, the"..." and propose to export to PDF. markdown. »):
/// the top bar's « ... » opens the DropdownMenu, its two items each download the document: Export Markdown its text as
/// Markdown, Export PDF its HTML form printed to PDF. pdftotext (poppler) reads the PDF; none fails the test, never skips it.
#[test]
fn the_page_menu_exports_the_document_to_markdown_and_pdf() {
    let _budget = TestBudget::new();
    // Served, as dash serves it, so Export PDF's link reaches its file: the page, and the PDF dash answers it with.
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = listener.local_addr().expect("its address").port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut head = String::new();
            let mut read = BufReader::new(&stream);
            while read.read_line(&mut head).is_ok_and(|n| n > 2) {}
            let target = head.split_whitespace().nth(1).unwrap_or_default();
            let (kind, extra, body) = match target == dash_surface::design::DOCUMENT_PDF {
                true => (
                    "application/pdf",
                    "Content-Disposition: attachment; filename=\"rendering-plan.pdf\"\r\n",
                    dash_surface::design::document_pdf().expect("printed"),
                ),
                false => (
                    "text/html; charset=utf-8",
                    "",
                    dash_surface::forms::with_menu(
                        &dash_surface::design::page("notion", None, ""),
                        "/design",
                        "tab=notion",
                    )
                    .into_bytes(),
                ),
            };
            let mut out = &stream;
            let _ = write!(
                out,
                "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = out.write_all(&body);
        }
    });
    let mut b = Browser::start("export");
    b.go(
        &format!("http://127.0.0.1:{port}/design?tab=notion"),
        "!!document.getElementById('no-block')",
    );
    let state = "document.getElementById('no-page-menu').dataset.state";
    assert!(!b.shown("#no-page-menu-content"), "closed at first");
    b.click("#no-page-menu-trigger");
    assert!(b.shown("#no-page-menu-content"), "the « ... » opens it");
    assert_eq!(b.eval(state), json!("open"));
    assert_eq!(
        b.eval("[...document.querySelectorAll('#no-page-menu-content [role=menuitem]')].map(e => e.textContent)"),
        json!(["Export PDF", "Export Markdown"])
    );
    b.key("Escape", 27);
    assert!(!b.shown("#no-page-menu-content"), "Escape closes it");
    // A click in its padding keeps it open; one elsewhere closes it.
    b.click("#no-page-menu-trigger");
    b.eval("document.getElementById('no-page-menu-content').dispatchEvent(new MouseEvent('click', {bubbles: true}))");
    assert!(b.shown("#no-page-menu-content"), "its padding keeps it");
    b.click(".no-title");
    assert!(
        !b.shown("#no-page-menu-content"),
        "a click elsewhere closes it"
    );

    // Export Markdown: a real click saves rendering-plan.md, the document as Markdown, and closes the menu.
    let saved = b.dir.join("downloads");
    b.send(
        "Page.setDownloadBehavior",
        json!({"behavior": "allow", "downloadPath": saved.display().to_string()}),
    );
    b.click("#no-page-menu-trigger");
    b.click("#no-export-md");
    assert!(!b.shown("#no-page-menu-content"), "an item closes it");
    let file = saved.join("rendering-plan.md");
    let markdown = "# Rendering plan\n\n2026:\n- One tree, two renderers\n  - the window and the web draw the same components\n  - theme.css generated from design.json, shared by both\n  - each behaviour is checked against the live document\n- Memory\n- Frame time\n- Fonts\n  - one face a window\n  - matches on family name, then falls back\n- Selection\n- Open questions\n  - Does a slow font load hold the first frame?\n  - One theme file for print too?\n";
    let got = written(&file, 10, |b| b == markdown.as_bytes());
    assert_eq!(String::from_utf8_lossy(&got), markdown);

    // Export PDF: a real click saves rendering-plan.pdf, the page printed by headless Chrome (dash_surface::pdf), whose text
    // is the document's lines alone, the closed toggle's included: none of the menu, the sidebar or the catalog under it.
    b.click("#no-page-menu-trigger");
    b.click("#no-export-pdf");
    assert_eq!(b.eval(state), json!("closed"));
    let pdf = saved.join("rendering-plan.pdf");
    let got = written(&pdf, 60, |b| b.trim_ascii_end().ends_with(b"%%EOF"));
    assert!(got.starts_with(b"%PDF-"), "a PDF");
    let text = Command::new("pdftotext")
        .arg(&pdf)
        .arg("-")
        .output()
        .expect("pdftotext: install poppler");
    let lines: Vec<String> = String::from_utf8_lossy(&text.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "\u{c}")
        .map(String::from)
        .collect();
    assert_eq!(
        lines,
        [
            "Rendering plan",
            "2026:",
            "One tree, two renderers",
            "the window and the web draw the same components",
            "theme.css generated from design.json, shared by both",
            "each behaviour is checked against the live document",
            "Memory",
            "Frame time",
            "Fonts",
            "one face a window",
            "matches on family name, then falls back",
            "Selection",
            "Open questions",
            "Does a slow font load hold the first frame?",
            "One theme file for print too?",
        ]
    );
}

/// Done-when 1 and 2 of rust-notion-one-menu (@nha, 2026-11-10: « Good but no double menu. the "ask" field is a good idea
/// though it needs to live somewhere else »): the Notion tab draws no sidebar of its own, the app's menu the one menu on the
/// page; its head holds the one Search or ask field beside the compass, and ⌘ K focuses it, a folded menu opened first.
#[test]
fn the_notion_tab_has_one_menu_its_ask_field_beside_the_compass_focused_by_cmd_k() {
    let _budget = TestBudget::new();
    let mut b = Browser::notion("one-menu");
    assert_eq!(
        b.eval("[document.querySelectorAll('.no-side').length, [...document.querySelectorAll('a[href=\"/inbox\"]')].filter(a => a.getClientRects().length).map(a => a.closest('nav, aside, .no-side, .no').id), document.querySelectorAll('[role=search]').length]"),
        json!([0, ["dash-menu"], 1]),
        "no .no-side; one menu, its Inbox the one shown (the tab's own sidebar had a second); one search field"
    );
    let head = b.eval(
        "(() => { const f = document.querySelector('#dash-nav .nav-head #dash-search .ask input').getBoundingClientRect();
           const c = document.getElementById('dash-compass').getBoundingClientRect();
           return { row: Math.abs((f.top + f.height / 2) - (c.top + c.height / 2)) < 2, left: f.right <= c.left, width: f.width,
             says: document.querySelector('#dash-search input').placeholder }; })()",
    );
    assert_eq!(head["row"], json!(true), "beside the compass: {head}");
    assert_eq!(head["left"], json!(true), "left of it: {head}");
    assert!(
        head["width"].as_f64().unwrap() >= 120.0,
        "room for its words: {head}"
    );
    assert_eq!(head["says"], json!("Search or ask"));
    // ⌘ K focuses it; folded, the menu opens first
    b.click("label[title='Fold the menu']");
    assert!(!b.shown("#dash-search input"), "folded");
    // ⌘ on a Mac (Meta, 4), Ctrl elsewhere (2), as the page reads it
    let mac = b.eval("/Mac|iP/.test(navigator.platform)") == json!(true);
    for kind in ["keyDown", "keyUp"] {
        b.send(
            "Input.dispatchKeyEvent",
            json!({"type": kind, "key": "k", "code": "KeyK", "windowsVirtualKeyCode": 75, "modifiers": if mac { 4 } else { 2 }}),
        );
    }
    assert!(b.shown("#dash-search input"), "opened");
    assert_eq!(
        b.eval("document.activeElement === document.querySelector('#dash-search input')"),
        json!(true),
        "⌘ K focuses it"
    );
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

/// Done-when 1 and 2 of rust-timeline-component in a real browser: /design's Timeline reads « 3 days ago », a click shows the
/// full date in its place and a second the age again, Enter on the focused date does the same; each row's dot sits centred on
/// its date's line, the line joins it to the next row's dot. The two looks are saved as PNGs in the build's tmp dir, where
/// a reviewer opens them.
#[test]
fn a_timeline_date_toggles_to_its_full_date_and_its_dot_sits_on_its_line() {
    let _budget = TestBudget::new();
    let page = dash_surface::forms::with_menu(
        &dash_surface::design::page("data", None, ""),
        "/design",
        "tab=data",
    );
    let mut b = Browser::open("timeline", &page, "!!document.querySelector('.timeline')");
    let row = ".app.light.ds-cell .timeline li:first-child";
    let (ago, day, summary) = (
        format!("{row} .when span"),
        format!("{row} .when time"),
        format!("{row} .when summary"),
    );
    let today = dash_surface::components::today();
    assert!(b.shown(&ago) && !b.shown(&day), "the age first");
    assert_eq!(b.text(&ago), "3 days ago");
    let shots = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    shot(&mut b, "#s-timeline", &shots.join("timeline-ago.png"));
    // the date's two forms as the browser holds them: its HTML, and the words it shows (innerText skips what CSS hides)
    let when = format!("{row} .when");
    let forms = |b: &mut Browser| {
        let v = b.eval(&format!(
            "(e => [e.outerHTML, e.innerText])(document.querySelector({when:?}))"
        ));
        (
            v[0].as_str().unwrap().to_string(),
            v[1].as_str().unwrap().to_string(),
        )
    };
    let full = dash_core::date(today - 3);
    assert_eq!(
        forms(&mut b),
        (
            format!(
                r#"<details class="when"><summary><span>3 days ago</span><time datetime="{full}">{full}</time></summary></details>"#
            ),
            "3 days ago".to_string()
        ),
        "« 3 days ago » by default"
    );
    b.click(&summary);
    assert!(!b.shown(&ago) && b.shown(&day), "the full date on a click");
    assert_eq!(b.text(&day), full);
    assert_eq!(
        forms(&mut b),
        (
            format!(
                r#"<details class="when" open=""><summary><span>3 days ago</span><time datetime="{full}">{full}</time></summary></details>"#
            ),
            full.clone()
        ),
        "the full date after the toggle"
    );
    shot(&mut b, "#s-timeline", &shots.join("timeline-date.png"));
    b.click(&summary);
    assert!(
        b.shown(&ago) && !b.shown(&day),
        "the age again on a second click"
    );
    b.eval(&format!(
        "document.querySelector({summary:?}).focus(), true"
    ));
    // Enter as a keyboard sends it, its character with it: a summary acts on that, not on the bare key
    for kind in ["keyDown", "keyUp"] {
        b.send(
            "Input.dispatchKeyEvent",
            json!({"type": kind, "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "text": "\r"}),
        );
    }
    assert!(b.shown(&day), "Enter opens it too");
    let geometry = b.eval(&format!(
        "(() => {{ const li = document.querySelector({row:?}), s = document.querySelector({summary:?});
           const r = li.getBoundingClientRect(), d = getComputedStyle(li, '::before'), l = getComputedStyle(li, '::after');
           const dot = r.top + parseFloat(d.top) + parseFloat(d.height) / 2, at = s.getBoundingClientRect();
           return {{ off: Math.abs(dot - (at.top + at.height / 2)), dot: d.width + ' ' + d.backgroundColor, line: l.height }}; }})()"
    ));
    assert!(
        geometry["off"].as_f64().unwrap() <= 1.0,
        "the dot on the date's line: {geometry}"
    );
    assert!(
        !geometry["dot"]
            .as_str()
            .unwrap()
            .contains("rgba(0, 0, 0, 0)"),
        "{geometry}"
    );
    assert!(
        geometry["line"].as_str().unwrap() != "auto" && geometry["line"] != json!("0px"),
        "{geometry}"
    );
    // the CRM entry document draws its events with the same component
    let documents = dash_surface::forms::with_menu(
        &dash_surface::design::page("documents", None, ""),
        "/design",
        "tab=documents",
    );
    let file = b.dir.join("documents.html");
    std::fs::write(&file, documents).unwrap();
    b.go(
        &format!("file://{}", file.display()),
        "!!document.querySelector('.doc-crm-entry .timeline')",
    );
    let entry = ".doc-crm-entry .timeline li:first-child .when";
    assert!(
        b.shown(&format!("{entry} span")),
        "the CRM entry's age first"
    );
    b.click(&format!("{entry} summary"));
    assert_eq!(b.text(&format!("{entry} time")), "2026-11-02");
    shot(&mut b, ".doc-crm-entry", &shots.join("crm-entry.png"));
}

/// A download's bytes once `done` holds, or the last read at `secs`: Chrome may create the file before it has written it all,
/// so its mere existence is no download (the Markdown export read a partial file once after a workspace build, task
/// rust-timeline-component).
fn written(file: &std::path::Path, secs: u64, done: impl Fn(&[u8]) -> bool) -> Vec<u8> {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        let got = std::fs::read(file).unwrap_or_default();
        if done(&got) || Instant::now() >= end {
            assert!(file.exists(), "no {}", file.display());
            return got;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The part `sel` picks, scrolled into view, saved as a PNG at `to`.
fn shot(b: &mut Browser, sel: &str, to: &std::path::Path) {
    let r = b.eval(&format!(
        "(f => (f.scrollIntoView(), (r => [r.x + scrollX, r.y + scrollY, r.width, r.height])(f.getBoundingClientRect())))(document.querySelector({sel:?}))"
    ));
    let clip = json!({"x": r[0], "y": r[1], "width": r[2], "height": r[3], "scale": 1});
    let png = b.send(
        "Page.captureScreenshot",
        json!({"format": "png", "clip": clip}),
    );
    let bytes = base64_decode(png["data"].as_str().expect("a shot"));
    std::fs::write(to, bytes).unwrap();
}

/// Chrome's base64, decoded: a shot's bytes, with no crate for one test.
fn base64_decode(text: &str) -> Vec<u8> {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0);
    for c in text.bytes().filter(|&c| c != b'=') {
        acc = acc << 6 | ABC.iter().position(|&a| a == c).expect("base64") as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// /design#stage, the Notion-style toggle (task design-stage-notion-toggle): the current value, a click opens the values, a pick
/// sets it and closes them.
#[test]
fn stage_shows_its_value_and_a_click_picks_another() {
    let _budget = TestBudget::new();
    let page = dash_surface::design::page("feedback", None, "");
    let mut b = Browser::open("stage", &page, "!!document.getElementById('s-stage')");
    let toggle = "#s-stage .ds-row:nth-child(2) .light .ds-toggle";
    let (value, menu) = (format!("{toggle} > summary"), format!("{toggle} .ds-pop"));
    assert_eq!(b.text(&value), "Review");
    assert!(!b.shown(&menu), "closed at first");
    b.click(&value);
    assert!(b.shown(&menu));
    b.click(&format!("{toggle} .ds-pop-line:nth-child(3)"));
    assert_eq!(b.text(&value), "Accepted");
    assert!(!b.shown(&menu), "a pick closes it");
    assert_eq!(b.text(&format!("{toggle} .ds-pop-line.on")), "Accepted");
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

/// Task rust-notion-doc-editable (@nha, 2026-11-09: « this doc needs to be able to be edited too. fine not to save it
/// automatically of course. »): a line takes what is typed, a comment on it keeps its words, a reload draws LINES again.
#[test]
fn a_line_is_edited_in_place_and_a_reload_draws_it_again() {
    let _budget = TestBudget::new();
    let mut b = Browser::notion("edit");
    let line = format!("#no-text-{LINE}");
    let was = b.text(&line);
    b.drag(LINE, WORDS);
    b.click("#no-comment");
    b.typed("Looks right");
    b.key("Enter", 13);
    // The caret put by script: Home and End scroll the page in Chrome on a Mac.
    let caret = |b: &mut Browser, js: &str| {
        b.eval(&format!(
            "(e => getSelection().collapse({js}))(document.querySelector({line:?}))"
        ));
    };
    b.click(&line);
    // Typed right where the commented words start: they stay outside the mark.
    caret(&mut b, "e.querySelector('.no-mark').firstChild, 0");
    b.typed("[");
    assert_eq!(
        b.text(".no-mark"),
        WORDS,
        "text typed at a mark's start stays out"
    );
    caret(&mut b, "e, 0");
    b.typed("Now ");
    caret(&mut b, "e, e.childNodes.length");
    b.key("Enter", 13);
    b.typed(", always\nhere");
    // A paste, as the browser sends one: its line breaks come in as spaces.
    b.eval(&format!(
        "(dt => (dt.setData('text/plain', 'and\\r\\nthere'), document.querySelector({line:?}).dispatchEvent(new InputEvent('beforeinput',
           {{ inputType: 'insertFromPaste', dataTransfer: dt, bubbles: true, cancelable: true }}))))(new DataTransfer())"
    ));
    let edited = format!(
        "Now {}, always hereand there",
        was.replace(WORDS, &format!("[{WORDS}"))
    );
    assert_eq!(
        b.text(&line),
        edited,
        "Enter, a newline and a paste add no line"
    );
    // A redraw (Add comment folds the threads) keeps the edit, and the comment's mark on its words.
    b.click("#no-add");
    assert_eq!(b.text(&line), edited, "a redraw keeps the edit");
    assert_eq!(b.text(".no-mark"), WORDS);
    // The commented words deleted: the thread keeps quoting them.
    b.eval(&format!(
        "getSelection().selectAllChildren(document.querySelector('{line} .no-mark')), document.execCommand('delete')"
    ));
    b.click("#no-add");
    assert_eq!(b.text(".no-mark"), "<none>");
    assert_eq!(b.text("#no-thread-0 .no-quote"), WORDS);
    b.eval("window.stale = true");
    b.send("Page.reload", json!({}));
    b.until("!window.stale && document.readyState === 'complete' && !!document.getElementById('no-block')");
    assert_eq!(b.text(&line), was, "nothing is saved");
    // A click lands past the last word, at the line's right end (its text fills the line; its middle is inside the words
    // since the tab lost its own sidebar, task rust-notion-one-menu). Undo works on a line with no marks: typing never
    // rebuilds it.
    b.click_end(&line);
    b.typed("!");
    assert_eq!(b.text(&line), format!("{was}!"));
    b.eval("document.execCommand('undo')");
    assert_eq!(b.text(&line), was, "undo takes the typing back");
    // A click drops a pick, on another line or on its own, and the redraw that follows keeps the caret where clicked. The
    // other line is below this one, so its toolbar never covers the click.
    b.drag(6, "Frame time");
    b.click_end(&line);
    b.typed("?");
    assert_eq!(b.text("#no-text-6 .no-pick"), "<none>");
    b.drag(LINE, WORDS);
    b.click_end(&line);
    b.typed("?");
    assert_eq!(
        b.text(&line),
        format!("{was}??"),
        "the caret stays where clicked"
    );
    // Picked words typed over are replaced, and the toolbar goes.
    b.drag(LINE, "live");
    assert!(b.shown("#no-tool"));
    b.typed("whole");
    assert_eq!(b.text(&line), format!("{}??", was.replace("live", "whole")));
    assert!(
        !b.shown("#no-tool"),
        "typing over a pick closes the toolbar"
    );
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
        // the documents editor's send, ⌘Enter, as every compose box's (task compose-uses-doc-editor)
        for kind in ["keyDown", "keyUp"] {
            b.send(
                "Input.dispatchKeyEvent",
                json!({"type": kind, "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "modifiers": 4}),
            );
        }
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
    b.click("#dash-more");
    b.until("document.getElementById('dash-nav').checkVisibility()");
    b.arm();
    b.click("#dash-menu a[href='/tools']");
    assert_eq!(b.eval("picks"), json!(3));
    assert_eq!(b.text("#dash-compose blockquote"), "Tools");
    // the phone's BottomNav is the menu too (task rust-ui-bottom-nav, /code-review): a tap on its slot is ours
    b.eval("document.getElementById('dash-compose').remove(), true");
    b.arm();
    b.click("#dash-bottom a[href='/mail']");
    assert_eq!(b.eval("picks"), json!(4));
    assert_eq!(b.text("#dash-compose blockquote"), "Mail");
}

/// Done-when 2 of rust-ui-bottom-nav (@nha, 2026-11-10: « mobile - https://rust-ui.com/docs/components/bottom-nav »): under
/// 700 px wide the menu's first entries are the BottomNav, one row of equal slots at the screen's foot, the page's own marked,
/// the page clear of it; More, last, opens the column with the rest (Tasks kept, its projects under it), by a tap or a key,
/// and a second tap closes it. Wide, the column alone.
#[test]
fn on_a_phone_the_menu_s_first_entries_are_the_bottom_nav_and_more_opens_the_rest() {
    let _budget = TestBudget::new();
    // a page as dash serves it: its theme and its viewport (without it a phone lays out 980 px wide)
    let tree = r#"{"root":"h","elements":{"h":{"type":"heading","props":{"text":"Inbox"}}}}"#;
    let page = dash_surface::screen::page("Inbox", tree).expect("the page");
    let page = dash_surface::forms::with_menu(&page, "/inbox", "");
    let mut b = Browser::open(
        "bottom-nav",
        &page,
        "!!document.getElementById('dash-bottom')",
    );
    assert!(
        b.shown("#dash-nav") && !b.shown("#dash-bottom"),
        "wide: the column alone"
    );
    b.send(
        "Emulation.setDeviceMetricsOverride",
        json!({"width": 390, "height": 844, "deviceScaleFactor": 1, "mobile": true}),
    );
    b.until("document.getElementById('dash-bottom').checkVisibility()");
    assert!(
        !b.shown("#dash-nav") && !b.shown("#dash-nav-open"),
        "the bar alone"
    );
    let slots = b.eval(
        "[...document.querySelectorAll('#dash-bottom .bottom-nav-item')].map(e => { const r = e.getBoundingClientRect(); \
         return [e.textContent, Math.round(r.top), Math.round(r.bottom), Math.round(r.width), e.getAttribute('aria-current')] })",
    );
    assert_eq!(
        slots,
        json!([
            ["Inbox", 788, 844, 78, "page"],
            ["Today", 788, 844, 78, null],
            ["Mail", 788, 844, 78, null],
            ["Tasks", 788, 844, 78, null],
            ["More", 788, 844, 78, null]
        ])
    );
    let tone = "[...document.querySelectorAll('#dash-bottom .bottom-nav-item')].map(e => getComputedStyle(e).color)";
    let tones = b.eval(tone);
    assert_ne!(
        tones[0], tones[1],
        "the current one in the primary tone: {tones}"
    );
    assert_eq!(tones[1], tones[4], "{tones}");
    // the bar's room is measured, never copied: its height in components.css and the page's padding stay one
    assert_eq!(
        b.eval("getComputedStyle(document.documentElement).paddingBottom === document.getElementById('dash-bottom').offsetHeight + 'px'"),
        json!(true),
        "the page's foot clears the bar"
    );
    let rest = "[...document.querySelectorAll('#dash-menu > a')].filter(a => a.checkVisibility()).map(a => a.getAttribute('href'))";
    b.click("#dash-more");
    b.until("document.getElementById('dash-nav').checkVisibility()");
    assert_eq!(
        b.eval(rest),
        json!([
            "/tasks",
            "/plan",
            "/insights",
            "/agents",
            "/tools",
            "/design",
            "/settings"
        ])
    );
    assert!(
        b.shown("#dash-menu a[href='/tasks?project=klar']"),
        "Tasks keeps its projects"
    );
    assert_eq!(
        b.eval("(r => document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2).closest('#dash-more') !== null)(document.getElementById('dash-more').getBoundingClientRect())"),
        json!(true),
        "More stays in reach beside the open column"
    );
    assert_eq!(
        b.eval("[...document.querySelectorAll('#dash-bottom .bottom-nav-item')].every(e => (r => document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2).closest('.bottom-nav-item') === e)(e.getBoundingClientRect()))"),
        json!(true),
        "the column ends above the bar: every slot in reach"
    );
    assert_eq!(
        b.eval("document.getElementById('dash-more').getAttribute('aria-expanded')"),
        json!("true")
    );
    b.click("#dash-more");
    b.until("!document.getElementById('dash-nav').checkVisibility()");
    b.eval("document.getElementById('dash-more').focus(), true");
    b.key("Enter", 13);
    b.until("document.getElementById('dash-nav').checkVisibility()");
    b.key("Enter", 13);
    b.until("!document.getElementById('dash-nav').checkVisibility()");
    b.send(
        "Emulation.setDeviceMetricsOverride",
        json!({"width": 1280, "height": 900, "deviceScaleFactor": 1, "mobile": false}),
    );
    b.until("document.getElementById('dash-nav').checkVisibility()");
    assert!(!b.shown("#dash-bottom"), "wide again: the column alone");
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

/// The ground and the ink of the frame `sel` picks as painted, scrolled into view once two frames have drawn: the shot of its
/// inside, clear of its rounded corners, its most common pixel and the one farthest from it, with the frame's own background as
/// the canvas turns it to sRGB. A sandboxed frame's document is out of the page's reach.
fn painted(b: &mut Browser, sel: &str) -> Value {
    b.eval("window.drawn = false, requestAnimationFrame(() => requestAnimationFrame(() => drawn = true)), true");
    b.until("drawn");
    let r = b.eval(&format!(
        "(f => (f.scrollIntoView(), (r => [r.x + scrollX + 16, r.y + scrollY + 16, r.width - 32, r.height - 32])(f.getBoundingClientRect())))(document.querySelector({sel:?}))"
    ));
    let clip = json!({"x": r[0], "y": r[1], "width": r[2], "height": r[3], "scale": 1});
    let shot = b.send(
        "Page.captureScreenshot",
        json!({"format": "png", "clip": clip}),
    );
    let js = format!(
        "new Promise(done => {{ const i = new Image(); i.onload = () => {{
           const c = document.createElement('canvas'); c.width = i.width; c.height = i.height; const x = c.getContext('2d', {{willReadFrequently: true}});
           x.drawImage(i, 0, 0); const d = x.getImageData(0, 0, c.width, c.height).data, seen = new Map();
           for (let k = 0; k < d.length; k += 4) {{ const p = d[k] + ',' + d[k+1] + ',' + d[k+2]; seen.set(p, (seen.get(p) || 0) + 1); }}
           const ground = [...seen].sort((a, b) => b[1] - a[1])[0][0].split(',').map(Number);
           const far = p => Math.abs(p[0] - ground[0]) + Math.abs(p[1] - ground[1]) + Math.abs(p[2] - ground[2]);
           const ink = [...seen.keys()].map(p => p.split(',').map(Number)).sort((a, b) => far(b) - far(a))[0];
           x.fillStyle = getComputedStyle(document.querySelector({sel:?})).backgroundColor; x.fillRect(0, 0, 1, 1);
           done({{ground, ink, theme: [...x.getImageData(0, 0, 1, 1).data.slice(0, 3)]}}); }}; i.src = 'data:image/png;base64,{}'; }})",
        shot["data"].as_str().expect("a shot")
    );
    let v = b.send(
        "Runtime.evaluate",
        json!({"expression": js, "returnByValue": true, "awaitPromise": true}),
    );
    v["result"]["value"].clone()
}

/// Done-when 1 and 2 of frame-dark-mode on /mail/t/<thread> (/design's cells are the same frame under the same theme roots):
/// a mail with no colours draws on the theme's ground with its text light on dark and dark on light; a mail with a
/// background of its own keeps it in both themes, its own dark-mode rule never matching on its light ground. The theme is the page root's class, as dash's line sets it.
#[test]
fn a_mail_frame_follows_the_theme_unless_it_has_its_own_ground() {
    let _budget = TestBudget::new();
    let tree = json!({"root": "page", "elements": {
        "page": {"type": "stack", "children": ["plain", "own"]},
        "plain": {"type": "mail", "props": {"from": "Ada", "to": "nicolas@klar.im", "at": "2026-11-08T20:28:45Z",
            "text": "", "html": "<p>Plain words, no colour</p>", "open": true}},
        "own": {"type": "mail", "props": {"from": "Bob", "to": "nicolas@klar.im", "at": "2026-11-08T20:30:00Z",
            "text": "", "html": r##"<body bgcolor="#ffcc00"><style>@media (prefers-color-scheme: dark) { p { color: #fff } }</style><p>Its own yellow</p></body>"##, "open": true}},
    }});
    let page = dash_surface::screen::page("Mail", &tree.to_string()).expect("the page");
    let mut b = Browser::open(
        "frame-theme",
        &page,
        "document.querySelectorAll('iframe.frame').length === 2",
    );
    for theme in ["light", "dark"] {
        b.eval(&format!(
            "document.documentElement.className = 'app {theme}'; true"
        ));
        let own = painted(&mut b, "iframe[title=\"The mail from Bob\"]");
        assert_eq!(own["ground"], json!([255, 204, 0]), "{theme}: {own}");
        let ink = (0..3).map(|k| own["ink"][k].as_i64().unwrap()).sum::<i64>() / 3;
        assert!(ink < 80, "{theme}: its words stay dark: {own}");
        on_the_theme(
            &painted(&mut b, "iframe[title=\"The mail from Ada\"]"),
            theme,
        );
    }
}

/// A plain mail's frame drawn on its theme's ground, its words light on dark and dark on light.
fn on_the_theme(plain: &Value, theme: &str) {
    let close = (0..3).all(|k| {
        (plain["ground"][k].as_i64().unwrap() - plain["theme"][k].as_i64().unwrap()).abs() <= 2
    });
    assert!(close, "{theme}: the theme's ground: {plain}");
    let light = |p: &Value| (0..3).map(|k| p[k].as_i64().unwrap()).sum::<i64>() / 3;
    let (ground, ink) = (light(&plain["ground"]), light(&plain["ink"]));
    match theme {
        "dark" => assert!(
            ground < 80 && ink > 180,
            "{theme}: light words on dark: {plain}"
        ),
        _ => assert!(
            ground > 180 && ink < 80,
            "{theme}: dark words on light: {plain}"
        ),
    }
}

/// Done-when 1 of frame-dark-mode on /design: the Frame's Dark cell draws its plain mail on the dark ground, light words, in
/// a page whose root is light; its Light cell stays light.
#[test]
fn the_design_frame_is_dark_in_its_dark_cell() {
    let _budget = TestBudget::new();
    let page = dash_surface::design::page("components", None, "");
    let mut b = Browser::open(
        "frame-design",
        &page,
        "!!document.querySelector('.ds-cell.dark iframe.frame')",
    );
    b.eval("document.documentElement.className = 'app light'; true");
    on_the_theme(
        &painted(&mut b, "#s-frame .ds-cell.light iframe.frame"),
        "light",
    );
    on_the_theme(
        &painted(&mut b, "#s-frame .ds-cell.dark iframe.frame"),
        "dark",
    );
}

/// /tasks' board as dash serve draws it, a field and a fold above it: `moved` draws card a in progress rather than todo.
fn a_board(moved: bool) -> String {
    let (todo, doing): (&[&str], &[&str]) = match moved {
        false => (&["task-a"], &[]),
        true => (&[], &["task-a"]),
    };
    let at = if moved { "in-progress" } else { "todo" };
    let tree = json!({"root": "page", "elements": {
        "page": {"type": "stack", "children": ["find", "d", "board"]},
        "find": {"type": "search-bar", "props": {"placeholder": "Find"}},
        "d": {"type": "diagram", "props": {"source": "graph TD\nA-->B"}},
        "board": {"type": "board", "props": {"view": "board"}, "children": ["backlog", "todo", "in-progress"]},
        "backlog": {"type": "issue-group", "props": {"status": "backlog"}, "children": ["task-b"]},
        "todo": {"type": "issue-group", "props": {"status": "todo"}, "children": todo},
        "in-progress": {"type": "issue-group", "props": {"status": "in-progress"}, "children": doing},
        "task-a": {"type": "issue-row", "props": {"id": "a", "title": "A", "status": at, "says": "", "comments": 0}},
        "task-b": {"type": "issue-row", "props": {"id": "b", "title": "B", "status": "backlog", "says": "", "comments": 0}},
    }});
    dash_surface::screen::page("Tasks", &tree.to_string()).expect("the page")
}

/// A server of /tasks as dash serve's: the board, `moved` once set, and `?view=patches`, an event stream of what `patches`
/// sends. Returns its port and how many times the page was fetched.
fn a_board_server(moved: Arc<AtomicBool>, patches: Receiver<String>) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = listener.local_addr().expect("its address").port();
    let fetched = Arc::new(AtomicUsize::new(0));
    let (count, patches) = (Arc::clone(&fetched), Arc::new(Mutex::new(patches)));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (moved, count, patches) =
                (Arc::clone(&moved), Arc::clone(&count), Arc::clone(&patches));
            std::thread::spawn(move || {
                let mut head = String::new();
                let mut read = BufReader::new(&stream);
                while read.read_line(&mut head).is_ok_and(|n| n > 2) {}
                let target = head.split_whitespace().nth(1).unwrap_or_default();
                let mut out = &stream;
                if target.contains("view=patches") {
                    let _ = out.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\n");
                    let _ = out.flush();
                    while let Ok(ids) = patches.lock().unwrap().recv() {
                        let _ = write!(out, "data: {ids}\n\n");
                        let _ = out.flush();
                    }
                } else if target.starts_with("/tasks") {
                    count.fetch_add(1, SeqCst);
                    let page = a_board(moved.load(SeqCst));
                    let _ = write!(
                        out,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
                        page.len()
                    );
                } else {
                    let _ = out.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                }
            });
        }
    });
    (port, fetched)
}

/// Done-when 2 of rust-view-patches: a view's diff reaches the page as a patch of the columns it changes; the words typed in
/// a field, its focus, an open fold and the column the diff left alone are the same elements after it (@nha, 2026-11-09:
/// « both should work together locally too, via the timely dataflow updates for views. »).
#[test]
fn a_board_diff_patches_its_columns_and_keeps_a_field_and_an_open_fold() {
    let _budget = TestBudget::new();
    let moved = Arc::new(AtomicBool::new(false));
    let (send, patches) = std::sync::mpsc::channel();
    let (port, fetched) = a_board_server(Arc::clone(&moved), patches);
    let mut b = Browser::start("board-patch");
    b.go(
        &format!("http://127.0.0.1:{port}/tasks"),
        "!!document.querySelector('#todo [data-task=a]')",
    );
    // the stream's opening draws the board once, for a diff missed while away: settled before the hand starts
    let end = Instant::now() + Duration::from_secs(10);
    while fetched.load(SeqCst) < 2 {
        assert!(Instant::now() < end, "the stream never opened");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(500));
    b.click("#d-source summary");
    b.until("document.getElementById('d-source').open");
    b.click("input[type=search]");
    b.typed("half a");
    b.eval("document.getElementById('backlog').dataset.kept = 'yes'");

    moved.store(true, SeqCst);
    send.send(r#"["todo","in-progress"]"#.to_string()).unwrap();
    b.until("!!document.querySelector('#in-progress [data-task=a]')");
    assert_eq!(
        b.eval("document.querySelector('#todo [data-task=a]')"),
        Value::Null
    );
    let kept = [
        (
            "document.querySelector('input[type=search]').value",
            json!("half a"),
        ),
        ("document.activeElement.type", json!("search")),
        ("document.getElementById('d-source').open", json!(true)),
        // a column the diff left alone is the same element
        (
            "document.getElementById('backlog').dataset.kept",
            json!("yes"),
        ),
    ];
    for (js, was) in kept {
        assert_eq!(b.eval(js), was, "{js}");
    }
}

/// Done-when 2 of rust-deps-files-hover: on /deps' treemap, the pointer on a box outlines, solid, the boxes it uses and,
/// dashed, those using it, double those that do both, at every depth down to files; a box with no link to it keeps no outline,
/// and leaving clears them.
#[test]
fn a_hovered_box_outlines_what_it_uses_and_what_uses_it() {
    let _budget = TestBudget::new();
    let tree = json!({"root": "page", "elements": {
        "page": {"type": "stack", "children": ["map"]},
        "map": {"type": "treemap", "props": {"boxes": [
            {"name": "Rings", "uses": ["Core"], "children": [
                {"name": "store", "uses": ["Core/core", "Rings/surface"], "children": [
                    {"name": "a.rs", "size": 600, "uses": ["Rings/store/b.rs"]},
                    {"name": "b.rs", "size": 400}]},
                {"name": "surface", "size": 500, "uses": ["Rings/store"]}]},
            {"name": "Core", "children": [{"name": "core", "children": [{"name": "lib.rs", "size": 700}]}]}]}},
    }});
    let page = dash_surface::screen::page("Code map", &tree.to_string()).expect("the page");
    let mut b = Browser::open(
        "treemap-hover",
        &page,
        "!!document.querySelector('.treemap-box')",
    );
    let outline = |b: &mut Browser, path: &str| {
        let js = format!(
            "getComputedStyle(document.querySelector('[data-path=\"{path}\"]')).outlineStyle"
        );
        b.eval(&js).as_str().expect("a style").to_string()
    };
    let boxes = [
        "Rings",
        "Rings/store",
        "Rings/store/a.rs",
        "Rings/store/b.rs",
        "Rings/surface",
        "Core",
        "Core/core",
        "Core/core/lib.rs",
    ];
    for path in boxes {
        assert!(
            b.shown(&format!("[data-path=\"{path}\"]")),
            "{path} not drawn"
        );
    }
    let hover = |b: &mut Browser, path: &str| {
        let at = b.centre(&format!("[data-path=\"{path}\"]"));
        b.mouse("mouseMoved", at);
    };
    hover(&mut b, "Rings/store/a.rs");
    assert_eq!(outline(&mut b, "Rings/store/b.rs"), "solid", "a file's use");
    assert_eq!(outline(&mut b, "Rings/surface"), "none");
    hover(&mut b, "Rings/store/b.rs");
    assert_eq!(
        outline(&mut b, "Rings/store/a.rs"),
        "dashed",
        "a file's user"
    );
    hover(&mut b, "Core/core/lib.rs");
    assert_eq!(
        outline(&mut b, "Rings/store/a.rs"),
        "none",
        "left: no outline stays"
    );
    // a crate box is hovered on its name's line, above its files
    let js = "(r => [r.x + 8, r.y + 6])(document.querySelector('[data-path=\"Rings/store\"]').getBoundingClientRect())";
    let at = b.eval(js);
    b.mouse(
        "mouseMoved",
        (at[0].as_f64().unwrap(), at[1].as_f64().unwrap()),
    );
    assert_eq!(outline(&mut b, "Core/core"), "solid", "a crate's use");
    assert_eq!(
        outline(&mut b, "Rings/surface"),
        "double",
        "used and using: both shown"
    );
    let js = "(r => [r.x + 8, r.y + 6])(document.querySelector('[data-path=\"Core\"]').getBoundingClientRect())";
    let at = b.eval(js);
    b.mouse(
        "mouseMoved",
        (at[0].as_f64().unwrap(), at[1].as_f64().unwrap()),
    );
    assert_eq!(outline(&mut b, "Rings"), "dashed", "a layer's user");
}

/// Task rust-task-brief-linear, done-when 1 and 2 (@nha, 2026-11-10: « not matching the design yet (see elixir/linear as
/// example) »): on /design?tab=documents the Task brief is Linear's issue page, its title over its description, its properties
/// beside them, its Activity last and ending on the comment box; its description and a comment, each the one DocView, take
/// what is typed in place, and a reload draws the saved words again. Its shot is left at CARGO_TARGET_TMPDIR/task-brief.png.
#[test]
fn the_task_brief_is_linear_s_issue_and_its_words_are_edited_in_place() {
    let _budget = TestBudget::new();
    let page = dash_surface::forms::with_menu(
        &dash_surface::design::page("documents", None, ""),
        "/design",
        "tab=documents",
    );
    let ready = "!!document.getElementById('d-task-brief')";
    let mut b = Browser::open("brief", &page, ready);
    let at = b.eval(
        "(() => { const one = s => document.querySelector('#d-task-brief ' + s), box = s => one(s).getBoundingClientRect();
           const [title, why, main, side] = ['.li-issue-title', '#d-task-brief-why', '.li-issue-main', '.li-issue-side'].map(box);
           const activity = [...document.querySelectorAll('#d-task-brief .li-part')].at(-1);
           return { beside: side.left >= main.right, level: side.top < why.bottom, under: why.top >= title.bottom,
             last: activity.querySelector('h2').textContent, ends: activity.lastElementChild.id,
             thread: !!activity.querySelector('.doc-thread .dv'), field: !!activity.querySelector('.editor-field .dv[contenteditable]') }; })()",
    );
    assert_eq!(
        at,
        json!({"beside": true, "level": true, "under": true, "last": "Activity", "ends": "d-task-brief-comment",
               "thread": true, "field": true}),
        "Linear's issue layout"
    );
    let shot_at = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("task-brief.png");
    shot(&mut b, "#s-task-brief .ds-cell", &shot_at);
    eprintln!("the Task brief's shot: {}", shot_at.display());
    // Typed at the end of each, the caret put by script as the edit test does (Home and End scroll the page on a Mac).
    let why = "#d-task-brief-why pre";
    let said = "#d-task-brief .doc-words pre";
    let (why_was, said_was) = (b.text(why), b.text(said));
    for (sel, words) in [(why, ", edited"), (said, ", now")] {
        b.click(sel);
        b.eval(&format!(
            "(e => getSelection().collapse(e.firstChild, e.firstChild.length))(document.querySelector({sel:?}))"
        ));
        b.typed(words);
    }
    assert_eq!(b.text(why), format!("{why_was}, edited"), "the description");
    assert_eq!(b.text(said), format!("{said_was}, now"), "the comment");
    // Kept in the page's memory alone: a reload draws the saved words.
    b.go(
        &format!("file://{}", b.dir.join("page.html").display()),
        ready,
    );
    assert_eq!((b.text(why), b.text(said)), (why_was, said_was));
}

/// Task compose-uses-doc-editor, done-when 1, 2 and 4 (@nha, 2026-11-09: « the input should be a component reused from
/// documents. same for the input controls. »): /design's Composer, as a hand uses it. Its field takes what is typed, Undo and
/// Redo walk its history, Preview shows it read only, Discard empties it and Undo brings the words back; Send sits inside the
/// field's border, and ⌘Enter posts the field's words as `text`.
#[test]
fn the_composer_is_the_documents_editor_and_sends_its_words() {
    let _budget = TestBudget::new();
    let page = dash_surface::forms::with_menu(
        &dash_surface::design::page("mail", None, ""),
        "/design",
        "tab=mail",
    );
    let mut b = Browser::open("compose", &page, "!!document.getElementById('ds-composer')");
    let field = "#ds-composer .editor-field .dv";
    let empty = |b: &mut Browser| {
        b.eval("document.querySelector('#ds-composer .editor-field').hasAttribute('data-empty')")
            == json!(true)
    };
    assert!(empty(&mut b), "its placeholder shows");
    b.click(field);
    b.typed("Dear Ada");
    assert_eq!(b.text(field), "Dear Ada");
    assert!(!empty(&mut b), "the placeholder goes");
    // another editable takes an edit after, as a task page's description does: Undo here walks this field's history alone
    // (/code-review); the dark cell's field is inert
    b.eval("document.body.append(Object.assign(document.createElement('div'), { id: 'other', contentEditable: 'plaintext-only' })), true");
    let other = "#other";
    b.click(other);
    b.typed("Other");
    b.click("#ds-composer .editor-undo");
    assert_eq!(b.text(field).trim(), "", "Undo");
    assert_eq!(b.text(other), "Other", "the other field kept its words");
    b.click("#ds-composer .editor-redo");
    assert_eq!(b.text(field), "Dear Ada", "Redo");
    b.click("#ds-composer .editor-preview");
    assert_eq!(
        b.eval(&format!(
            "document.querySelector({field:?}).isContentEditable"
        )),
        json!(false),
        "Preview is read only"
    );
    b.click("#ds-composer .editor-preview");
    b.click("#ds-composer .editor-discard");
    assert_eq!(b.text(field).trim(), "", "Discard");
    assert!(empty(&mut b));
    b.click(field);
    for kind in ["keyDown", "keyUp"] {
        b.send(
            "Input.dispatchKeyEvent",
            json!({"type": kind, "key": "z", "code": "KeyZ", "windowsVirtualKeyCode": 90, "modifiers": 4}),
        );
    }
    assert_eq!(b.text(field), "Dear Ada", "⌘Z brings a discard back");
    let inside = b.eval(
        "[...document.querySelectorAll('#s-composer .editor-field')].every(field => { const f = field.getBoundingClientRect();
           return [...field.querySelectorAll('.editor-discard, .editor-send')].map(e => e.getBoundingClientRect())
             .every(r => r.width > 0 && r.left >= f.left && r.right <= f.right && r.top >= f.top && r.bottom <= f.bottom); })",
    );
    shot(
        &mut b,
        "#s-composer",
        &std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("composer.png"),
    );
    assert_eq!(
        inside,
        json!(true),
        "Discard and Send inside the field, in each cell"
    );
    b.eval("window.addEventListener('submit', e => { e.preventDefault(); window.sent = e.target.elements.text.value }), true");
    b.click(field);
    for kind in ["keyDown", "keyUp"] {
        b.send(
            "Input.dispatchKeyEvent",
            json!({"type": kind, "key": "Enter", "code": "Enter", "windowsVirtualKeyCode": 13, "modifiers": 4}),
        );
    }
    assert_eq!(
        b.eval("window.sent"),
        json!("Dear Ada"),
        "⌘Enter sends the words"
    );
}

/// Task compose-uses-doc-editor, done-when 4: on /, Ask sits inside the field's border, as on /design's Composer. The tree is
/// Home's prompt (dash's home_page), drawn by the same screen::page dash serves.
#[test]
fn home_s_ask_sits_inside_the_field() {
    let _budget = TestBudget::new();
    let tree = r#"{"root":"ask","elements":{"ask":{"type":"compose","props":{"action":"/compass","send":"Ask","placeholder":"Ask dash, or tell it what to do","hidden":[["page","/"]]}}}}"#;
    let page = dash_surface::screen::page("Home", tree).expect("a page");
    let mut b = Browser::open("home-ask", &page, "!!document.getElementById('ask')");
    let at = b.eval(
        "(() => { const field = document.querySelector('#ask .editor-field'), f = field.getBoundingClientRect();
           const ask = field.querySelector('.editor-send'), r = ask.getBoundingClientRect();
           return { nested: field.contains(ask), word: ask.textContent, inside: r.width > 0 && r.left >= f.left
             && r.right <= f.right && r.top >= f.top && r.bottom <= f.bottom }; })()",
    );
    assert_eq!(
        at,
        json!({"nested": true, "word": "Ask", "inside": true}),
        "Ask inside the field"
    );
}
