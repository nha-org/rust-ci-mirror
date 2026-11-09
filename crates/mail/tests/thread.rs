//! /mail/t/<thread> on a JMAP fixture (thread.json: a conversation of three mails, listed out of order, the latest with an HTML
//! body that would run a script, refresh to a tracker and load a remote image), and his reply on a fake JMAP server that keeps
//! what it was asked to make and send. The page's HTML and text from this tree: dash's mail.rs tests.
use dash_mail::{OWN, SCHEMA, Thread, reply, send_once, thread, thread_tree};
use dash_meter::memory::TestBudget;
use serde_json::{Value, json};
use std::cell::RefCell;

fn recorded() -> Value {
    serde_json::from_str(include_str!("thread.json")).expect("the fixture")
}

fn read() -> Thread {
    let fixture = recorded();
    let http = |url: &str, body: Option<&str>| match (url, body) {
        (dash_mail::SESSION, None) => Ok(fixture["session"].to_string()),
        ("https://mail.klar.im/jmap/", Some(_)) => Ok(fixture["thread"].to_string()),
        _ => panic!("asked {url}"),
    };
    thread(&http, "brq").expect("the thread")
}

#[test]
fn the_thread_stacks_its_mails_oldest_first_the_older_folded_the_latest_open() {
    let _budget = TestBudget::new();
    let t = read();
    assert_eq!(t.subject, "The grant");
    let ids: Vec<&str> = t.mails.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        ["m1", "m2", "m3"],
        "oldest first, whatever the list's order"
    );
    assert_eq!(t.mails[1].from, OWN, "no name: the address");
    assert_eq!(t.mails[1].to, ["ada@example.org", "bob@example.org"]);
    assert_eq!(t.mails[2].files[0].blob, "b1");

    let tree = thread_tree(OWN, "brq", &Ok(t), None);
    let page: Value = serde_json::from_str(&tree).expect("a tree");
    let e = &page["elements"];
    let open: Vec<bool> = (0..3)
        .map(|i| e[format!("mail-{i}")]["props"]["open"] == true)
        .collect();
    assert_eq!(open, [false, false, true], "the latest open");
    assert_eq!(
        e["note"]["props"]["text"],
        "3 mails, as nicolas@klar.im reads them."
    );
    assert_eq!(
        e["reply"]["props"]["send"],
        "Send to ada@example.org from nicolas@klar.im"
    );
    assert_eq!(e["reply"]["props"]["action"], "/mail/t/brq/reply");

    assert_eq!(
        e["mail-2"]["props"]["files"],
        json!([[
            "slots.pdf",
            "/mail/blob/b1?name=slots.pdf&type=application%2Fpdf&account=nicolas@klar.im"
        ]])
    );
    assert_eq!(
        e["page"]["children"],
        json!(["title", "note", "mail-0", "mail-1", "mail-2", "reply"])
    );
}

#[test]
fn a_thread_the_server_lacks_or_refuses_is_said() {
    let _budget = TestBudget::new();
    let fixture = recorded();
    let empty = |url: &str, _: Option<&str>| match url {
        dash_mail::SESSION => Ok(fixture["session"].to_string()),
        _ => Ok(
            r#"{"methodResponses":[["Thread/get",{"list":[]},"t"],["Email/get",{"list":[]},"e"]]}"#
                .into(),
        ),
    };
    let missing = thread(&empty, "zz");
    assert_eq!(missing, Err("no conversation zz on this server".into()));
    let tree = thread_tree(OWN, "zz", &missing, None);
    let page: Value = serde_json::from_str(&tree).expect("a tree");
    assert_eq!(
        page["elements"]["note"]["props"]["text"],
        "Conversation zz of nicolas@klar.im: no conversation zz on this server"
    );
    assert_eq!(
        page["elements"]["page"]["children"],
        json!(["title", "note"]),
        "no reply box without a thread"
    );
}

/// A JMAP server in memory: the session, his identity, Drafts and Sent, and each mail made and submission sent, kept.
struct Fake {
    made: RefCell<Vec<Value>>,
    submitted: RefCell<Vec<Value>>,
    destroyed: RefCell<Vec<Value>>,
    refuse_submission: bool,
}

