//! Task draft-from-is-address, slice 1 of rfc-rust-plugins-ui section 7, end to end through the plugins as the binary links
//! them: mail's channel « email », its thread and compose box, and drafts' card under the thread with its Send and Undo, on a
//! fake JMAP server and a fake clock. The outbound gate is a stand-in registered here, as the hooks plugin registers its own.
use dash_core::{Answer, Asked, Host, Later, OpLog, Transport, WallClock};
use dash_drafts::{Draft, Kept, Version, card};
use dash_meter::{TestDir, dash_test};
use dash_store::Connection;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// the stand-in gate: a text holding BANNED does not leave
inventory::submit! { dash_core::ContextProvider { key: "hooks/outbound-gate", lines: |_, text| {
    Ok(if text.contains("BANNED") { vec!["a banned phrase".into()] } else { vec![] })
} } }

/// The fake world: a data folder, a clock the test moves, a JMAP server counting its sends, and what waits to run later.
struct Fake {
    dir: PathBuf,
    now: AtomicI64,
    sends: Arc<AtomicUsize>,
    later: Mutex<Vec<(i64, Later)>>,
    /// The thread does not read for a check (a JMAP timeout); the page's own read still does.
    unread: bool,
    /// The card's sends as `dash mail send` reads them, at each mail handed to the server.
    during: Arc<Mutex<Vec<Vec<Value>>>>,
}

