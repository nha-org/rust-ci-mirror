//! `dash mail send <draft-id> [--no-attachment]` (task outbound-gate): an approved mail draft sent from nicolas@klar.im over
//! JMAP, through the outbound gate on the exact text, logged in the older app's outbound log, then marked sent. Was
//! klar-mail's send (Python, SMTP and IMAP); Dashboard.Webmail's send is retired with it. It never mails twice: one send at
//! a time, a draft the log already shows is refused and marked, one he sent himself from his client too, and a send the
//! log refused is kept beside it until the next send logs it.
//!
//! The front door answers it (main.rs's `cli`), as it answers `dash outbox`: the server reads his Keychain and reaches the
//! mail server, which a session's sandbox does not.
use dash_drafts::send as draft;
use dash_hooks::outbound::{self as gate, Flags};
use dash_mail as mail;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

inventory::submit! { crate::usage::Command::usage("dash mail send <draft-id> [--no-attachment] (an approved mail draft, through the outbound gate, logged and marked sent)") }

/// The tool its log lines name.
const TOOL: &str = "dash mail send";
/// The mirror's record of each draft's copy in his Drafts, in the mailbox folder.
const COPIES: &str = ".drafts.json";
/// A wedged holder (a locked Keychain) must not hang every later send in silence: past its own bound it is stuck, not slow.
const LOCK_WAIT: Duration = Duration::from_secs(300);

/// JMAP's transport, signed in as his own address.
pub type Transport = Box<dyn Fn(&str, Option<&str>) -> Result<String, String>>;

/// What a send reads and does outside this process: the older app's model, the gate, the mail server, the logs.
pub trait Outside {
    fn draft(&mut self, id: &str) -> Result<Value, String>;
    fn contacts(&mut self) -> Result<Value, String>;
    /// The outbound log's lines, those not logged yet included.
    fn outbound(&mut self) -> Result<Vec<Value>, String>;
    /// A file of the mailbox folder, none when it is not there.
    fn mailbox(&mut self, name: &str) -> Result<Option<String>, String>;
    /// The outbound gate on the exact text: why it may not leave.
    fn refusal(&mut self, text: &str, flags: Flags) -> Option<String>;
    /// The gate on a first mail's own subject: the banned rows and the tells, why it may not leave.
    fn subject_refusal(&mut self, subject: &str, flags: Flags) -> Option<String>;
    /// The send of draft `id` now tried, its log line kept from before the mail is handed over until the log has it; none
    /// clears it. A send the server never answered, or cut short before its log, is found by it at the next one.
    fn trying(&mut self, id: &str, line: Option<&Value>) -> Result<(), String>;
    fn tried(&mut self, id: &str) -> Result<Option<Value>, String>;
    /// The app logs the send: it must answer before the mail goes, not after.
    fn ping(&mut self) -> Result<(), String>;
    fn http(&mut self) -> Result<Transport, String>;
    /// A file of the checkout uploaded for the mail: its blob.
    fn upload(&mut self, url: &str, file: &str, mime: &str) -> Result<String, String>;
    fn html(&mut self, text: &str) -> String;
    fn now(&mut self) -> String;
    /// A send refused for a missing file, counted.
    fn attach_refused(&mut self, id: &str, why: &str);
    fn log(&mut self, line: &Value) -> Result<(), String>;
    fn mark_sent(&mut self, id: &str) -> Result<(), String>;
}

fn marked(o: &mut dyn Outside, id: &str) -> String {
    match o.mark_sent(id) {
        Ok(()) => "marked sent".into(),
        Err(e) => {
            format!("NOT marked sent ({e}): run private/dashboard/bin/dash model sent {id}")
        }
    }
}

fn mime(file: &str) -> &'static str {
    match file
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "txt" | "md" => "text/plain",
        "html" => "text/html",
        "json" => "application/json",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

/// The copy of draft `id` the mirror put in his Drafts, and the Message-IDs of every other draft's copy.
fn copy_of(copies: &Value, id: &str) -> Option<(mail::Copy, Vec<String>)> {
    let rec = &copies[id];
    rec.get("uid")?;
    let said = |k: &str| rec[k].as_str().map(String::from);
    let made = (rec["made"].as_f64()? * 1000.0) as i64;
    let copy = mail::Copy {
        message_id: said("message_id")?,
        to: said("to")?,
        subject: said("subject")?,
        made: crate::comments::iso(made),
    };
    let others = (copies.as_object().into_iter().flatten())
        .filter(|(other, _)| other.as_str() != id)
        .filter_map(|(_, r)| r["message_id"].as_str().map(String::from))
        .collect();
    Some((copy, others))
}

