//! `dash apply greenhouse <job-url> --draft <id> [--send]` (task rust-apply-greenhouse; @nha, 2026-11-11: « it says
//! "approved" if it really is why do you ask ? If it is then where is the CV ? »): an approved form draft goes out through
//! a Greenhouse job board, which has no API to post an application, in a headless Chrome of its own profile.
//! A dry run by default: the form is filled with his name and mail (the model's profile.edn `candidate`), the draft's CV
//! (its first `attach`) and its text (as a cover letter file, the one way in that needs no React click), then each field
//! is read back and printed, with every required field left empty. `--send` refuses a draft that is not approved (a sent
//! one is never sent again), a page whose company the draft does not name (free-work's TL Consulting lesson, 2026-11-05)
//! and a form with a required field the draft does not answer; else it submits, waits for the confirmation page, and
//! records the draft sent and an outbound line holding that page's text, through the older app (`dash draft status`,
//! `dash model outbound add`), which owns both writes until they are ported.
//! Every other field (phone, city, a select such as Privacy, a screening question) takes its answer by its label from the
//! draft's `answers` (`[{question, answer}]`, the browser-apply shape), then from `--answer '<question>=<answer>'`, which
//! wins: a text field typed, a select picked among its options (an exact match first, else the first that starts so: «
//! France » picks « France +33 »). A Greenhouse that asks its emailed security code after the submit gets it from his inbox
//! (`mail::search`), then the submit is clicked again.
//! ponytail: a reCAPTCHA challenge shown as a picture is not answered: it ends in « no confirmation », nothing recorded.
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The whole run, Chrome's start and a browser slot's wait included: past it, Chrome's group is killed and the run fails.
const LIMIT: Duration = Duration::from_secs(300);
/// How long the form, then the confirmation page, may take to show.
const SHOW: Duration = Duration::from_secs(30);
/// The older app's writes: some 0.2 s.
const OLDER_CALL: Duration = Duration::from_secs(60);

/// What goes in the form.
#[derive(Debug, PartialEq)]
pub struct Application {
    pub first: String,
    pub last: String,
    pub email: String,
    pub cv: PathBuf,
    pub message: String,
    /// Each other field's answer by its label, as `norm` reads it.
    pub answers: Vec<(String, String)>,
}

/// A label or a question as both are compared: lower case, no `*` or `?`, single spaces.
pub fn norm(s: &str) -> String {
    let s = s.to_lowercase().replace(['*', '?'], " ");
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The answer to the field labelled `label`, if one is given.
fn answer<'a>(answers: &'a [(String, String)], label: &str) -> Option<&'a str> {
    let label = norm(label);
    (answers.iter().rev())
        .find(|(q, _)| *q == label)
        .map(|(_, a)| a.as_str())
}

/// The application `draft` makes, his name and mail from `candidate` (profile.edn's), its CV under `repo`, its answers from
/// its `answers`, then from `given` (`--answer`, each `<question>=<answer>`), which come last so they win.
pub fn application(
    draft: &Value,
    candidate: &Value,
    repo: &Path,
    given: &[&str],
) -> Result<Application, String> {
    let id = draft["id"].as_str().unwrap_or("the draft");
    let name = candidate["name"].as_str().unwrap_or_default().trim();
    let (first, last) = name.split_once(' ').ok_or(format!(
        "profile.edn's candidate name « {name} » is no first and last name"
    ))?;
    let email = candidate["email-pro"].as_str().unwrap_or_default();
    if !email.contains('@') {
        return Err("profile.edn's candidate has no email-pro".into());
    }
    let cv = draft["attach"][0]
        .as_str()
        .ok_or(format!("{id} attaches no CV"))?;
    let cv = repo.join(cv);
    if !cv.is_file() {
        return Err(format!("{id}'s CV is not there: {}", cv.display()));
    }
    let message = draft["body"].as_str().unwrap_or_default().trim();
    if message.is_empty() {
        return Err(format!("{id} has no text"));
    }
    // a draft's own answers are plain text: one still holding a store marker (« m:contacts:x ») is skipped, never typed
    let kept = (draft["answers"].as_array().into_iter().flatten()).filter_map(|a| {
        let (q, a) = (a["question"].as_str()?, a["answer"].as_str()?);
        (!a.contains('\u{E000}')).then(|| (norm(q), a.trim().to_string()))
    });
    let mut answers: Vec<(String, String)> = kept.collect();
    for g in given {
        let (q, a) = g
            .split_once('=')
            .ok_or(format!("--answer {g}: write it '<question>=<answer>'"))?;
        answers.push((norm(q), a.trim().to_string()));
    }
    Ok(Application {
        first: first.into(),
        last: last.trim().into(),
        email: email.into(),
        cv,
        message: message.into(),
        answers,
    })
}

/// Why `draft` may not be sent at all: only an approved draft goes, so a sent one never goes twice. Asked before any Chrome.
pub fn unapproved(draft: &Value) -> Option<String> {
    let id = draft["id"].as_str().unwrap_or("the draft");
    let status = draft["status"].as_str().unwrap_or_default();
    (status != "approved").then(|| format!("{id} is {status}, not approved: nothing sent"))
}