impl Fake {
    fn new(refuse_submission: bool) -> Fake {
        Fake {
            made: RefCell::new(vec![]),
            submitted: RefCell::new(vec![]),
            destroyed: RefCell::new(vec![]),
            refuse_submission,
        }
    }

    fn answer(&self, url: &str, body: Option<&str>) -> Result<String, String> {
        let Some(body) = body else {
            assert_eq!(url, dash_mail::SESSION);
            return Ok(recorded()["session"].to_string());
        };
        let asked: Value = serde_json::from_str(body).expect("JSON asked");
        let using = asked["using"].as_array().expect("using");
        let mut answers = vec![];
        for call in asked["methodCalls"].as_array().expect("calls") {
            let (name, args, id) = (&call[0], &call[1], &call[2]);
            let needs = match name.as_str() {
                Some("Identity/get" | "EmailSubmission/set") => "urn:ietf:params:jmap:submission",
                _ => "urn:ietf:params:jmap:mail",
            };
            assert!(using.iter().any(|u| u == needs), "{name} without {needs}");
            let result = match name.as_str().expect("a name") {
                "Identity/get" => json!({"list": [
                    {"id": "i-agent", "email": "agent@klar.im", "name": "Agent"},
                    {"id": "i-own", "email": "nicolas@klar.im", "name": "Nicolas Ha"}]}),
                "Mailbox/query" if args["filter"]["role"] == "drafts" => json!({"ids": ["d"]}),
                "Mailbox/query" if args["filter"]["role"] == "sent" => json!({"ids": ["s"]}),
                "Thread/get" | "Email/get" => return Ok(recorded()["thread"].to_string()),
                "Email/set" if args.get("destroy").is_some() => {
                    self.destroyed.borrow_mut().push(args["destroy"].clone());
                    json!({"destroyed": args["destroy"]})
                }
                "Email/set" => {
                    self.made.borrow_mut().push(args["create"]["draft"].clone());
                    json!({"created": {"draft": {"id": format!("e{}", self.made.borrow().len())}}})
                }
                "EmailSubmission/set" if self.refuse_submission => {
                    json!({"notCreated": {"send": {"type": "forbiddenFrom"}}})
                }
                "EmailSubmission/set" => {
                    assert_eq!(args["create"]["send"]["emailId"], "#draft");
                    self.submitted.borrow_mut().push(args.clone());
                    json!({"created": {"send": {"id": "x1"}}})
                }
                other => panic!("no {other} here"),
            };
            answers.push(json!([name, result, id]));
        }
        Ok(json!({"methodResponses": answers}).to_string())
    }
}

#[test]
fn a_reply_goes_from_his_own_address_to_their_latest_mail() {
    let _budget = TestBudget::new();
    let server = Fake::new(false);
    let http = |url: &str, body: Option<&str>| server.answer(url, body);
    let t = read();
    assert_eq!(
        reply(&http, OWN, &t, "  Monday at ten, then.  "),
        Ok("e1".into())
    );
    let made = server.made.borrow()[0].clone();
    assert_eq!(
        made["from"],
        json!([{"name": "Nicolas Ha", "email": "nicolas@klar.im"}])
    );
    assert_eq!(made["to"], json!([{"email": "ada@example.org"}]));
    assert_eq!(made["subject"], "Re: The grant");
    assert_eq!(made["inReplyTo"], json!(["c3@example.org"]));
    assert_eq!(
        made["references"],
        json!(["c1@example.org", "c2@klar.im", "c3@example.org"])
    );
    assert_eq!(made["bodyValues"]["body"]["value"], "Monday at ten, then.");
    assert_eq!(made["mailboxIds"], json!({"d": true}));
    let sent = server.submitted.borrow()[0].clone();
    assert_eq!(sent["create"]["send"]["identityId"], "i-own");
    assert_eq!(
        sent["onSuccessUpdateEmail"]["#send"],
        json!({"mailboxIds/d": null, "mailboxIds/s": true, "keywords/$draft": null})
    );

    assert_eq!(
        reply(&http, OWN, &t, " \n "),
        Err("an empty reply is not sent".into())
    );
    assert_eq!(
        reply(&http, "nobody@klar.im", &t, "Hi"),
        Err("jmap: nobody@klar.im has no identity to send from".into())
    );
    assert_eq!(server.made.borrow().len(), 1, "nothing made for a refusal");
}