/// Draft `id` sent: what to say, or why nothing went.
pub fn send(o: &mut dyn Outside, id: &str, no_attachment: bool) -> Result<String, String> {
    let d = o.draft(id)?;
    if let Some(why) = draft::refused(&d) {
        return Err(why);
    }
    let to = draft::addressed(&d, &draft::cards(&o.contacts()?))?;
    let files: Vec<&str> = match &d["attach"] {
        Value::Array(list) => list.iter().filter_map(Value::as_str).collect(),
        one => one.as_str().into_iter().collect(),
    };
    // the text part: **bold** as its words, as the draft's score read it, so Jev's verdict on the draft is this send's
    let body = gate::plain(d["body"].as_str().unwrap_or_default()) + "\n";
    if let Some(why) = draft::unfilled(&d, &body) {
        return Err(why);
    }
    // klar-mail crashed on a draft with no body; the gate passes no words
    if body.trim().is_empty() {
        return Err(format!("{id} has no text"));
    }
    // the same words to the same address, whatever sent them: klar-mail, this command, or /mail, whose line names no draft
    // and trims its text (/code-review of PR 1490)
    let went = |e: &Value| {
        let same = e["to"] == to.to.as_str()
            && (e["text"].as_str()).is_some_and(|t| t.trim() == body.trim());
        same || e["draft"] == id
    };
    if o.outbound()?.iter().any(went) {
        let _ = o.trying(id, None);
        let mark = marked(o, id);
        return Err(format!(
            "{id} already went to {} (the outbound log); {mark}",
            to.to
        ));
    }
    // after the log's check: a mail that went is marked sent, whatever its files or its text
    let missing = draft::attach_missing(&d)?;
    if let Some(why) = missing.as_ref().filter(|_| !no_attachment) {
        o.attach_refused(id, why);
        return Err(format!(
            "{id} {why}: dash draft status {id} pending, add the file to :attach, and he approves again; or pass --no-attachment"
        ));
    }
    if let Some(why) = o.refusal(&body, Flags::of(&d)) {
        return Err(format!("{id} refused by the outbound gate: {why}"));
    }
    o.ping()?;
    let http = o.http()?;
    // a try before this one handed the mail over and never logged it (no answer in time, a restart): Sent says whether it
    // went. ponytail: any mail to that address with that subject since that day counts, so a second one the same day
    // needs its try cleared by hand; the exact mail if the server ever echoes our Message-ID.
    if let Some(line) = o.tried(id)? {
        let said = |k: &str| line[k].as_str().unwrap_or_default().to_string();
        let since = format!("{}T00:00:00Z", said("date"));
        if mail::sent_since(&*http, &said("to"), &said("subject"), &since)? {
            let logged = match o.log(&line).and_then(|()| o.trying(id, None)) {
                Ok(()) => String::new(),
                Err(e) => format!("; not logged yet ({e})"),
            };
            let mark = marked(o, id);
            return Err(format!(
                "{id} went at its last try, which got no answer (Sent shows it); {mark}{logged}"
            ));
        }
    }
    let copies = o.mailbox(COPIES)?.unwrap_or_else(|| "{}".into());
    let copies: Value = serde_json::from_str(&copies).map_err(|e| format!("{COPIES}: {e}"))?;
    if let Some((copy, others)) = copy_of(&copies, id)
        && mail::he_sent(&*http, &copy, &others)?
    {
        let mark = marked(o, id);
        return Err(format!(
            "{id} left his Drafts and Sent shows it: he sent it himself; {mark}"
        ));
    }
    // the mail its :reply-to names, its file in the mailbox folder; a file that does not read, or holds no Message-ID,
    // threads by its :where instead
    let name = d["reply-to"].as_str().and_then(|r| r.rsplit('/').next());
    let answered = (name.and_then(|n| o.mailbox(n).ok().flatten()))
        .map(|eml| mail::answered(&eml))
        .filter(|a| a.message_id.is_some());
    let mut head = json!({
        "to": [{"email": to.to}],
        "subject": draft::subject(&d, answered.as_ref().map(|a| a.subject.as_str())),
    });
    if !to.cc.is_empty() {
        head["cc"] = to.cc.iter().map(|a| json!({"email": a})).collect();
    }
    let thread = draft::thread_named(d["where"].as_str().unwrap_or_default());
    // a first mail's subject is his draft's own words, and leaves too; a reply's is theirs
    if answered.is_none()
        && thread.is_none()
        && let Some(why) =
            o.subject_refusal(head["subject"].as_str().unwrap_or_default(), Flags::of(&d))
    {
        return Err(format!(
            "{id}'s subject is refused by the outbound gate: {why}"
        ));
    }
    match (&answered, thread) {
        (Some(a), _) => {
            head["inReplyTo"] = json!([a.message_id]);
            head["references"] = json!(a.references);
        }
        (None, Some(title)) => {
            if let Some(original) = mail::original_id(&*http, draft::base(title))? {
                head["inReplyTo"] = json!([original]);
                head["references"] = json!([original]);
            }
        }
        (None, None) => {}
    }
    if !files.is_empty() {
        let url = mail::upload_url(&*http)?;
        let mut attached = vec![];
        for file in &files {
            let kind = mime(file);
            let blob = o.upload(&url, file, kind)?;
            let name = file.rsplit('/').next().unwrap_or(file);
            attached.push(
                json!({"blobId": blob, "type": kind, "name": name, "disposition": "attachment"}),
            );
        }
        head["attachments"] = json!(attached);
    }
    let html = o.html(d["body"].as_str().unwrap_or_default().trim());
    let at = o.now();
    let mut line = json!({
        "date": at.get(..10).unwrap_or(&at), "at": at, "channel": "email", "tool": TOOL, "from": draft::FROM,
        "to": to.to, "subject": head["subject"], "text": body, "draft": id,
    });
    if let Some(why) = missing {
        line["no-attachment"] = json!(why);
    }
    // kept before the mail is handed over: a try that cannot be kept sends nothing, and one the server never answers is
    // not « nothing sent » at the next send (/code-review of PR 1490)
    o.trying(id, Some(&line))?;
    let email = mail::submit(&*http, draft::FROM, head.clone(), &body, Some(&html))?;
    line["email"] = json!(email);
    // logged first: it went out, and nothing after may read as a failure to send (a retry would send it twice)
    let logged = match o.log(&line) {
        Ok(()) => {
            let _ = o.trying(id, None);
            String::new()
        }
        Err(e) => format!("; the outbound log refused it ({e}): kept, logged at the next send"),
    };
    let mark = marked(o, id);
    let cc = match to.cc.is_empty() {
        true => String::new(),
        false => format!(" (cc {})", to.cc.join(", ")),
    };
    Ok(format!(
        "sent {id} to {}{cc}, {} file(s); {mark}{logged}",
        to.to,
        files.len()
    ))
}