impl WallClock for Fake {
    fn now_ms(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

impl Host for Fake {
    fn repo(&self) -> PathBuf {
        self.dir.clone()
    }
    fn data(&self) -> PathBuf {
        self.dir.clone()
    }
    fn home(&self) -> Result<PathBuf, String> {
        Err("no home here".into())
    }
    fn arm(&self, _: &Path, _: &str) {}
    fn open(&self, file: &str, schema: &[&str]) -> Result<Connection, String> {
        dash_store::plugin::open(&self.dir.join(file), schema, self.now_ms())
    }
    fn transact(
        &self,
        db: &Connection,
        run: &mut dyn FnMut(&Connection) -> Result<(), String>,
    ) -> Result<(), String> {
        dash_store::plugin::transact(db, self.now_ms(), run)
    }
    fn log(&self, dir: &Path, file: &str, schema: &[&str]) -> Result<Box<dyn OpLog>, String> {
        Ok(Box::new(dash_store::replica(&dir.join(file), schema)?))
    }
    fn signed_in(&self, _: &str) -> Result<Transport, String> {
        let (sends, during, unread) = (self.sends.clone(), self.during.clone(), self.unread);
        let drafts = self.dir.join(dash_drafts::FILE);
        Ok(Box::new(move |_, body| {
            let b = body.unwrap_or_default();
            if unread && b.contains("Thread/get") {
                return Err("the server did not answer".into());
            }
            if b.contains("EmailSubmission/set") {
                let db = dash_store::plugin::open(&drafts, &dash_drafts::SCHEMAS, 0)?;
                during.lock().expect("during").push(card::sent(&db)?);
            }
            jmap(&sends, body)
        }))
    }
    fn after(&self, ms: i64, run: Later) -> Result<(), String> {
        self.later
            .lock()
            .expect("later")
            .push((self.now_ms() + ms, run));
        Ok(())
    }
}

impl Fake {
    fn new(name: &str, d: Draft) -> (TestDir, Fake) {
        let dir = TestDir::new(name);
        let fake = Fake {
            dir: dir.0.clone(),
            now: AtomicI64::new(1_763_200_000_000),
            sends: Arc::default(),
            later: Mutex::default(),
            unread: false,
            during: Arc::default(),
        };
        let db = fake.drafts();
        let mut kept = |db: &Connection| {
            dash_drafts::save(db, &Kept::default(), std::slice::from_ref(&d)).map(drop)
        };
        fake.transact(&db, &mut kept).expect("the draft kept");
        (dir, fake)
    }

    fn drafts(&self) -> Connection {
        dash_drafts::db(self).expect("drafts.sqlite")
    }

    /// The clock moved by `ms`, then what was due runs, as the host's timer would.
    fn pass(&self, ms: i64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
        let now = self.now_ms();
        let due: Vec<Later> = {
            let mut later = self.later.lock().expect("later");
            let (due, wait) = later.drain(..).partition(|(at, _)| *at <= now);
            *later = wait;
            due.into_iter().map(|(_, run)| run).collect()
        };
        for run in due {
            run(self);
        }
    }

    fn sent(&self) -> usize {
        self.sends.load(Ordering::SeqCst)
    }

    /// The rows of mail's send log.
    fn logged(&self) -> i64 {
        let log = dash_store::replica(&self.dir.join(dash_mail::FILE), &[dash_mail::SCHEMA])
            .expect("mail.sqlite");
        log.read(|db| db.query_row("SELECT count(*) FROM mail_sent", [], |r| r.get(0)))
            .expect("read")
            .expect("counted")
    }

    fn post(&self, path: &str, form: &str) -> (String, String) {
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        let asked = Asked {
            path,
            query,
            html: false,
            held: None,
            host: self,
        };
        let post = dash_core::post(path).expect("a post answers it");
        match (post.serve)(&asked, form) {
            Answer::Body(status, _, body) => (status.into(), body),
            Answer::Drawn(_) => panic!("a post answers a body"),
        }
    }

    /// /mail/t/brq's tree as the binary draws it, the card placed under its mails.
    fn page(&self) -> String {
        let sends = self.sends.clone();
        let read = dash_mail::thread(&move |_: &str, b: Option<&str>| jmap(&sends, b), "brq");
        dash_mail::thread_tree(
            Some(self),
            dash_mail::OWN,
            &[dash_mail::OWN.into()],
            "brq",
            &read,
            None,
        )
    }
}

/// The mail plugin's fixture answering every read, and a send made and submitted, each send counted.
fn jmap(sends: &AtomicUsize, body: Option<&str>) -> Result<String, String> {
    let f: Value =
        serde_json::from_str(include_str!("../../mail/tests/thread.json")).expect("the fixture");
    let Some(body) = body else {
        return Ok(f["session"].to_string());
    };
    let asked: Value = serde_json::from_str(body).expect("JSON");
    Ok(match asked["methodCalls"][0][0].as_str() {
        Some("Thread/get") => f["thread"].to_string(),
        Some("Identity/get") => json!({"methodResponses": [
            ["Identity/get", {"list": [{"id": "i", "email": "nicolas@klar.im", "name": "N"}]}, "i"],
            ["Mailbox/query", {"ids": ["d"]}, "d"], ["Mailbox/query", {"ids": ["s"]}, "s"]]})
        .to_string(),
        _ => {
            sends.fetch_add(1, Ordering::SeqCst);
            json!({"methodResponses": [["Email/set", {"created": {"draft": {"id": "e1"}}}, "e"],
                ["EmailSubmission/set", {"created": {"send": {"id": "x"}}}, "s"]]})
            .to_string()
        }
    })
}

/// A reply draft to Ada's thread brq, from `sender`.
fn draft(sender: &str) -> Draft {
    Draft {
        id: "d1".into(),
        goal: "Answer Ada's six questions on the grant".into(),
        sender: sender.into(),
        receiver: "ada@example.org".into(),
        channel: "email".into(),
        lead: None,
        status: "pending".into(),
        approved: None,
        from_blocks: None,
        sent_on: None,
        created: "2026-11-14".into(),
        versions: vec![Version {
            text: "Monday at ten, then.".into(),
            by: "claude".into(),
            ..Version::default()
        }],
        reply: None,
        thread: Some("brq".into()),
        facts: Value::Null,
    }
}

fn text(tree: &str) -> String {
    dash_surface::text(tree).expect("its text")
}

fn html(tree: &str) -> String {
    dash_surface::markup(&dash_surface::screen::page("Mail", tree).expect("its HTML"))
}

/// Done-when 1: the core's Channel and Element ports hold mail's « email » (its checks and its send), its thread and its
/// compose box, each found by key; drafts' card is placed under the thread.
#[dash_test]
fn mail_registers_email_its_thread_and_its_compose_box() {
    let (_dir, fake) = Fake::new("draft-card-ports", draft(dash_mail::OWN));
    let email = dash_core::channel("email").expect("mail registers « email »");
    let m = dash_core::Message {
        from: "nicolas@klar.im".into(),
        to: vec!["ada@example.org".into()],
        text: "Monday.".into(),
        answers: Some("brq".into()),
        ..Default::default()
    };
    let checks = (email.checks)(&fake, &m);
    let names: Vec<_> = checks.iter().map(|c| (c.name, c.fails.clone())).collect();
    assert_eq!(
        names,
        [
            ("sender", None),
            ("receivers", None),
            ("outbound gate", None)
        ],
        "{checks:?}"
    );
    let banned = dash_core::Message {
        text: "BANNED words".into(),
        ..m.clone()
    };
    let gate = (email.checks)(&fake, &banned)
        .into_iter()
        .find(|c| c.name == "outbound gate");
    assert_eq!(
        gate.and_then(|c| c.fails).as_deref(),
        Some("a banned phrase")
    );
    let mut tree = dash_core::Tree::default();
    let compose = dash_core::element("mail/compose").expect("mail registers its compose box");
    let boxed = (compose.draw)(
        &fake,
        &json!({"action": "/x", "send": "Send", "words": "Hi"}),
        "c",
        &mut tree,
    );
    assert_eq!(boxed, Ok(vec!["c".to_string()]));
    let elements = tree.elements();
    assert_eq!(elements["c"]["type"], "compose");
    assert_eq!(elements["c"]["props"]["words"], "Hi");
    let placed: Vec<_> = dash_core::placed("mail/thread")
        .iter()
        .map(|e| e.key)
        .collect();
    assert_eq!(placed, ["mail/thread/10-drafts"]);
    assert_eq!((email.send)(&fake, &m), Ok("e1".into()));
    assert_eq!((fake.sent(), fake.logged()), (1, 1));
}

/// Done-when 2: /mail/t/<thread> draws the reply draft's card under its mail, before his own reply box: its goal, its sender
/// and its receiver first, then its words in mail's compose box with Send, in the HTML and in the text.
#[dash_test]
fn the_thread_page_draws_a_reply_draft_s_card_under_its_mail() {
    let (_dir, fake) = Fake::new("draft-card-page", draft(dash_mail::OWN));
    let page = fake.page();
    let said = text(&page);
    let at = |words: &str| {
        said.find(words)
            .unwrap_or_else(|| panic!("{words:?} in {said}"))
    };
    let (mail, goal, who, words, send, reply) = (
        at("Monday works."),
        at("Goal: Answer Ada's six questions on the grant"),
        at("From nicolas@klar.im · To ada@example.org · every check passes"),
        at("Monday at ten, then."),
        at("Send to ada@example.org from nicolas@klar.im: POST /drafts/send/d1"),
        at("POST /mail/t/brq/reply"),
    );
    assert!(
        mail < goal && goal < who && who < words && words < send && send < reply,
        "{said}"
    );
    let html = html(&page);
    let form = format!(
        "{}form class=\"form\" id=\"placed-0-0-compose\" method=\"post\" action=\"/drafts/send/d1\">",
        '<'
    );
    assert!(html.contains(&form), "{html}");
    assert!(
        html.contains(r#"name="thread" type="hidden" value="brq""#),
        "{html}"
    );
    let (_kept, other) = Fake::new(
        "draft-card-other",
        Draft {
            thread: Some("zz".into()),
            ..draft(dash_mail::OWN)
        },
    );
    assert!(
        !text(&other.page()).contains("Goal:"),
        "a draft of another thread is not drawn"
    );
    let (_also, by_subject) = Fake::new(
        "draft-card-subject",
        Draft {
            thread: Some("The grant".into()),
            ..draft(dash_mail::OWN)
        },
    );
    assert!(
        text(&by_subject.page()).contains("Goal:"),
        "the older app's « sujet « The grant » » finds it"
    );
}

/// Done-when 3: a sender not among his identities fails the check: the card holds no Send, names the check, and the draft
/// goes back to its agent, recorded once where `dash draft show` reads it; a Send posted anyway sends nothing.
#[dash_test]
fn a_sender_not_his_fails_the_check_and_goes_back_to_its_agent() {
    let (_dir, fake) = Fake::new("draft-card-sender", draft("Nicolas Ha"));
    let page = fake.page();
    let said = text(&page);
    assert!(said.contains("Back to its agent: no Send"), "{said}");
    assert!(
        said.contains("sender: « Nicolas Ha » is not one of his addresses (nicolas@klar.im,"),
        "{said}"
    );
    assert!(!said.contains("/drafts/send/"), "no Send: {said}");
    assert!(
        !html(&page).contains("action=\"/drafts/send/"),
        "no Send in the HTML"
    );
    let back = card::back_to_agent(&fake.drafts(), "d1").expect("read");
    assert!(back.is_some_and(|why| why.starts_with("sender: « Nicolas Ha »")));
    fake.page();
    // the commits' changesets, the file's one stream (dash_store::plugin)
    let rows = dash_store::plugin::changes_after(&fake.drafts(), None, 1000).expect("changes");
    assert_eq!(
        rows.iter()
            .filter(|o| o.write.table == "draft_sends" && o.write.field == "record")
            .count(),
        1,
        "recorded once"
    );
    let (status, body) = fake.post("/drafts/send/d1", "thread=brq&text=Monday");
    assert_eq!(status, "409 Conflict");
    assert!(body.contains("back to its agent"), "{body}");
    fake.pass(card::UNDO_MS + 1);
    assert_eq!(fake.sent(), 0);
}

/// Done-when 4: every check green, the card's Send goes once its window passed, once though tapped twice, and is logged once:
/// one mail on the fake JMAP server, one row in mail's send log, the card then saying it went.
#[dash_test]
fn send_sends_once_and_is_logged_once() {
    let (_dir, fake) = Fake::new("draft-card-send", draft(dash_mail::OWN));
    let form = "thread=brq&text=Monday%20at%20ten%2C%20then.";
    assert_eq!(
        fake.post("/drafts/send/d1", form),
        ("303 See Other".into(), "/mail/t/brq".into())
    );
    assert_eq!(
        fake.post("/drafts/send/d1", form).0,
        "303 See Other",
        "a second tap queues nothing"
    );
    assert!(
        text(&fake.page()).contains("Sending in 10 s."),
        "the card waits with its Undo"
    );
    assert_eq!(fake.sent(), 0, "nothing before the window ends");
    fake.pass(card::UNDO_MS);
    assert_eq!((fake.sent(), fake.logged()), (1, 1));
    assert_eq!(
        card::go(&fake, "d1"),
        Ok(None),
        "a later run sends nothing more"
    );
    let (status, body) = fake.post("/drafts/send/d1", form);
    assert_eq!(
        (status.as_str(), body.as_str()),
        (
            "409 Conflict",
            "Not sent: it went already (e1)\n\nYour words:\nMonday at ten, then.\n"
        )
    );
    assert!(text(&fake.page()).contains("Sent (e1)."));
    assert_eq!((fake.sent(), fake.logged()), (1, 1));
    // /code-review 1: `dash mail send` reads the card's sends with the outbound log's, so the draft never goes twice
    let lines = card::sent(&fake.drafts()).expect("read");
    assert_eq!(
        lines,
        [json!({"draft": "d1", "email": "e1", "channel": "email", "tool": "draft-card"})]
    );
}

/// /code-review of the slice: a closed draft offers no Send; a check failing on a redraw keeps a waiting send; a refused Send
/// gives his words back; a failed send says why; an Undo past a window whose timer was late or lost still stops it.
#[dash_test]
fn the_card_keeps_every_send_he_meant_and_none_he_did_not() {
    let (_d1, closed) = Fake::new(
        "draft-card-closed",
        Draft {
            status: "rejected".into(),
            ..draft(dash_mail::OWN)
        },
    );
    let (status, body) = closed.post("/drafts/send/d1", "thread=brq&text=Mine");
    assert_eq!(status, "409 Conflict");
    assert!(
        body.contains("status: it is rejected, not open") && body.ends_with("Your words:\nMine\n"),
        "{body}"
    );
    let (_d2, fake) = Fake::new("draft-card-redraw", draft(dash_mail::OWN));
    fake.post("/drafts/send/d1", "thread=brq&text=Monday");
    // the thread no longer reads while it waits: the receivers check fails on this redraw
    let failing = Fake {
        sends: fake.sends.clone(),
        dir: fake.dir.clone(),
        now: AtomicI64::new(fake.now_ms()),
        later: Mutex::default(),
        unread: true,
        during: Arc::default(),
    };
    let unread = dash_drafts::card::go(&failing, "d1");
    assert_eq!(unread, Ok(None), "inside its window nothing goes");
    assert!(text(&failing.page()).contains("Sending in 10 s."));
    // its timer late (a restart lost it, or it has not run yet): Undo past the window still stops it, never sends it
    fake.now.fetch_add(card::UNDO_MS, Ordering::SeqCst);
    assert_eq!(
        fake.post("/drafts/undo/d1?thread=brq", ""),
        ("303 See Other".into(), "/mail/t/brq".into())
    );
    fake.pass(0);
    assert_eq!(fake.sent(), 0, "the late timer's run sends nothing");
}

/// /code-review of PR 1640: a check that did not answer (the thread's read timed out) shows no Send but never sends the draft
/// back to its agent; Send and Undo go back to the account the thread was read as; the mail goes as plain text, its
/// `**bold**` as its words.
#[dash_test]
fn a_check_that_did_not_answer_holds_nothing_against_the_draft() {
    let (_d, fake) = Fake::new("draft-card-unread", draft(dash_mail::OWN));
    let unread = Fake {
        sends: fake.sends.clone(),
        dir: fake.dir.clone(),
        now: AtomicI64::new(fake.now_ms()),
        later: Mutex::default(),
        unread: true,
        during: Arc::default(),
    };
    let said = text(&unread.page());
    assert!(
        said.contains("No Send yet: a check did not answer"),
        "{said}"
    );
    assert!(said.contains("its thread did not read"), "{said}");
    assert!(!said.contains("/drafts/send/"), "no Send: {said}");
    assert_eq!(card::back_to_agent(&fake.drafts(), "d1"), Ok(None));
    let (status, body) = unread.post("/drafts/send/d1", "thread=brq&text=Monday");
    assert!(
        status == "409 Conflict" && body.contains("nothing sent"),
        "{body}"
    );
    assert_eq!(card::back_to_agent(&fake.drafts(), "d1"), Ok(None));
    let html = html(&fake.page());
    assert!(
        html.contains(r#"name="account" type="hidden" value="nicolas@klar.im""#),
        "{html}"
    );
    let form = "thread=brq&account=support%40klar.im&text=**Monday**%20at%20ten";
    assert_eq!(
        fake.post("/drafts/send/d1", form),
        (
            "303 See Other".into(),
            "/mail/t/brq?account=support%40klar.im".into()
        )
    );
    let said = text(&fake.page());
    assert!(
        said.contains("Monday at ten") && !said.contains("**"),
        "{said}"
    );
}

/// /code-review of PR 1640: « sending » is recorded before the mail is handed over, so a record that fails after it never
/// offers Send again, and `dash mail send` counts it gone; while the card's Send waits, `dash mail send` holds off; a mail
/// `dash mail send` sent shows on the card as sent; the cards quote the words he sent, his edit included.
#[dash_test]
fn the_card_and_dash_mail_send_never_both_send_it() {
    let (_d, fake) = Fake::new("draft-card-both", draft(dash_mail::OWN));
    fake.post("/drafts/send/d1", "thread=brq&text=Tuesday%20then.");
    let now = fake.now_ms();
    assert!(card::waits(&fake.drafts(), "d1", now).is_err());
    assert!(
        text(&fake.page()).contains("Tuesday then."),
        "his edit quoted"
    );
    fake.pass(card::UNDO_MS);
    let going = json!({"draft": "d1", "email": null, "channel": "email", "tool": "draft-card"});
    assert_eq!(*fake.during.lock().expect("during"), [vec![going]]);
    assert_eq!(card::waits(&fake.drafts(), "d1", now), Ok(()));
    let said = text(&fake.page());
    assert!(
        said.contains("Sent (e1).") && said.contains("Tuesday then."),
        "{said}"
    );
    let (_e, other) = Fake::new("draft-card-went", draft(dash_mail::OWN));
    let line = json!({"draft": "d1", "email": "e9"});
    let db = other.drafts();
    other
        .transact(&db, &mut |db| card::went(db, &line))
        .expect("written");
    let said = text(&other.page());
    assert!(
        said.contains("Sent (e9).") && !said.contains("/drafts/send/"),
        "{said}"
    );
    let (status, _) = other.post("/drafts/send/d1", "thread=brq&text=Monday");
    assert_eq!(status, "409 Conflict");
    assert_eq!(other.sent(), 0);
}

/// Done-when 5: Undo inside the window stops the send; once the send went, Undo says it went.
#[dash_test]
fn undo_stops_the_send_inside_its_window_and_past_it_the_send_goes() {
    let (_dir, fake) = Fake::new("draft-card-undo", draft(dash_mail::OWN));
    let form = "thread=brq&text=Monday";
    fake.post("/drafts/send/d1", form);
    fake.pass(card::UNDO_MS - 1);
    assert_eq!(
        fake.post("/drafts/undo/d1?thread=brq", ""),
        ("303 See Other".into(), "/mail/t/brq".into())
    );
    fake.pass(card::UNDO_MS);
    assert_eq!(fake.sent(), 0, "undone inside its window: nothing sent");
    assert!(
        text(&fake.page()).contains("POST /drafts/send/d1"),
        "Send is back"
    );
    fake.post("/drafts/send/d1", form);
    fake.pass(card::UNDO_MS);
    assert_eq!(
        (fake.sent(), fake.logged()),
        (1, 1),
        "past its window the send goes"
    );
    let (status, body) = fake.post("/drafts/undo/d1?thread=brq", "");
    assert_eq!(
        (status.as_str(), body.as_str()),
        (
            "409 Conflict",
            "Not undone: past its window: it went (e1)\n"
        )
    );
}

// Task rust-draft-corrections, slice 2 of rfc-rust-plugins-ui section 7: his corrections land on the card's words.

const ACCOUNT: &str = "&account=nicolas%40klar.im";

/// His comment rows on the draft's text, as the comments plugin keeps them in comments.sqlite.
fn corrections(fake: &Fake) -> Vec<dash_comments::Row> {
    let log = fake
        .log(&fake.dir, "comments.sqlite", &[dash_comments::SCHEMA])
        .expect("comments.sqlite");
    dash_comments::anchored::on(&*log, &card::anchor("d1")).expect("read")
}

/// The agent's rewrite, as `dash draft revise` writes it.
fn revise(fake: &Fake, text: &str) {
    dash_drafts::command::revise(fake, "d1", "his correction", text).expect("revised");
}

/// Done-when 1: every element of the card carries the core's anchor prop, its text a lasting one, and the text's HTML gives
/// each word its byte range, the spaces between left as they are.
#[dash_test]
fn every_element_carries_the_anchor_and_the_text_its_word_ranges() {
    let (_dir, fake) = Fake::new("draft-corrections-anchor", draft(dash_mail::OWN));
    let page = fake.page();
    let tree: Value = serde_json::from_str(&page).expect("JSON");
    let card: Vec<(&String, &Value)> = (tree["elements"].as_object().expect("elements").iter())
        .filter(|(id, _)| id.starts_with("placed-"))
        .collect();
    assert!(card.len() >= 4, "{card:?}");
    // what it is, the design system's draft card: its goal, from and to, its checks' state
    let (_, top) = card
        .iter()
        .find(|(_, e)| e["type"] == "draft-card")
        .expect("a draft card");
    let props = &top["props"];
    assert_eq!(
        (&props["goal"], &props["to"], &props["state"]),
        (
            &json!("Answer Ada's six questions on the grant"),
            &json!("ada@example.org"),
            &json!("every check passes")
        ),
        "{top}"
    );
    for (id, e) in &card {
        assert!(e["anchor"].is_string(), "{id} carries no anchor: {e}");
    }
    let text = &tree["elements"]["placed-0-0-text"];
    assert_eq!(
        (text["type"].as_str(), text["anchor"].as_str()),
        (Some("quote"), Some("drafts/d1/text"))
    );
    let html = html(&page);
    assert!(html.contains(r#"data-anchor="drafts/d1/text""#), "{html}");
    // each word its range, in order, the HTML's attributes alone (the gate keeps tag literals out of dash's tests)
    let ranges = ["0-6\">Monday", "7-9\">at", "10-14\">ten,", "15-20\">then."];
    let at: Vec<Option<usize>> = (ranges.iter())
        .map(|r| html.find(&format!("data-at=\"{r}")))
        .collect();
    assert!(
        at.iter().all(Option::is_some) && at.is_sorted(),
        "{at:?} in {html}"
    );
}

/// Done-when 3: the Request corrections box's handler files his words on the range of the words he picked, a comment of the
/// queue that wakes the agent, and sends the draft back to its agent: no Send until a new version; words not in the text
/// file nothing.
#[dash_test]
fn request_corrections_files_his_words_on_the_range_and_sends_it_back() {
    let (_dir, fake) = Fake::new("draft-corrections-file", draft(dash_mail::OWN));
    let said = text(&fake.page());
    assert!(
        said.contains("Request corrections: POST /drafts/correct/d1"),
        "{said}"
    );
    let form = format!("thread=brq&quote=at%20ten&text=Say%20eleven{ACCOUNT}");
    assert_eq!(
        fake.post("/drafts/correct/d1", &form),
        (
            "303 See Other".into(),
            "/mail/t/brq?account=nicolas%40klar.im".into()
        )
    );
    let rows = corrections(&fake);
    let [row] = rows.as_slice() else {
        panic!("one comment: {rows:?}")
    };
    assert_eq!(
        (row.anchor.clone(), row.quote.as_str(), row.text.as_str()),
        (
            Some(("drafts/d1/text".into(), 7, 13)),
            "at ten",
            "Say eleven"
        )
    );
    assert_eq!(&"Monday at ten, then."[7..13], "at ten");
    let queue = rows.iter().map(|r| (r.id.clone(), r.clone())).collect();
    let queued = dash_comments::next(&queue);
    assert_eq!(
        queued.map(|r| r.id.as_str()),
        Some(row.id.as_str()),
        "the queue's agent works it next"
    );
    let back = card::back_to_agent(&fake.drafts(), "d1").expect("read");
    assert_eq!(
        back.as_deref(),
        Some("his correction on « at ten »: Say eleven")
    );
    let said = text(&fake.page());
    assert!(
        said.contains("Back to its agent: your correction"),
        "{said}"
    );
    assert!(said.contains("On « at ten »: Say eleven"), "{said}");
    assert!(!said.contains("POST /drafts/send/d1"), "no Send: {said}");
    // /code-review 1 and 6: a Send from a page drawn before the correction sends nothing and keeps the correction's why
    let (status, body) = fake.post("/drafts/send/d1", "thread=brq&text=Monday%20at%20ten");
    assert_eq!(status, "409 Conflict");
    assert!(
        body.contains("you asked corrections on this version"),
        "{body}"
    );
    fake.pass(card::UNDO_MS + 1);
    assert_eq!(fake.sent(), 0);
    let back = card::back_to_agent(&fake.drafts(), "d1").expect("read");
    assert!(back.is_some_and(|why| why.starts_with("his correction")));
    // /code-review 2: words found twice take the place `at` names, else their first
    let twice = |at: &str| format!("thread=brq&quote=en&text=This%20one&at={at}{ACCOUNT}");
    assert_eq!(
        fake.post("/drafts/correct/d1", &twice("17")).0,
        "303 See Other"
    );
    assert_eq!(
        fake.post("/drafts/correct/d1", &twice("2")).0,
        "303 See Other"
    );
    let ranges: Vec<_> = (corrections(&fake).iter())
        .filter(|r| r.quote == "en")
        .map(|r| r.anchor.clone().map(|(_, from, to)| (from, to)))
        .collect();
    assert_eq!(
        ranges,
        [Some((17, 19)), Some((11, 13))],
        "then's, then ten's"
    );
    let (status, body) = fake.post(
        "/drafts/correct/d1",
        "thread=brq&quote=at%20noon&text=Other",
    );
    assert_eq!(status, "409 Conflict");
    assert!(
        body.contains("those words are not in its text") && body.ends_with("Your words:\nOther\n"),
        "{body}"
    );
    assert_eq!(corrections(&fake).len(), 3, "nothing filed");
    let (_shut, closed) = Fake::new(
        "draft-corrections-closed",
        Draft {
            status: "rejected".into(),
            ..draft(dash_mail::OWN)
        },
    );
    let (status, body) = closed.post("/drafts/correct/d1", &form);
    assert_eq!(status, "409 Conflict");
    assert!(body.contains("it is rejected, not open"), "{body}");
    assert!(
        corrections(&closed).is_empty(),
        "a closed draft files nothing"
    );
}

/// Done-when 4: the agent's rewrite is a new version: every check of its channel runs on it (a banned word sends it back, a
/// clean one shows Send), and the earlier versions stay readable.
#[dash_test]
fn the_agent_s_rewrite_is_a_new_version_its_checks_rerun() {
    let (_dir, fake) = Fake::new("draft-corrections-version", draft(dash_mail::OWN));
    let form = format!("thread=brq&quote=&text=Shorter{ACCOUNT}");
    assert_eq!(fake.post("/drafts/correct/d1", &form).0, "303 See Other");
    assert_eq!(
        corrections(&fake)[0].anchor,
        Some(("drafts/d1/text".into(), 0, 20)),
        "no words picked: all of them"
    );
    revise(&fake, "Monday, BANNED.");
    let said = text(&fake.page());
    assert!(said.contains("Back to its agent: no Send"), "{said}");
    assert!(said.contains("outbound gate: a banned phrase"), "{said}");
    revise(&fake, "Monday works.");
    // /code-review 4: a correction answered by a new version no longer holds the draft
    let back = card::back_to_agent(&fake.drafts(), "d1").expect("read");
    assert_eq!(back, None);
    let said = text(&fake.page());
    assert!(
        said.contains("Send to ada@example.org from nicolas@klar.im: POST /drafts/send/d1"),
        "{said}"
    );
    let kept = dash_drafts::load(&fake.drafts()).expect("read").drafts;
    let texts: Vec<&str> = kept["d1"]
        .versions
        .iter()
        .map(|v| v.text.as_str())
        .collect();
    assert_eq!(
        texts,
        ["Monday at ten, then.", "Monday, BANNED.", "Monday works."]
    );
    // /code-review 7: no version is written on a send waiting to go, nor on a closed draft
    let sending = "thread=brq&text=Monday%20works.";
    assert_eq!(fake.post("/drafts/send/d1", sending).0, "303 See Other");
    let refused = dash_drafts::command::revise(&fake, "d1", "late", "Other words");
    assert_eq!(
        refused,
        Err("d1: its send is waiting, going or gone".into())
    );
    let (_shut, closed) = Fake::new(
        "draft-corrections-revise-closed",
        Draft {
            status: "sent".into(),
            ..draft(dash_mail::OWN)
        },
    );
    let refused = dash_drafts::command::revise(&closed, "d1", "late", "Other words");
    assert_eq!(refused, Err("d1: it is sent, not open".into()));
    let shown = dash_drafts::show(&kept["d1"], "2026-11-15");
    assert!(
        shown.contains("v1 on  by claude") && shown.contains("  Monday at ten, then."),
        "{shown}"
    );
}

/// Done-when 5: a comment whose words left the new version shows « on an earlier version »; one whose words stayed shows on
/// them as before.
#[dash_test]
fn a_comment_whose_words_left_shows_on_an_earlier_version() {
    let (_dir, fake) = Fake::new("draft-corrections-earlier", draft(dash_mail::OWN));
    let at = |quote: &str, words: &str| format!("thread=brq&quote={quote}&text={words}{ACCOUNT}");
    assert_eq!(
        fake.post("/drafts/correct/d1", &at("at%20ten", "Say%20eleven"))
            .0,
        "303 See Other"
    );
    assert_eq!(
        fake.post("/drafts/correct/d1", &at("Monday", "Keep%20the%20day"))
            .0,
        "303 See Other"
    );
    revise(&fake, "Monday at eleven, then.");
    let said = text(&fake.page());
    assert!(
        said.contains("On « at ten », on an earlier version: Say eleven"),
        "{said}"
    );
    assert!(said.contains("On « Monday »: Keep the day"), "{said}");
}

/// The drafts file under `dir` holding `drafts`, and its views fed from its changesets, as serve feeds them.
fn viewed(dir: &Path, drafts: &[Draft]) -> (Connection, dash_views::Views) {
    let file = dir.join(dash_drafts::FILE);
    let db = dash_store::plugin::open(&file, &dash_drafts::SCHEMAS, 0).expect("drafts.sqlite");
    let mut kept = |db: &Connection| dash_drafts::save(db, &Kept::default(), drafts).map(drop);
    dash_store::plugin::transact(&db, 1, &mut kept).expect("the drafts");
    let feed = dash_views::Feed {
        read: dash_store::plugin::changes(file, &dash_drafts::SCHEMAS),
        page: 100,
        plugin: Some("drafts table"),
    };
    (db, dash_views::Views::start(vec![feed]).expect("the views"))
}

/// A send's record of draft `id`, kept as its card keeps one.
fn send(db: &Connection, id: &str, state: &str) {
    let sql = "INSERT OR REPLACE INTO draft_sends (id, record) VALUES (?1, ?2)";
    let record = json!({"state": state, "due": 10_000}).to_string();
    let mut put = |db: &Connection| {
        db.execute(sql, [id, &record])
            .map(drop)
            .map_err(|e| e.to_string())
    };
    dash_store::plugin::transact(db, 2, &mut put).expect("the send");
}

/// Done-when 2 of drafts-genuinely-good: each drafts commit's changeset reaches the drafts view, from the file alone, and a
/// subscriber hears a send the moment its card records it: the draft's row moves from no send to « waiting ».
#[dash_test]
fn a_send_s_changeset_reaches_the_drafts_view_s_subscriber() {
    let dir = TestDir::new("drafts-view");
    let (db, views) = viewed(&dir.0, &[draft("Nicolas Ha <nicolas@klar.im>")]);
    let rows = views.rows("drafts").expect("rows");
    assert_eq!(
        (rows.len(), rows[0].1.as_str(), rows[0].3.as_str()),
        (1, "d1", ""),
        "{rows:?}"
    );
    let diffs = views.subscribe("drafts").expect("subscribed");
    diffs
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("its rows first");
    send(&db, "d1", "waiting");
    let diff = diffs
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the send's diff");
    let moved = |send: &str, w: i64| {
        diff.iter()
            .any(|(r, d)| r.1 == "d1" && r.3 == send && *d == w)
    };
    assert!(moved("waiting", 1) && moved("", -1), "{diff:?}");
}

/// The page registered at `path`, served through a host whose views are `views`, as serve hands a page that reads the
/// board: its title and tree.
fn served(
    fake: &Fake,
    views: &dash_views::Views,
    path: &str,
    query: &str,
) -> Result<(String, String), String> {
    let host = dash_store::plugin::Handed {
        host: fake,
        view: Some(views),
    };
    let asked = Asked {
        path,
        query,
        html: false,
        held: None,
        host: &host,
    };
    let page = dash_core::page(&host, path).ok_or("no page")?;
    match (page.serve)(&asked) {
        Answer::Drawn(dash_core::Drawing::Tree { title, tree }) => Ok((title, tree)),
        Answer::Drawn(_) => Err("a page drawn whole".into()),
        Answer::Body(status, _, body) => Err(format!("{status}: {body}")),
    }
}

/// The terminal form of `tree`, as `dash tui` draws it.
fn terminal(tree: &str) -> String {
    let tui = dash_surface::tui::Tui::of(tree).expect("a tree");
    dash_surface::tui::lines(&tui.screen(120, 60).0).join("\n")
}

/// Done-when 3 and 4 of drafts-genuinely-good: /drafts and /drafts/<id> are the drafts plugin's pages, read through the host's
/// views alone (no file, no older app), one tree drawn as HTML, text and dash tui; the list is the design system's draft
/// list (its Send column once a send began, a live part following the drafts view), the draft its document with the facts
/// the import kept; /d/<id> sends to /drafts/<id> only for a draft held.
#[dash_test]
fn the_drafts_pages_read_the_view_and_draw_one_tree_in_three_forms() {
    let (dir, fake) = Fake::new("drafts-pages", draft("Nicolas Ha <nicolas@klar.im>"));
    let mut approved = draft("Nicolas Ha <nicolas@klar.im>");
    (approved.id, approved.status, approved.receiver) =
        ("d2".into(), "approved".into(), "bo@example.org".into());
    let now = fake.now_ms();
    approved.facts = json!({"by_claude": true, "answers": {"apply": "https://acme.example/apply"}, "unread": ["leads: gone"],
        "read_ms": now, "to_send": 0});
    // one the older app no longer holds: kept, off the list
    let mut gone = draft("Nicolas Ha <nicolas@klar.im>");
    (gone.id, gone.receiver, gone.facts) = (
        "d3".into(),
        "cy@example.org".into(),
        json!({"gone": true, "read_ms": now}),
    );
    let (db, views) = viewed(
        &dir.0,
        &[draft("Nicolas Ha <nicolas@klar.im>"), approved, gone],
    );
    send(&db, "d1", "waiting");
    // the views read the send at their next tail: wait for its row
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !views
        .rows("drafts")
        .expect("rows")
        .iter()
        .any(|r| r.3 == "waiting")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the send never reached the view"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let (title, tree) = served(&fake, &views, "/drafts", "").expect("/drafts");
    assert_eq!(title, "Drafts");
    let said = text(&tree);
    let row = said
        .lines()
        .find(|l| l.starts_with("ada@example.org"))
        .unwrap_or_default();
    assert!(
        said.contains("\nTo review\n") && said.contains("\nTo send\n"),
        "{said}"
    );
    assert!(
        said.contains("Scores\tSend\tLink") && row.ends_with("\twaiting\t/drafts/d1"),
        "{said}"
    );
    let page = html(&tree);
    assert!(
        page.contains(r#"id="drafts" data-view="drafts""#) && page.contains(r#"href="/drafts/d2""#),
        "{page}"
    );
    let tui = terminal(&tree);
    assert!(
        tui.contains("ada@example.org") && tui.contains("waiting"),
        "{tui}"
    );
    let (_, to_send) = served(&fake, &views, "/drafts", "show=to-send").expect("to send");
    let to_send = text(&to_send);
    assert!(
        to_send.contains("bo@example.org") && !to_send.contains("ada@example.org"),
        "{to_send}"
    );
    let (title, draft_tree) = served(&fake, &views, "/drafts/d2", "").expect("/drafts/d2");
    assert_eq!(title, "Draft d2");
    let shown = html(&draft_tree);
    assert!(
        shown.contains("acme.example/apply") && !shown.contains(r#"data-view="drafts""#),
        "{shown}"
    );
    assert!(
        text(&draft_tree).contains("What it answers did not read (leads: gone)"),
        "{}",
        text(&draft_tree)
    );
    assert!(
        terminal(&draft_tree).contains("Monday at ten"),
        "{}",
        terminal(&draft_tree)
    );
    assert_eq!(
        served(&fake, &views, "/drafts/none", ""),
        Err("404 Not Found: no draft of that id".into())
    );
    assert_eq!(
        served(&fake, &views, "/d/d1", "view=data"),
        Err("308 Permanent Redirect: /drafts/d1?view=data".into())
    );
    assert_eq!(
        served(&fake, &views, "/d/none", ""),
        Err("404 Not Found: no draft of that id".into())
    );
}