/// Why `draft` may not be sent to the page of `company`, if it may not: the company named in its `to` as whole words, so a
/// page of « Data » never takes a draft to Datadog.
/// ponytail: a company named by a word another company's name holds (« AI » in « Mistral AI ») still matches; the job's
/// own URL on the lead if that ever shows.
pub fn refused(draft: &Value, company: &str) -> Option<String> {
    if let Some(why) = unapproved(draft) {
        return Some(why);
    }
    let id = draft["id"].as_str().unwrap_or("the draft");
    let words = |s: &str| -> Vec<String> {
        (s.to_lowercase().split(|c: char| !c.is_alphanumeric()))
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect()
    };
    let (to, name) = (
        words(draft["to"].as_str().unwrap_or_default()),
        words(company),
    );
    if name.is_empty() || !to.windows(name.len()).any(|w| w == name.as_slice()) {
        return Some(format!(
            "wrong posting, nothing sent: the page is « {company} »'s and {id} is to « {} »",
            draft["to"].as_str().unwrap_or_default()
        ));
    }
    None
}

/// A headless Chrome of its own profile, driven over its DevTools pipe (fds 3 and 4), as notion_browser.rs drives one.
struct Chrome {
    child: Child,
    to: ChildStdin,
    from: BufReader<ChildStdout>,
    id: u64,
    session: Option<String>,
    dir: PathBuf,
    /// Set once Drop reaped Chrome: the limit's kill then never reaches a pid reused by another group.
    reaped: Arc<AtomicBool>,
}

impl Chrome {
    fn start(dir: &Path) -> Result<Chrome, String> {
        let bin = dash_surface::pdf::chrome()
            .ok_or("no Chrome: install Google Chrome or chrome-headless-shell, or set CHROME")?;
        let born = Instant::now();
        // One of the machine's browser slots (bin/browser-slot), taken for this process: freed when dash exits. Its wait
        // counts inside LIMIT: five slots held elsewhere end the run, never hang it.
        let slot = dash_surface::pdf::slot_script();
        if slot.exists() {
            let mut waiting = Command::new(&slot)
                .arg(std::process::id().to_string())
                .stdout(Stdio::null())
                .spawn()
                .map_err(|e| format!("bin/browser-slot: {e}"))?;
            let taken = loop {
                if let Some(s) = waiting.try_wait().map_err(|e| e.to_string())? {
                    break s;
                }
                if born.elapsed() > LIMIT {
                    let _ = waiting.kill();
                    let _ = waiting.wait();
                    return Err(format!("no browser slot in {} s", LIMIT.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(100));
            };
            if !taken.success() {
                return Err(format!("bin/browser-slot: {taken}"));
            }
        }
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
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
            ])
            .arg(format!("--user-data-dir={}", dir.join("profile").display()))
            .arg("about:blank")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // its own group, so the limit's kill ends its helpers too
            .process_group(0)
            .spawn()
            .map_err(|e| format!("{bin}: {e}"))?;
        // The limit: a Chrome that hangs is killed, so the blocked read below ends and the run says so.
        let pid = child.id() as libc::pid_t;
        let reaped = Arc::new(AtomicBool::new(false));
        let watched = reaped.clone();
        let left = LIMIT.saturating_sub(born.elapsed());
        std::thread::spawn(move || {
            std::thread::sleep(left);
            if !watched.load(Ordering::SeqCst) {
                // SAFETY: killpg reads no memory; the group is still Chrome's, not reaped yet (Drop sets the flag first).
                unsafe { libc::killpg(pid, libc::SIGKILL) };
            }
        });
        let mut c = Chrome {
            to: child.stdin.take().ok_or("Chrome's pipe")?,
            from: BufReader::new(child.stdout.take().ok_or("Chrome's pipe")?),
            child,
            id: 0,
            session: None,
            dir: dir.to_path_buf(),
            reaped,
        };
        let target = c.send("Target.createTarget", json!({"url": "about:blank"}))?;
        let attached = c.send(
            "Target.attachToTarget",
            json!({"targetId": target["targetId"], "flatten": true}),
        )?;
        c.session = attached["sessionId"].as_str().map(str::to_string);
        Ok(c)
    }

    /// One command's result; the events in between are dropped.
    fn send(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.id += 1;
        let mut msg = json!({"id": self.id, "method": method, "params": params});
        if let Some(s) = &self.session {
            msg["sessionId"] = json!(s);
        }
        let gone = |e: std::io::Error| format!("Chrome ended during {method}: {e}");
        self.to
            .write_all(msg.to_string().as_bytes())
            .map_err(gone)?;
        self.to.write_all(b"\0").map_err(gone)?;
        self.to.flush().map_err(gone)?;
        loop {
            let mut raw = Vec::new();
            if self.from.read_until(0, &mut raw).map_err(gone)? == 0 {
                return Err(format!(
                    "Chrome ended during {method} (past {} s it is killed)",
                    LIMIT.as_secs()
                ));
            }
            raw.pop();
            let answer: Value = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
            if answer["id"] == json!(self.id) {
                return match answer.get("error") {
                    Some(e) => Err(format!("{method}: {e}")),
                    None => Ok(answer["result"].clone()),
                };
            }
        }
    }

    fn eval(&mut self, js: &str) -> Result<Value, String> {
        let r = self.send(
            "Runtime.evaluate",
            json!({"expression": js, "returnByValue": true, "awaitPromise": true}),
        )?;
        match r.get("exceptionDetails") {
            Some(e) => Err(format!("the page refused a script: {e}")),
            None => Ok(r["result"]["value"].clone()),
        }
    }

    /// `js`'s value once it is truthy, asked every 200 ms for `wait` at most; a page swapping documents answers nothing
    /// meanwhile, read as not yet.
    fn until(&mut self, js: &str, wait: Duration) -> Option<Value> {
        let end = Instant::now() + wait;
        while Instant::now() < end {
            match self.eval(js) {
                Ok(v) if !(v.is_null() || v == json!(false) || v == json!("")) => return Some(v),
                _ => std::thread::sleep(Duration::from_millis(200)),
            }
        }
        None
    }