/// The real outside: the older app, the gate, klar's mail server as his own address.
struct Live {
    repo: PathBuf,
    data: PathBuf,
}

impl Live {
    fn older(&self, args: &[&str]) -> Result<String, String> {
        dash_process::run(
            &crate::tasks::older_app().bin,
            args,
            None,
            Duration::from_secs(60),
        )
    }

    fn json(&self, args: &[&str]) -> Result<Value, String> {
        serde_json::from_str(&self.older(args)?)
            .map_err(|e| format!("dash {}: {e}", args.join(" ")))
    }

    /// A send the app could not log (it went down between the ping and the log): kept here, so `outbound` still sees it and
    /// a rerun never mails twice.
    fn unlogged(&self) -> PathBuf {
        self.data.join("mail-send-unlogged.jsonl")
    }

    fn pending(&self) -> Result<Vec<Value>, String> {
        kept(&self.unlogged())
    }

    fn try_file(&self, id: &str) -> PathBuf {
        let name: String = (id.chars())
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.data.join(format!("mail-send-trying-{name}.json"))
    }
}

/// The lines of a file of sends kept for the log. No file: none kept. One that does not read, or a line of it, stops the
/// send: it may name this draft (/code-review of PR 1490: the file read loosely passed a draft it named).
fn kept(file: &Path) -> Result<Vec<Value>, String> {
    let said = |e: &dyn std::fmt::Display| format!("{}: {e}", file.display());
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(said(&e)),
    };
    (text.lines().filter(|l| !l.trim().is_empty()))
        .map(|l| serde_json::from_str(l).map_err(|e| said(&e)))
        .collect()
}

/// `lines` logged in order through `add`; what is not logged yet (from the first refusal on) is written to `kept`, whole or
/// removed, so a line logged before the failure is never logged twice.
fn log_all(
    lines: &[Value],
    kept: &Path,
    mut add: impl FnMut(&Value) -> Result<(), String>,
) -> Result<(), String> {
    let failed = lines.iter().position(|l| add(l).is_err());
    let Some(at) = failed else {
        let _ = std::fs::remove_file(kept);
        return Ok(());
    };
    let rest: String = lines[at..].iter().map(|l| format!("{l}\n")).collect();
    let tmp = kept.with_extension("tmp");
    std::fs::write(&tmp, rest)
        .and_then(|()| std::fs::rename(&tmp, kept))
        .map_err(|e| format!("and {} did not write: {e}", kept.display()))?;
    Err(format!(
        "{} line(s) kept in {}",
        lines.len() - at,
        kept.display()
    ))
}