use dash_core::MemoryLog as Log;

#[test]
fn each_send_is_logged_once_and_the_same_form_twice_sends_once() {
    let _budget = TestBudget::new();
    assert!(SCHEMA.contains("mail_sent"));
    let server = Fake::new(false);
    let http = |url: &str, body: Option<&str>| server.answer(url, body);
    let (t, mut log) = (read(), Log::default());
    let at = "2026-11-09T10:00:00Z";
    let sent = send_once(&http, &mut log, OWN, &t, "Monday at ten.", at).expect("sent");
    assert_eq!((sent.email.as_str(), &sent.logged), ("e1", &Ok(())));
    assert_eq!(
        (
            &sent.record["date"],
            &sent.record["tool"],
            &sent.record["channel"]
        ),
        (&json!("2026-11-09"), &json!("mail-page"), &json!("email")),
        "the older app's outbound line, as Webmail.send wrote it"
    );
    assert_eq!(log.0.len(), 1, "one row a send");
    let row: Value = match &log.0[0].write.value {
        dash_core::Value::Text(t) => serde_json::from_str(t).expect("a record"),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        (
            &row["from"],
            &row["to"],
            &row["email"],
            &row["thread"],
            &row["at"]
        ),
        (
            &json!(OWN),
            &json!("ada@example.org"),
            &json!("e1"),
            &json!("brq"),
            &json!(at)
        )
    );

    let again = send_once(&http, &mut log, OWN, &t, " Monday at ten. ", at);
    assert_eq!(
        again.map(|s| s.email),
        Err("this reply already went (the send log): nothing sent again".into())
    );
    assert_eq!(
        (server.submitted.borrow().len(), log.0.len()),
        (1, 1),
        "sent once, logged once"
    );

    let ps = send_once(&http, &mut log, OWN, &t, "And a P.S.", at);
    assert_eq!(ps.map(|s| s.email), Ok("e2".into()));
    assert_eq!(
        (server.submitted.borrow().len(), log.0.len()),
        (2, 2),
        "another reply is another send"
    );
}

#[test]
fn a_refused_submission_leaves_no_draft_and_no_log() {
    let _budget = TestBudget::new();
    let server = Fake::new(true);
    let http = |url: &str, body: Option<&str>| server.answer(url, body);
    let (t, mut log) = (read(), Log::default());
    let refused = send_once(&http, &mut log, OWN, &t, "Monday.", "2026-11-09T10:00:00Z");
    assert_eq!(
        refused.map(|s| s.email),
        Err(r#"jmap: the server did not send it: {"send":{"type":"forbiddenFrom"}}"#.into())
    );
    assert_eq!(
        server.destroyed.borrow().as_slice(),
        [json!(["e1"])],
        "the draft made goes"
    );
    assert!(log.0.is_empty(), "nothing went, nothing logged");
}

/// A send that went is said as sent even when its row did not write: an error would read as not sent, and he would send it
/// again (/code-review of rust-mail-thread).
#[test]
fn a_send_whose_log_fails_still_says_it_went() {
    let _budget = TestBudget::new();
    let server = Fake::new(false);
    let http = |url: &str, body: Option<&str>| server.answer(url, body);
    let mut log = Log(vec![], true);
    let sent = send_once(
        &http,
        &mut log,
        OWN,
        &read(),
        "Monday.",
        "2026-11-09T10:00:00Z",
    );
    let sent = sent.expect("it went");
    assert_eq!(
        (sent.email.as_str(), sent.logged),
        ("e1", Err("disk full".into()))
    );
}

/// A mail with no sender's address (a server's notice) is answered by nobody: the reply goes to the latest real sender
/// (/code-review of rust-mail-thread).
#[test]
fn a_mail_with_no_sender_is_never_answered() {
    let _budget = TestBudget::new();
    let mut t = read();
    let mut notice = t.mails[2].clone();
    (notice.id, notice.address, notice.from) = ("m4".into(), String::new(), String::new());
    t.mails.push(notice);
    let tree: Value = serde_json::from_str(&thread_tree(OWN, "brq", &Ok(t), None)).expect("a tree");
    assert_eq!(
        tree["elements"]["reply"]["props"]["send"],
        "Send to ada@example.org from nicolas@klar.im"
    );
}