    /// `files` set on the file input `selector` matches, as a person picking them does: Chrome fires its input and change.
    fn upload(&mut self, selector: &str, files: &[&Path]) -> Result<(), String> {
        let found = self.send(
            "Runtime.evaluate",
            json!({"expression": format!("document.querySelector({})", json!(selector))}),
        )?;
        let object = found["result"]["objectId"]
            .as_str()
            .ok_or(format!("no {selector} on the page"))?;
        let files: Vec<String> = files.iter().map(|f| f.display().to_string()).collect();
        self.send(
            "DOM.setFileInputFiles",
            json!({"files": files, "objectId": object}),
        )?;
        Ok(())
    }
}

impl Drop for Chrome {
    fn drop(&mut self) {
        self.reaped.store(true, Ordering::SeqCst);
        // SAFETY: as in start; the group is ours, from process_group(0)
        unsafe { libc::killpg(self.child.id() as libc::pid_t, libc::SIGKILL) };
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// How a field is named: its label, else its select's or fieldset's, else its aria-label or id; no trailing `*`.
macro_rules! label_js {
    () => {
        r#"e => ((e.labels && e.labels[0]) || e.closest('.select__container, .field-wrapper, fieldset')?.querySelector('label, legend'))
    ?.innerText?.replace(/\*\s*$/, '').trim() || e.getAttribute('aria-label') || e.id || e.name || 'a field'"#
    };
}

/// The page's own setter, so a React form (Greenhouse's) takes the value as typed.
const PUT: &str = r#"((id, v) => {
  const el = document.getElementById(id);
  if (!el) return false;
  Object.getOwnPropertyDescriptor(Object.getPrototypeOf(el), 'value').set.call(el, v);
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return true;
})"#;

/// The fields answered by label: every input or textarea with an id but his name, mail, the files and the security code;
/// `combo` for a select (its typed box).
const FIELDS: &str = concat!(
    "(() => { const label = ",
    label_js!(),
    r#";
  const skip = new Set(['first_name', 'last_name', 'email', 'resume', 'cover_letter', 'resume_text', 'cover_letter_text']);
  return [...document.querySelectorAll('#application-form input[id], #application-form textarea[id]')]
    .filter(e => !skip.has(e.id) && e.type !== 'file' && e.type !== 'hidden' && !e.id.startsWith('security-input'))
    // a field of its own label only: the phone's country search box has none, and its fieldset's « Phone » is not its name
    .filter(e => e.labels && e.labels.length)
    .map(e => ({ id: e.id, combo: e.getAttribute('role') === 'combobox', label: label(e) }));
})()"#
);

/// The middle of select `id`'s control (its whole box, or the typed box where none), scrolled into view: where a click opens it.
const CONTROL: &str = r#"(id => {
  const box = document.getElementById(id);
  const ctl = box.closest('.select__control') || box.closest('.select__container')?.querySelector('.select__control') || box;
  ctl.scrollIntoView({ block: 'center' });
  const r = ctl.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
})"#;

/// The open select's option for `want`: an exact match, else the first starting so, clicked; its text, or false.
const PICK: &str = r#"(want => {
  const n = s => s.toLowerCase().replace(/[*?]/g, ' ').replace(/\s+/g, ' ').trim();
  // shown ones only: the phone's country picker keeps its whole hidden list (Afghanistan +93…) as role=option
  const opts = [...document.querySelectorAll('.select__option, [role=option]')].filter(o => o.offsetParent !== null);
  // exact, else starting so, else holding it as whole words (a country shown with its flag or its dialling code)
  const o = opts.find(o => n(o.innerText) === n(want)) || opts.find(o => n(o.innerText).startsWith(n(want))) ||
    opts.find(o => (' ' + n(o.innerText) + ' ').includes(' ' + n(want) + ' '));
  if (!o) return false;
  o.click();
  return o.innerText.trim();
})"#;

/// Each field read back, and the labels of the required fields still empty. A select is read by its hidden required input:
/// its typed box stays empty once an option is picked.
const READ: &str = concat!(
    "(() => { const label = ",
    label_js!(),
    r#";
  const f = document.getElementById('application-form');
  const v = id => document.getElementById(id);
  const file = id => v(id)?.files?.[0]?.name ?? null;
  // a select holds two inputs, its visible one and its hidden required one: named once
  const empty = [...new Set([...f.querySelectorAll('input, textarea, select')]
    .filter(e => e.getAttribute('role') !== 'combobox')
    .filter(e => e.required || e.getAttribute('aria-required') === 'true')
    .filter(e => e.type === 'file' ? !e.files.length : !String(e.value).trim())
    .map(label))];
  const skip = new Set(['first_name', 'last_name', 'email', 'resume', 'cover_letter', 'resume_text', 'cover_letter_text']);
  const others = [...f.querySelectorAll('input[id], textarea[id]')]
    .filter(e => !skip.has(e.id) && e.type !== 'file' && e.type !== 'hidden' && !e.id.startsWith('security-input'))
    .map(e => [label(e), e.getAttribute('role') === 'combobox'
      ? (e.closest('.select__container, .select')?.querySelector('.select__single-value')?.innerText ?? '') : e.value])
    .filter(([, value]) => value);
  return { first_name: v('first_name')?.value ?? null, last_name: v('last_name')?.value ?? null,
    email: v('email')?.value ?? null, resume: file('resume'), cover_letter: file('cover_letter'), others, empty };
})()"#
);