impl Outside for Live {
    fn draft(&mut self, id: &str) -> Result<Value, String> {
        self.json(&["model", "draft", id])
    }

    fn contacts(&mut self) -> Result<Value, String> {
        self.json(&["model", "contacts"])
    }

    fn outbound(&mut self) -> Result<Vec<Value>, String> {
        let mut lines: Vec<Value> = serde_json::from_value(self.json(&["model", "outbound"])?)
            .map_err(|e| format!("dash model outbound: {e}"))?;
        lines.extend(self.pending()?);
        // klar-mail's sends the app never logged, kept by it before this command took the send over: still a send that
        // went (/code-review)
        lines.extend(kept(
            &self.repo.join(".claude/run/klar-mail-unlogged.jsonl"),
        )?);
        Ok(lines)
    }

    fn mailbox(&mut self, name: &str) -> Result<Option<String>, String> {
        let bin = crate::tasks::older_app().bin;
        let args = ["model", "mailbox", "get", name];
        let (status, text, reason) =
            dash_process::answer(&bin, &args, None, &[], Duration::from_secs(60))?;
        match (status.success(), reason.trim()) {
            (true, _) => Ok(Some(text)),
            // exit 1 and no message: the file is not there
            (false, "") if status.code() == Some(1) => Ok(None),
            (false, why) => Err(format!("the mailbox folder's {name} did not read: {why}")),
        }
    }

    fn refusal(&mut self, text: &str, flags: Flags) -> Option<String> {
        crate::claude_hooks::refusal(text, flags)
    }

    fn subject_refusal(&mut self, subject: &str, flags: Flags) -> Option<String> {
        crate::claude_hooks::refusal_typed(subject, Some(flags))
    }

    fn trying(&mut self, id: &str, line: Option<&Value>) -> Result<(), String> {
        let file = self.try_file(id);
        let done = match line {
            Some(line) => std::fs::write(&file, line.to_string()),
            None => match std::fs::remove_file(&file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
        };
        done.map_err(|e| format!("{}: {e}", file.display()))
    }

    fn tried(&mut self, id: &str) -> Result<Option<Value>, String> {
        Ok(kept(&self.try_file(id))?.into_iter().next())
    }

    fn ping(&mut self) -> Result<(), String> {
        self.older(&["ping"]).map(|_| ())
    }

    fn http(&mut self) -> Result<Transport, String> {
        Ok(Box::new(crate::mail::signed_in(draft::FROM)?))
    }

    fn upload(&mut self, url: &str, file: &str, mime: &str) -> Result<String, String> {
        crate::mail::upload(draft::FROM, url, &self.repo.join(file), mime)
    }

    fn html(&mut self, text: &str) -> String {
        dash_surface::documents::mail_html(text)
    }

    fn now(&mut self) -> String {
        crate::comments::iso(crate::tasks::now_ms())
    }

    fn attach_refused(&mut self, id: &str, why: &str) {
        use std::io::Write;
        let row = json!({"at": self.now(), "draft": id, "why": why});
        let file = self.data.join("mail-send-attach-refused.jsonl");
        let wrote = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file);
        if let Err(e) = wrote.and_then(|mut f| writeln!(f, "{row}")) {
            eprintln!(
                "mail send: the refusal was not counted in {}: {e}",
                file.display()
            );
        }
    }

    fn log(&mut self, line: &Value) -> Result<(), String> {
        let mut lines = self.pending()?;
        lines.push(line.clone());
        // the plain add, never the bridge's `outbound`: that one keeps a refused line for the app's next check, and this
        // command keeps its own (`unlogged`, read back before a send), so the line would be logged twice
        let kept = self.unlogged();
        log_all(&lines, &kept, |l| {
            self.older(&["model", "outbound", "add", &l.to_string()])
                .map(|_| ())
        })
    }

    fn mark_sent(&mut self, id: &str) -> Result<(), String> {
        self.older(&["model", "sent", id]).map(|_| ())
    }
}

/// `dash mail send <words>` as (exit, stdout, stderr). One send at a time, every caller's: two runs of one draft at once
/// both passed the log's check and both mailed (audit-send-path); the second now reads the first's outcome.
pub fn command(args: &[&str]) -> (i32, String, String) {
    let ids: Vec<&str> = args
        .iter()
        .copied()
        .filter(|a| *a != "--no-attachment")
        .collect();
    let refuse = |why: String| {
        (
            1,
            String::new(),
            format!("mail send: {why}: nothing sent\n"),
        )
    };
    // the subject comes from the draft alone: a mistyped flag never goes out as one
    let [id] = ids[..] else {
        return refuse(format!(
            "send takes one draft id and --no-attachment, got {}",
            if ids.is_empty() {
                "no id".into()
            } else {
                ids.join(" ")
            }
        ));
    };
    let data = crate::tasks::live_data();
    let lock = data.join("mail-send.lock");
    let held = std::fs::create_dir_all(&data).and_then(|()| std::fs::File::create(&lock));
    let held = match held {
        Ok(file) => file,
        Err(e) => return refuse(format!("{}: {e}", lock.display())),
    };
    let began = Instant::now();
    while held.try_lock().is_err() {
        if began.elapsed() >= LOCK_WAIT {
            return refuse(format!(
                "another send has held {} for {} s",
                lock.display(),
                LOCK_WAIT.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    let mut live = Live {
        repo: crate::tasks::repo(),
        data,
    };
    match send(&mut live, id, args.contains(&"--no-attachment")) {
        Ok(said) => (0, said + "\n", String::new()),
        Err(why) => refuse(why),
    }
}

/// The binary's own arm, for a caller outside any sandbox: `command`'s answer printed, its exit the process's.
pub fn run(args: &[&str], mut out: std::fs::File) -> ! {
    use std::io::Write;
    let (exit, said, err) = command(args);
    let _ = write!(out, "{said}");
    eprint!("{err}");
    std::process::exit(exit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// The outside, in memory: one approved draft, his contact cards, a JMAP server that keeps each mail made, and a gate
    /// that refuses the word « banned ».
    #[derive(Default)]
    struct Fake {
        draft: Value,
        outbound: Vec<Value>,
        mailbox: Vec<(&'static str, String)>,
        he_sent: bool,
        log_down: bool,
        judged: Vec<(String, Flags)>,
        subjects: Vec<String>,
        tried: Option<Value>,
        no_answer: bool,
        made: Rc<RefCell<Vec<Value>>>,
        uploaded: Vec<(String, String)>,
        logged: Vec<Value>,
        marked: Vec<String>,
        refused_files: Vec<String>,
    }

    fn fake() -> Fake {
        Fake {
            draft: json!({"id": "x", "status": "approved", "sendable": true, "channel": "email", "held-until": null,
                "attach-missing": null, "attach": null, "to": "Ada (Example)", "where": "mail", "cold": false,
                "first": true, "body": "Bonjour Ada,\n\nVoici **le plan**.\n", "subject": "Le plan"}),
            ..Default::default()
        }
    }

    impl Outside for Fake {
        fn draft(&mut self, id: &str) -> Result<Value, String> {
            assert_eq!(id, "x");
            Ok(self.draft.clone())
        }
        fn contacts(&mut self) -> Result<Value, String> {
            Ok(
                json!([{"name": "Ada", "email": "ada@example.org"}, {"name": "Bob", "email": "bob@example.org"}]),
            )
        }
        fn outbound(&mut self) -> Result<Vec<Value>, String> {
            Ok(self.outbound.clone())
        }
        fn mailbox(&mut self, name: &str) -> Result<Option<String>, String> {
            Ok(self
                .mailbox
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, t)| t.clone()))
        }
        fn refusal(&mut self, text: &str, flags: Flags) -> Option<String> {
            self.judged.push((text.to_string(), flags));
            text.contains("banned")
                .then(|| "banned phrase « banned »: drop it".to_string())
        }
        fn subject_refusal(&mut self, subject: &str, _: Flags) -> Option<String> {
            self.subjects.push(subject.into());
            subject
                .contains('—')
                .then(|| "an AI tell, an em-dash".to_string())
        }
        fn trying(&mut self, id: &str, line: Option<&Value>) -> Result<(), String> {
            assert_eq!(id, "x");
            self.tried = line.cloned();
            Ok(())
        }
        fn tried(&mut self, _: &str) -> Result<Option<Value>, String> {
            Ok(self.tried.clone())
        }
        fn ping(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn http(&mut self) -> Result<Transport, String> {
            let (made, he_sent, no_answer) = (self.made.clone(), self.he_sent, self.no_answer);
            Ok(Box::new(move |_url, body| {
                if no_answer && body.is_some_and(|b| b.contains("EmailSubmission/set")) {
                    return Err("jmap: no answer".into());
                }
                let Some(body) = body else {
                    return Ok(json!({"apiUrl": "https://mail.klar.im/jmap/", "uploadUrl": "https://up/{accountId}/",
                        "primaryAccounts": {"urn:ietf:params:jmap:mail": "f"}}).to_string());
                };
                let asked: Value = serde_json::from_str(body).expect("JSON asked");
                let mut answers = vec![];
                let mut sent_asked = false;
                for call in asked["methodCalls"].as_array().expect("calls") {
                    let (name, args, id) = (&call[0], &call[1], &call[2]);
                    let result = match name.as_str().expect("a name") {
                        "Mailbox/query" => json!({"ids": [args["filter"]["role"]]}),
                        "Email/query" => {
                            sent_asked = args["filter"]["inMailbox"] == "sent";
                            json!({"ids": []})
                        }
                        "Email/get" if sent_asked && he_sent => {
                            json!({"list": [{"messageId": ["his@x"], "subject": "Le plan"}]})
                        }
                        "Email/get" if args["#ids"].is_object() && !he_sent && sent_asked => {
                            json!({"list": []})
                        }
                        "Email/get" => json!({"list": []}),
                        "Identity/get" => {
                            json!({"list": [{"id": "i", "email": draft::FROM, "name": "Nicolas Ha"}]})
                        }
                        "Email/set" => {
                            made.borrow_mut().push(args["create"]["draft"].clone());
                            json!({"created": {"draft": {"id": "e1"}}})
                        }
                        "EmailSubmission/set" => json!({"created": {"send": {"id": "s1"}}}),
                        other => panic!("no {other} here"),
                    };
                    answers.push(json!([name, result, id]));
                }
                Ok(json!({"methodResponses": answers}).to_string())
            }))
        }
        fn upload(&mut self, url: &str, file: &str, mime: &str) -> Result<String, String> {
            assert_eq!(url, "https://up/f/");
            self.uploaded.push((file.into(), mime.into()));
            Ok(format!("blob-{}", self.uploaded.len()))
        }
        fn html(&mut self, text: &str) -> String {
            format!("html of {text}")
        }
        fn now(&mut self) -> String {
            "2026-11-12T10:00:00Z".into()
        }
        fn attach_refused(&mut self, id: &str, _: &str) {
            self.refused_files.push(id.into());
        }
        fn log(&mut self, line: &Value) -> Result<(), String> {
            match self.log_down {
                true => Err("the app is down".into()),
                false => {
                    self.logged.push(line.clone());
                    Ok(())
                }
            }
        }
        fn mark_sent(&mut self, id: &str) -> Result<(), String> {
            self.marked.push(id.into());
            Ok(())
        }
    }

    /// Done-when 2: `dash mail send <id>` calls the gate on the exact text that leaves, with the draft's own flags, sends it,
    /// and logs the send as the outbound log's lines are (date, at, channel, tool, to, text), then marks the draft sent.
    #[dash_test]
    fn a_draft_passes_the_gate_then_is_sent_logged_and_marked() {
        let mut o = fake();
        assert_eq!(
            send(&mut o, "x", false),
            Ok("sent x to ada@example.org, 0 file(s); marked sent".into())
        );
        let text = "Bonjour Ada,\n\nVoici le plan.\n";
        let flags = Flags {
            cold: false,
            first: true,
            long_ok: false,
        };
        assert_eq!(
            o.judged,
            [(text.to_string(), flags)],
            "the gate, on the text that leaves"
        );
        let made = o.made.borrow()[0].clone();
        assert_eq!(
            made["bodyValues"]["body"]["value"], text,
            "the mail holds the text the gate judged"
        );
        assert_eq!(
            made["bodyValues"]["html"]["value"],
            "html of Bonjour Ada,\n\nVoici **le plan**."
        );
        assert_eq!(
            (&made["to"][0]["email"], &made["subject"]),
            (&json!("ada@example.org"), &json!("Le plan"))
        );
        assert_eq!(
            o.logged,
            [
                json!({"date": "2026-11-12", "at": "2026-11-12T10:00:00Z", "channel": "email", "tool": "dash mail send",
                    "from": "nicolas@klar.im", "to": "ada@example.org", "subject": "Le plan", "text": text,
                    "draft": "x", "email": "e1"})
            ]
        );
        assert_eq!(o.marked, ["x"]);
    }

    /// Done-when 1, the mail path: a text the gate refuses is not sent, logged or marked.
    #[dash_test]
    fn a_draft_the_gate_refuses_is_not_sent() {
        let mut o = fake();
        o.draft["body"] = json!("A banned line.");
        assert_eq!(
            send(&mut o, "x", false),
            Err("x refused by the outbound gate: banned phrase « banned »: drop it".into())
        );
        assert!(o.made.borrow().is_empty() && o.logged.is_empty() && o.marked.is_empty());
    }

    /// It never mails twice: a draft the log shows (by its id, or by the same text to the same address from this tool or
    /// klar-mail) is marked sent and not sent, before the gate is even asked.
    #[dash_test]
    fn a_draft_the_log_already_shows_is_marked_not_sent_again() {
        for line in [
            json!({"draft": "x", "tool": "mail-page"}),
            // /mail's own line: no draft, its text trimmed
            json!({"tool": "mail-page", "to": "ada@example.org", "text": "Bonjour Ada,\n\nVoici le plan."}),
            json!({"tool": "klar-mail", "to": "ada@example.org", "text": "Bonjour Ada,\n\nVoici le plan.\n"}),
            json!({"tool": "dash mail send", "to": "ada@example.org", "text": "Bonjour Ada,\n\nVoici le plan.\n"}),
        ] {
            let mut o = fake();
            o.outbound = vec![line];
            assert_eq!(
                send(&mut o, "x", false),
                Err("x already went to ada@example.org (the outbound log); marked sent".into())
            );
            assert!(o.made.borrow().is_empty() && o.judged.is_empty());
            assert_eq!(o.marked, ["x"]);
        }
        let mut o = fake();
        o.outbound = vec![
            json!({"tool": "klar-mail", "to": "bob@example.org", "text": "Bonjour Ada,\n\nVoici le plan.\n"}),
        ];
        assert!(
            send(&mut o, "x", false).is_ok(),
            "another receiver is another mail"
        );
    }

    /// /code-review of PR 1490. A send the server never answered is not « nothing sent »: its try is kept, and the next send
    /// asks Sent, logs and marks it when it went, sends it when it did not. A first mail's subject passes the gate too, a
    /// reply's is theirs; a draft with no text is not sent.
    #[dash_test]
    fn a_send_with_no_answer_is_never_sent_twice_and_a_subject_or_no_text_is_refused() {
        let mut o = fake();
        o.no_answer = true;
        assert_eq!(send(&mut o, "x", false), Err("jmap: no answer".into()));
        let tried = o.tried.clone().expect("the try is kept");
        assert_eq!(
            (&tried["draft"], &tried["subject"]),
            (&json!("x"), &json!("Le plan"))
        );
        assert!(o.logged.is_empty() && o.marked.is_empty());
        // Sent shows it: it went
        (o.no_answer, o.he_sent) = (false, true);
        assert_eq!(
            send(&mut o, "x", false),
            Err("x went at its last try, which got no answer (Sent shows it); marked sent".into())
        );
        assert_eq!(
            (o.logged.clone(), o.tried.clone()),
            (vec![tried.clone()], None)
        );
        assert_eq!((o.marked.len(), o.made.borrow().len()), (1, 0));
        // Sent does not: this try sends it, and clears itself once logged
        let mut o = fake();
        o.tried = Some(tried);
        assert!(send(&mut o, "x", false).is_ok());
        assert_eq!(
            (o.made.borrow().len(), o.logged.len(), &o.tried),
            (1, 1, &None)
        );
        assert_eq!(o.subjects, ["Le plan"]);
        let mut o = fake();
        o.draft["subject"] = json!("Le plan — 650 €");
        assert_eq!(
            send(&mut o, "x", false),
            Err("x's subject is refused by the outbound gate: an AI tell, an em-dash".into())
        );
        assert!(o.made.borrow().is_empty() && o.tried.is_none());
        let mut o = fake();
        o.draft["where"] = json!("sujet « Mission — Kafka »");
        assert!(
            send(&mut o, "x", false).is_ok(),
            "their subject is not his to change"
        );
        let mut o = fake();
        o.draft["body"] = json!(" ** \n");
        assert_eq!(send(&mut o, "x", false), Err("x has no text".into()));
        // a kept file that does not read stops the send
        let dir = std::env::temp_dir().join(format!("mail-send-kept-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        assert_eq!(kept(&dir.join("none.jsonl")), Ok(vec![]));
        std::fs::write(dir.join("k.jsonl"), "{\"draft\": \"x\"}\n\nnot json\n").expect("write");
        assert!(kept(&dir.join("k.jsonl")).is_err());
    }

    /// A draft he sent himself from his client (its copy left his Drafts, Sent shows it) is marked, not sent again.
    #[dash_test]
    fn a_draft_he_sent_himself_is_marked_not_sent_again() {
        let mut o = fake();
        o.he_sent = true;
        let copies = json!({"x": {"uid": 7, "message_id": "<ours@klar.im>", "to": "ada@example.org",
                                   "subject": "Le plan", "made": 1794477600.5}});
        o.mailbox = vec![(COPIES, copies.to_string())];
        assert_eq!(
            send(&mut o, "x", false),
            Err("x left his Drafts and Sent shows it: he sent it himself; marked sent".into())
        );
        assert!(o.made.borrow().is_empty() && o.logged.is_empty());
    }

    /// A draft not ready, or with a file missing, is refused before the gate; --no-attachment sends it, the reason logged.
    #[dash_test]
    fn a_missing_file_refuses_the_send_unless_overridden() {
        let mut o = fake();
        o.draft["attach-missing"] = json!("names a CV and carries none");
        let said = send(&mut o, "x", false).expect_err("refused");
        assert!(
            said.starts_with("x names a CV and carries none: dash draft status x pending"),
            "{said}"
        );
        assert_eq!((o.refused_files.len(), o.made.borrow().len()), (1, 0));
        assert!(send(&mut o, "x", true).is_ok());
        assert_eq!(o.logged[0]["no-attachment"], "names a CV and carries none");
        let mut o = fake();
        o.draft["sendable"] = json!(false);
        assert_eq!(
            send(&mut o, "x", false),
            Err("x is approved, not in the outbox (private/dashboard/bin/dash outbox)".into())
        );
    }

    /// A reply threads under the mail its :reply-to names, with its copies and its files; a send the log refuses still
    /// says it went, and is kept.
    #[dash_test]
    fn a_reply_carries_its_thread_copies_and_files() {
        let mut o = fake();
        o.draft["to"] = json!("Ada, cc Bob");
        o.draft["reply-to"] = json!("private/store/mail/klar/2026-11-10-ada.eml");
        o.draft["attach"] = json!(["private/cv/sent/a.pdf"]);
        let eml = "Subject: Mission Kafka\nMessage-ID: <m2@example.org>\nReferences: <m1@example.org>\n\nBonjour";
        o.mailbox = vec![("2026-11-10-ada.eml", eml.into())];
        o.log_down = true;
        let said = send(&mut o, "x", false).expect("sent");
        assert_eq!(
            said,
            "sent x to ada@example.org (cc bob@example.org), 1 file(s); marked sent; the outbound log refused it (the app is down): kept, logged at the next send"
        );
        let made = o.made.borrow()[0].clone();
        assert_eq!(made["subject"], "Re: Mission Kafka");
        assert_eq!(made["inReplyTo"], json!(["m2@example.org"]));
        assert_eq!(
            made["references"],
            json!(["m1@example.org", "m2@example.org"])
        );
        assert_eq!(made["cc"], json!([{"email": "bob@example.org"}]));
        assert_eq!(
            made["attachments"],
            json!([{"blobId": "blob-1", "type": "application/pdf", "name": "a.pdf", "disposition": "attachment"}])
        );
        assert_eq!(
            o.uploaded,
            [(
                "private/cv/sent/a.pdf".to_string(),
                "application/pdf".to_string()
            )]
        );
    }

    /// A send the log refused is kept, and logged before the next one; a line logged before a failure is not kept twice.
    #[dash_test]
    fn an_unlogged_send_is_kept_until_it_logs() {
        let kept =
            std::env::temp_dir().join(format!("mail-send-unlogged-{}.jsonl", std::process::id()));
        let lines = [
            json!({"draft": "a"}),
            json!({"draft": "b"}),
            json!({"draft": "c"}),
        ];
        let mut logged = vec![];
        let said = log_all(&lines, &kept, |l| {
            if l["draft"] == "b" {
                return Err("down".into());
            }
            logged.push(l.clone());
            Ok(())
        });
        assert!(said.expect_err("kept").starts_with("2 line(s) kept in "));
        assert_eq!(logged, [json!({"draft": "a"})]);
        assert_eq!(
            std::fs::read_to_string(&kept).expect("kept"),
            "{\"draft\":\"b\"}\n{\"draft\":\"c\"}\n"
        );
        assert_eq!(log_all(&lines[1..], &kept, |_| Ok(())), Ok(()));
        assert!(!kept.exists());
    }

    /// A mistyped flag never goes out as a draft id, and nothing is read for it.
    #[dash_test]
    fn send_takes_one_id() {
        let (exit, out, err) = command(&["x", "--subject", "y"]);
        assert_eq!((exit, out.as_str()), (1, ""));
        assert_eq!(
            err,
            "mail send: send takes one draft id and --no-attachment, got x --subject y: nothing sent\n"
        );
        assert_eq!(
            command(&[]).2,
            "mail send: send takes one draft id and --no-attachment, got no id: nothing sent\n"
        );
    }
}