/// After a submit: Greenhouse's confirmation (the form gone and a thank-you shown, or its /confirmation page) with the page's
/// text, or its security code asked.
const SUBMITTED: &str = r#"(() => {
  const text = document.body?.innerText?.trim() ?? '';
  const done = location.pathname.endsWith('/confirmation') ||
    (!document.getElementById('application-form') && /thank you|application (has been )?(received|submitted)|merci/i.test(text));
  if (done) return { text };
  if (document.querySelector('input[id^="security-input"]')) return { code: true };
  return false;
})()"#;

/// The page's company, from Greenhouse's title « Job Application for <role> at <company> ».
/// The last « at » (« chez » on a French board): a role may hold one (« Head of Data at Scale at Acme »). A board's suffix
/// goes: « Mirakl - Labs » is Mirakl.
const COMPANY: &str =
    r#"((document.title.match(/.* (?:at|chez) (.+)$/) || [])[1] || '').split(' - ')[0].trim()"#;

/// The form kept: a mark set on it is still there. A board that loads its language after hydrating remounts the form, which
/// drops every value typed before (Mirakl's French board, 2026-11-12).
const KEPT: &str = r#"(() => {
  const f = document.getElementById('application-form');
  if (f && f.dataset.dashKept) return true;
  if (f) f.dataset.dashKept = '1';
  return false;
})()"#;
/// How long the form must stay the same before it is filled.
const STILL: Duration = Duration::from_millis(1500);

/// The form ready to take values: rendered, and hydrated where React runs it (a node holds React's keys once it has), so no
/// value set before hydration is lost; a page with no script (a saved form) is ready once rendered.
const READY: &str = r#"(() => {
  const f = document.getElementById('first_name');
  return !!document.getElementById('application-form') && !!f &&
    (!document.querySelector('script[src]') || Object.keys(f).some(k => k.startsWith('__react')));
})()"#;

/// How long a select's options may take to show (a city's are looked up as typed), and the emailed code to come.
const OPTIONS: Duration = Duration::from_secs(10);
const CODE: Duration = Duration::from_secs(120);

/// What a run read on the page: its title and each field after the fill.
pub struct Filled {
    pub title: String,
    pub company: String,
    pub fields: Value,
    /// Each select whose answer matched no option.
    pub unpicked: Vec<String>,
}

/// A send: the draft `refused` checks on the page's company, and where the emailed security code is read, given the minute
/// the submit went (« 2026-11-12 05:41 UTC »).
pub struct Sending<'a> {
    pub draft: &'a Value,
    /// The outbound gate (`judged`): why this application's words may not leave, asked before any Chrome starts.
    pub gate: &'a dyn Fn(&Application, &Value) -> Option<String>,
    pub code: &'a dyn Fn(&str) -> Result<String, String>,
}

/// The one outbound gate on an application's words (task outbound-gate): `dash_hooks::outbound`'s `Gate::application`, on
/// the model's draft, his banned rows and Jev (`refusal_application`).
pub fn judged(app: &Application, draft: &Value) -> Option<String> {
    let answers: Vec<&str> = app.answers.iter().map(|(_, a)| a.as_str()).collect();
    let id = draft["id"].as_str().unwrap_or_default();
    crate::claude_hooks::refusal_application(&app.message, &answers, id)
}

/// The security code in a Greenhouse mail's text: « …on your application:  11283510After you enter the code… », its 8
/// characters (the form's 8 boxes) run straight into the next word, so no word boundary ends it (Mirakl's mail, 2026-11-12).
pub fn code_in(text: &str) -> Option<String> {
    let re = regex::Regex::new(r"(?i)(?:application|code)\s*:\s*([A-Za-z0-9]{8})").ok()?;
    re.captures(text).map(|c| c[1].to_string())
}

/// The emailed security code from his inbox: the newest Greenhouse « security code » mail received since `since`.
pub fn mailed_code(since: &str) -> Result<String, String> {
    let since = since.replace(' ', "T");
    let end = Instant::now() + CODE;
    loop {
        let rows = crate::mail::search("security code")?;
        let fresh = rows
            .iter()
            .filter(|r| r.at.get(..16).is_some_and(|at| at >= &since[..16]));
        if let Some(code) = fresh
            .filter_map(|r| code_in(&format!("{} {}", r.subject, r.preview)))
            .next()
        {
            return Ok(code);
        }
        if Instant::now() > end {
            return Err(format!(
                "no security code mail in {} s since {since}",
                CODE.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

/// `app` filled into the Greenhouse form at `url`, in a Chrome whose profile lives under `dir`, then, with `send`, submitted
/// once its words pass the outbound gate (asked first: a refused text opens no page) and its draft passes `refused` on the
/// page's company: Ok with what was read and the confirmation page's text.
pub fn fill(
    url: &str,
    app: &Application,
    dir: &Path,
    send: Option<Sending>,
) -> Result<(Filled, Option<String>), String> {
    if let Some(why) = send.as_ref().and_then(|s| (s.gate)(app, s.draft)) {
        return Err(format!("outbound gate: {why}: nothing sent"));
    }
    let mut c = Chrome::start(dir)?;
    c.send("Page.enable", json!({}))?;
    // a headless page has no focus, so a React select never hears it focused and never opens its menu
    c.send(
        "Emulation.setFocusEmulationEnabled",
        json!({"enabled": true}),
    )?;
    c.send(
        "Emulation.setDeviceMetricsOverride",
        json!({"width": 1280, "height": 900, "deviceScaleFactor": 1, "mobile": false}),
    )?;
    let nav = c.send("Page.navigate", json!({"url": url}))?;
    if let Some(e) = nav["errorText"].as_str() {
        return Err(format!("{url} did not load: {e}"));
    }
    c.until(READY, SHOW).ok_or(format!(
        "no Greenhouse application form ready at {url} in {} s",
        SHOW.as_secs()
    ))?;
    // the form still, as one mark outlives a pause: a remount in between starts the wait again
    let end = Instant::now() + SHOW;
    c.eval(KEPT)?;
    loop {
        std::thread::sleep(STILL);
        if c.eval(KEPT)? == json!(true) {
            break;
        }
        if Instant::now() > end {
            return Err(format!(
                "the form at {url} kept being redrawn for {} s",
                SHOW.as_secs()
            ));
        }
    }
    for (id, v) in [
        ("first_name", &app.first),
        ("last_name", &app.last),
        ("email", &app.email),
    ] {
        c.eval(&format!("{PUT}({}, {})", json!(id), json!(v)))?;
    }
    c.upload("#resume", &[&app.cv])?;
    // the message as a cover letter file: the « Enter manually » box is React's alone, a file input every form has
    let letter = dir.join("Cover_letter.txt");
    std::fs::write(&letter, &app.message).map_err(|e| format!("{}: {e}", letter.display()))?;
    let has_letter = c.eval("!!document.getElementById('cover_letter')")? == json!(true);
    if has_letter {
        c.upload("#cover_letter", &[&letter])?;
    }
    let fields = c.eval(FIELDS)?;
    let mut unpicked = vec![];
    for f in fields.as_array().into_iter().flatten() {
        let (id, label) = (
            f["id"].as_str().unwrap_or_default(),
            f["label"].as_str().unwrap_or_default(),
        );
        let Some(want) = answer(&app.answers, label) else {
            continue;
        };
        if f["combo"] != json!(true) {
            c.eval(&format!("{PUT}({}, {})", json!(id), json!(want)))?;
            continue;
        }
        // a select: clicked open and typed, as a person does, then its option picked
        // twice at most: a click while the last select's menu still closes opens nothing
        let mut picked = false;
        for _ in 0..2 {
            let at = c.eval(&format!("{CONTROL}({})", json!(id)))?;
            let (x, y) = (
                at["x"].as_f64().unwrap_or(0.0),
                at["y"].as_f64().unwrap_or(0.0),
            );
            for kind in ["mousePressed", "mouseReleased"] {
                let click =
                    json!({"type": kind, "x": x, "y": y, "button": "left", "clickCount": 1});
                c.send("Input.dispatchMouseEvent", click)?;
            }
            c.send("Input.insertText", json!({"text": want}))?;
            picked = c
                .until(&format!("{PICK}({})", json!(want)), OPTIONS)
                .is_some();
            // the menu closes before the next select is clicked
            std::thread::sleep(Duration::from_millis(400));
            if picked {
                break;
            }
            c.eval(&format!("{PUT}({}, '')", json!(id)))?;
        }
        if !picked {
            unpicked.push(format!("{label} (no option « {want} »)"));
        }
    }
    let read = Filled {
        title: c
            .eval("document.title")?
            .as_str()
            .unwrap_or_default()
            .trim()
            .into(),
        company: c.eval(COMPANY)?.as_str().unwrap_or_default().into(),
        fields: c.eval(READ)?,
        unpicked: unpicked.clone(),
    };
    // an uploaded file shows as its name on the form once Greenhouse swapped its input for a chip
    let mut read = read;
    let files = [
        ("resume", app.cv.file_name()),
        ("cover_letter", letter.file_name()),
    ];
    for (key, name) in files {
        let name = name
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let shown = format!(
            "document.getElementById('application-form').innerText.includes({})",
            json!(name)
        );
        if read.fields[key].is_null() && c.eval(&shown)? == json!(true) {
            read.fields[key] = json!(name);
        }
    }
    let Some(Sending { draft, code, .. }) = send else {
        return Ok((read, None));
    };
    if let Some(why) = refused(draft, &read.company) {
        return Err(why);
    }
    if !has_letter {
        return Err(
            "this form has no cover letter field, so the draft's text cannot go: nothing sent"
                .into(),
        );
    }
    let empty: Vec<&str> = (read.fields["empty"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect();
    if !empty.is_empty() || !unpicked.is_empty() {
        return Err(format!(
            "the form asks {}, which the draft does not answer: nothing sent (--answer '<question>=<answer>' gives one)",
            [empty.join(", "), unpicked.join(", ")].join(" ").trim()
        ));
    }
    let since = now();
    let submit = "document.querySelector('#application-form [type=submit]').click(), true";
    c.eval(submit)?;
    // what the page shows, so a submit that went nowhere says why (a field the page refused, a challenge)
    let shown = |c: &mut Chrome| {
        let js = "location.href + ' : ' + (document.body?.innerText ?? '').trim().slice(0, 400)";
        c.eval(js)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default()
    };
    let unconfirmed = |c: &mut Chrome| {
        format!(
            "submitted, but no confirmation page came: it may have gone, so nothing is recorded and nothing is sent again; the page shows {}",
            shown(c)
        )
    };
    let Some(mut after) = c.until(SUBMITTED, SHOW) else {
        return Err(unconfirmed(&mut c));
    };
    if after["code"] == json!(true) {
        let code = code(&since).map_err(|e| format!("its security code: {e}; nothing sent"))?;
        for (i, ch) in code.chars().enumerate() {
            c.eval(&format!(
                "{PUT}({}, {})",
                json!(format!("security-input-{i}")),
                json!(ch.to_string())
            ))?;
        }
        c.eval(submit)?;
        // the confirmation alone: the code's boxes stay shown until the page moves on
        let confirmed = format!("(r => !!r && !!r.text && r)({SUBMITTED})");
        let Some(next) = c.until(&confirmed, SHOW) else {
            return Err(unconfirmed(&mut c));
        };
        after = next;
    }
    let Some(text) = after["text"].as_str() else {
        return Err(unconfirmed(&mut c));
    };
    Ok((read, Some(text.to_string())))
}

/// Now, as `dash_core::stamp` writes it: « 2026-11-12 05:41 UTC ».
fn now() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    dash_core::stamp(ms)
}

/// The run's report: each field as read back.
pub fn report(url: &str, read: &Filled) -> String {
    let f = &read.fields;
    let s = |k: &str| f[k].as_str().unwrap_or("(none)").to_string();
    let empty: Vec<&str> = (f["empty"].as_array().into_iter().flatten())
        .filter_map(Value::as_str)
        .collect();
    let empty = match empty.is_empty() {
        true => "none".to_string(),
        false => empty.join(", "),
    };
    let others: String = (f["others"].as_array().into_iter().flatten())
        .map(|o| {
            format!(
                "\n{}: {}",
                o[0].as_str().unwrap_or_default(),
                o[1].as_str().unwrap_or_default()
            )
        })
        .collect();
    format!(
        "{}\n{url}\nfirst name: {}\nlast name: {}\nemail: {}\nCV: {}\nmessage: {}{others}\nrequired and empty: {empty}{}",
        read.title,
        s("first_name"),
        s("last_name"),
        s("email"),
        s("resume"),
        s("cover_letter"),
        match read.unpicked.is_empty() {
            true => String::new(),
            false => format!("\nnot picked: {}", read.unpicked.join(", ")),
        },
    )
}

/// Draft `id` recorded sent through the older app `older` (`dash draft status`), then an outbound line holding the confirmation
/// page's text (`dash model outbound add`). Err once the application left: it says so, so nobody sends it again.
pub fn record(older: &str, id: &str, url: &str, confirmation: &str) -> Result<(), String> {
    let run = |args: &[&str]| dash_process::run(older, args, None, OLDER_CALL);
    run(&["draft", "status", id, "sent"]).map_err(|e| {
        format!("SENT, but {e}: record it with dash draft status {id} sent; never send it again. The confirmation page: {confirmation}")
    })?;
    let at = now();
    let line = json!({
        "date": &at[..10], "at": at, "channel": "form", "tool": "dash apply greenhouse",
        "to": url, "draft": id, "text": confirmation,
    });
    run(&["model", "outbound", "add", &line.to_string()]).map_err(|e| {
        format!("SENT and {id} marked sent, but its outbound line is not logged ({e}): dash model outbound add '{line}'")
    })?;
    Ok(())
}

inventory::submit! { crate::usage::Command::usage("dash apply greenhouse <job-url> --draft <id> [--answer '<question>=<answer>']... [--send]") }
/// The command: draft `id` and his profile read from the older app's file, the form filled at `url`, and with `send`
/// submitted and recorded. Prints the fields as read back, then the confirmation page's text; exits 1 with why otherwise.
pub fn run(url: &str, id: &str, flags: &[&str], mut out: std::fs::File) {
    let db = crate::tasks::older_db();
    let went = (|| {
        let mut send = false;
        let mut given = vec![];
        let mut rest = flags.iter();
        while let Some(f) = rest.next() {
            match *f {
                "--send" => send = true,
                "--answer" => {
                    given.push(*rest.next().ok_or("--answer takes '<question>=<answer>'")?)
                }
                other => {
                    return Err(format!(
                        "{other}: dash apply greenhouse <job-url> --draft <id> [--answer '<question>=<answer>']... [--send]"
                    ));
                }
            }
        }
        let draft = dash_store::older_drafts::drafts(&db, Some(id))?;
        let draft = draft
            .get(0)
            .cloned()
            .ok_or(format!("{id}: no such draft"))?;
        let candidate = dash_store::older_drafts::candidate(&db)?;
        let app = application(&draft, &candidate, &crate::tasks::repo(), &given)?;
        if let Some(why) = unapproved(&draft).filter(|_| send) {
            return Err(why);
        }
        // Chrome's Drop removes it
        let dir = std::env::temp_dir().join(format!("dash-apply-{}", std::process::id()));
        let sending = Sending {
            draft: &draft,
            gate: &judged,
            code: &mailed_code,
        };
        let (read, confirmation) = fill(url, &app, &dir, send.then_some(sending))?;
        let mut said = report(url, &read);
        match confirmation {
            None => said.push_str("\nDRY RUN, nothing sent: --send submits it"),
            Some(c) => {
                record(&crate::tasks::older_app().bin, id, url, &c)?;
                said.push_str(&format!(
                    "\nsent and recorded: {id} -> sent\nconfirmation: {c}"
                ));
            }
        }
        Ok::<_, String>(said)
    })();
    match went {
        Ok(said) => writeln!(out, "{said}").expect("stdout"),
        Err(e) => {
            eprintln!("apply: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/greenhouse")
    }

    fn draft(dir: &Path) -> (Value, Value) {
        std::fs::write(dir.join("cv.pdf"), b"%PDF-1.4\n%%EOF\n").unwrap();
        let draft = json!({"id": "acme-intro", "status": "approved", "to": "Acme (formulaire de candidature)",
            "attach": ["cv.pdf"], "body": "Hello,\n\nI saw your posting.\n\nNicolas HA\n",
            "answers": [{"question": "Do you speak French fluently ?", "answer": "Yes"}]});
        let candidate = json!({"name": "Nicolas HA", "email-pro": "nicolas@klar.im"});
        (draft, candidate)
    }

    /// The pure parts: his name split, the CV found, the text kept; an unapproved draft or another company's page refused.
    #[test]
    fn an_application_is_his_name_mail_cv_and_text_and_only_an_approved_draft_to_its_company_goes()
    {
        let _budget = dash_meter::memory::TestBudget::new();
        let dir = dash_meter::TestDir::new("apply-application");
        let (d, c) = draft(&dir.0);
        let app = application(&d, &c, &dir.0, &["Privacy *=Acknowledge"]).expect("an application");
        assert_eq!((app.first.as_str(), app.last.as_str()), ("Nicolas", "HA"));
        assert_eq!(app.email, "nicolas@klar.im");
        assert_eq!(app.cv, dir.0.join("cv.pdf"));
        assert!(app.message.starts_with("Hello,") && app.message.ends_with("Nicolas HA"));
        assert_eq!(refused(&d, "Acme"), None);
        let other = refused(&d, "Mirakl").expect("another company's page");
        assert!(other.starts_with("wrong posting"), "{other}");
        // whole words: a page of « Ac » never takes a draft to Acme
        assert!(refused(&d, "Ac").is_some());
        assert_eq!(unapproved(&d), None);
        // a label is matched with no case, star or question mark; --answer wins over the draft's own
        let answers = &app.answers;
        assert_eq!(
            answer(answers, "Do you speak French fluently ?*"),
            Some("Yes")
        );
        assert_eq!(answer(answers, "privacy"), Some("Acknowledge"));
        assert_eq!(answer(answers, "Phone"), None);
        let over = application(&d, &c, &dir.0, &["Do you speak French fluently=Oui"]).unwrap();
        assert_eq!(
            answer(&over.answers, "Do you speak French fluently ?"),
            Some("Oui")
        );
        assert!(application(&d, &c, &dir.0, &["no equals"]).is_err());
        // the code in Greenhouse's mail
        let mail = "Security code for your application to Mirakl Copy and paste this code into the security code field on your application: Ab3dE5gH";
        assert_eq!(code_in(mail).as_deref(), Some("Ab3dE5gH"));
        assert_eq!(code_in("no code here"), None);
        let real = "Copy and paste this code into the security code field on your application:  11283510After you enter the code, resubmit your application.";
        assert_eq!(code_in(real).as_deref(), Some("11283510"));
        let sent = json!({"id": "acme-intro", "status": "sent", "to": "Acme"});
        assert_eq!(
            refused(&sent, "Acme").unwrap(),
            "acme-intro is sent, not approved: nothing sent"
        );
        let no_cv = json!({"id": "x", "body": "Hi"});
        assert_eq!(
            application(&no_cv, &c, &dir.0, &[]).unwrap_err(),
            "x attaches no CV"
        );
    }

    /// Done-when 1 (on Mirakl's saved form) and 2 (on a stand-in form) of rust-apply-greenhouse, one Chrome each: a dry run
    /// fills name, mail, CV and message and submits nothing, naming the required fields the draft leaves empty; --send
    /// submits, reads the confirmation page, and records the draft sent with that page's text through the older app.
    #[test]
    fn a_greenhouse_form_is_filled_dry_and_a_stand_in_is_sent_and_recorded() {
        let _budget = dash_meter::memory::TestBudget::new();
        let dir = dash_meter::TestDir::new("apply-greenhouse");
        let (d, c) = draft(&dir.0);
        let app = application(&d, &c, &dir.0, &["Privacy=Acknowledge"]).unwrap();
        // the saved form runs no script, so its selects open no options: asked none, it waits for none
        let plain = application(&d, &c, &dir.0, &[]).unwrap();
        let mailed = |since: &str| {
            assert!(since.ends_with(" UTC"), "{since}");
            Ok("AB12CD34".to_string())
        };
        // the gate is asked the application's own words before any page opens; a refusal sends nothing (outbound-gate 1)
        let asked = std::cell::RefCell::new(vec![]);
        // The gate itself, dash_hooks::outbound's, nothing planted but the world it reads: two of his banned rows (a rate,
        // banned cold; a robotic aside, banned everywhere), the draft as `dash model draft` gives it, and Jev's answer.
        let typed_by_him = |_: &str| {
            Ok((
                json!({"reads_ai_written": {"noul": 0.1}, "too_long": {"noul": 0.1}}),
                json!({"reads_ai_written_max": 0.5, "too_long_max": 0.6}),
            ))
        };
        let rows = r#"[{"regex": "\\btjm\\b|\\b\\d{3,4} ?€", "label": "a rate", "scope": "~:cold", "instead": "Leave the rate."},
            {"regex": "(?i)\\band that stays\\b", "label": "« and that stays », a robotic aside", "instead": "Drop it."}]"#;
        // the one tell of this checkout: the em-dash
        let tells = dir.0.join("private/dashboard/priv");
        std::fs::create_dir_all(&tells).expect("its folder");
        let em_dash = r#"{"rows": [{"regex": "—", "label": "an em-dash", "instead": "a period"}]}"#;
        std::fs::write(tells.join("ai_isms.json"), em_dash).expect("the tells");
        let gate = dash_hooks::outbound::gate(rows, &dir.0, &typed_by_him).expect("the gate");
        let gate_on = |model: Value| {
            let (gate, asked) = (&gate, &asked);
            move |a: &Application, _: &Value| {
                asked.borrow_mut().push(a.message.clone());
                let answers: Vec<&str> = a.answers.iter().map(|(_, a)| a.as_str()).collect();
                let why = gate.application(&a.message, &answers, &model);
                (!why.is_empty()).then(|| why.join("; "))
            }
        };
        let (cold, warm) = (
            json!({"id": "acme-intro", "status": "approved", "cold": true, "first": true}),
            json!({"id": "acme-intro", "status": "approved", "cold": false, "first": false}),
        );
        let pass = gate_on(warm.clone());
        let sending = || Sending {
            draft: &d,
            gate: &pass,
            code: &mailed,
        };
        // the scope read: the model says the draft is cold, so its rate refuses the send, and no Chrome profile is made
        let rate = Application {
            message: "Hello,\n\nMy TJM is 650 €.\n\nNicolas HA".into(),
            answers: app.answers.clone(),
            ..application(&d, &c, &dir.0, &[]).unwrap()
        };
        let gated = |app: &Application, model: &Value| {
            let gate = gate_on(model.clone());
            let held = Sending {
                draft: &d,
                gate: &gate,
                code: &mailed,
            };
            let said = fill("file:///no-page", app, &dir.0.join("gated"), Some(held));
            said.err().expect("no page there")
        };
        let no = gated(&rate, &cold);
        println!("cold draft, a rate in the message: {no}");
        assert!(
            no.starts_with("outbound gate: banned phrase")
                && no.contains("a rate")
                && no.ends_with(": nothing sent"),
            "{no}"
        );
        assert!(!dir.0.join("gated").exists(), "no Chrome profile was made");
        // the same words in a draft the model says is warm pass the gate: the send goes on to the page, which is not there
        let on = gated(&rate, &warm);
        println!("warm draft, the same message: {on}");
        assert!(on.starts_with("file:///no-page did not load"), "{on}");
        // a draft no longer open is judged the strictest, whatever it says
        let sent = json!({"id": "acme-intro", "status": "sent", "cold": false, "first": false});
        assert!(gated(&rate, &sent).contains("a rate"));
        // an answer: a rate the form asks for passes even cold, a phrase banned everywhere does not
        let answering = |answer: &str| Application {
            answers: vec![("salary expectations".into(), answer.into())],
            ..application(&d, &c, &dir.0, &[]).unwrap()
        };
        let on = gated(&answering("650 € a day"), &cold);
        assert!(on.starts_with("file:///no-page did not load"), "{on}");
        let no = gated(&answering("In France, and that stays."), &cold);
        println!("an answer holding a phrase banned everywhere: {no}");
        assert!(
            no.contains("and that stays") && no.ends_with(": nothing sent"),
            "{no}"
        );
        let saved = format!("file://{}", fixtures().join("form.html").display());
        let (read, sent) = fill(&saved, &plain, &dir.0.join("dry"), None).expect("a dry run");
        let said = report(&saved, &read);
        println!("{said}");
        assert!(sent.is_none());
        assert!(
            said.starts_with("Job Application for Account Executive, Mirakl Connect"),
            "{said}"
        );
        for line in [
            "first name: Nicolas",
            "last name: HA",
            "email: nicolas@klar.im",
            "CV: cv.pdf",
            "message: Cover_letter.txt",
        ] {
            assert!(said.contains(line), "{line} in {said}");
        }
        assert!(
            said.contains("required and empty: Country, Phone, Location (City)"),
            "{said}"
        );
        // --send on the saved form refuses before any click: the page is Mirakl's, the draft Acme's
        let no = fill(&saved, &plain, &dir.0.join("refused"), Some(sending()))
            .err()
            .expect("refused");
        assert!(no.starts_with("wrong posting"), "{no}");

        let older = dir.0.join("older-dash");
        let calls = dir.0.join("calls");
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n",
            calls.display()
        );
        std::fs::write(&older, script).unwrap();
        std::fs::set_permissions(&older, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let stand_in = format!("file://{}", fixtures().join("stand-in.html").display());
        let (read, sent) =
            fill(&stand_in, &app, &dir.0.join("send"), Some(sending())).expect("sent");
        assert_eq!(read.company, "Acme");
        // the draft's answer typed, --answer's picked in the select, the mailed code taken: else no confirmation
        let said = report(&stand_in, &read);
        println!("{said}");
        for line in [
            "Do you speak French fluently ?: Yes",
            "Privacy: Acknowledge",
            "required and empty: none",
        ] {
            assert!(said.contains(line), "{line} in {said}");
        }
        let confirmation = sent.expect("the confirmation page's text");
        assert_eq!(asked.borrow().last(), Some(&app.message));
        println!("confirmation: {confirmation}");
        assert!(
            confirmation.starts_with("Thank you for applying."),
            "{confirmation}"
        );
        record(
            older.to_str().unwrap(),
            "acme-intro",
            &stand_in,
            &confirmation,
        )
        .expect("recorded");
        let calls = std::fs::read_to_string(&calls).unwrap();
        println!("{calls}");
        let mut lines = calls.lines();
        assert_eq!(lines.next(), Some("draft status acme-intro sent"));
        let outbound = lines
            .next()
            .unwrap()
            .strip_prefix("model outbound add ")
            .unwrap();
        let line: Value = serde_json::from_str(outbound).unwrap();
        assert_eq!(line["draft"], "acme-intro");
        assert_eq!(line["text"], json!(confirmation));
    }
}
