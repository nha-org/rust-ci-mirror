//! A mail written elsewhere and sent from here (task outbound-gate): the mail it answers, read from its file or found by its
//! subject, whether he already sent it himself, and the mail `submit` makes, on a JMAP server in memory.
use dash_mail::{Copy, OWN, answered, he_sent, original_id, submit, upload_url};
use dash_meter::dash_test;
use serde_json::{Value, json};
use std::cell::RefCell;

/// A JMAP server in memory: each folder's mails as (messageId, subject), and each mail made.
struct Fake {
    inbox: Vec<(&'static str, &'static str)>,
    sent: Vec<(&'static str, &'static str)>,
    drafts: Vec<(&'static str, &'static str)>,
    asked: RefCell<Vec<Value>>,
    made: RefCell<Vec<Value>>,
}

impl Fake {
    fn new() -> Fake {
        Fake {
            inbox: vec![],
            sent: vec![],
            drafts: vec![],
            asked: RefCell::new(vec![]),
            made: RefCell::new(vec![]),
        }
    }

    fn answer(&self, _url: &str, body: Option<&str>) -> Result<String, String> {
        let Some(body) = body else {
            return Ok(json!({"apiUrl": "https://mail.klar.im/jmap/",
                "uploadUrl": "https://mail.klar.im/jmap/upload/{accountId}/",
                "primaryAccounts": {"urn:ietf:params:jmap:mail": "f"}})
            .to_string());
        };
        let asked: Value = serde_json::from_str(body).expect("JSON asked");
        let mut answers = vec![];
        let mut listed: &[(&str, &str)] = &[];
        for call in asked["methodCalls"].as_array().expect("calls") {
            let (name, args, id) = (&call[0], &call[1], &call[2]);
            let result = match name.as_str().expect("a name") {
                "Mailbox/query" => json!({"ids": [args["filter"]["role"]]}),
                "Email/query" => {
                    self.asked.borrow_mut().push(args["filter"].clone());
                    listed = match args["filter"]["inMailbox"].as_str() {
                        Some("inbox") => &self.inbox,
                        Some("sent") => &self.sent,
                        _ => &self.drafts,
                    };
                    json!({"ids": []})
                }
                "Email/get" => json!({"list": listed.iter()
                    .map(|(id, subject)| json!({"messageId": [id], "subject": subject})).collect::<Vec<_>>()}),
                "Identity/get" => {
                    json!({"list": [{"id": "i-own", "email": OWN, "name": "Nicolas Ha"}]})
                }
                "Email/set" => {
                    self.made.borrow_mut().push(args["create"]["draft"].clone());
                    json!({"created": {"draft": {"id": "e1"}}})
                }
                "EmailSubmission/set" => json!({"created": {"send": {"id": "x1"}}}),
                other => panic!("no {other} here"),
            };
            answers.push(json!([name, result, id]));
        }
        Ok(json!({"methodResponses": answers}).to_string())
    }
}

/// The mail a reply answers, from its saved file: a folded and encoded subject read whole, its id and its thread's, no body.
#[dash_test]
fn a_saved_mail_names_what_a_reply_threads_under() {
    let eml = "From: Ada <ada@example.org>\r\nSubject: =?utf-8?B?TWlzc2lvbiBkw6l2?=\r\n =?utf-8?Q?_=C3=A0_Paris?= (remote)\r\nMessage-ID: <m2@example.org>\r\nReferences: <m0@example.org>\r\n <m1@example.org>\r\n\r\nSubject: not this one\r\n";
    let a = answered(eml);
    assert_eq!(a.subject, "Mission dév à Paris (remote)");
    assert_eq!(a.message_id.as_deref(), Some("m2@example.org"));
    assert_eq!(
        a.references,
        ["m0@example.org", "m1@example.org", "m2@example.org"]
    );
    let latin = answered("Subject: =?iso-8859-1?Q?Caf=E9?=\n\n");
    assert_eq!((latin.subject.as_str(), latin.message_id), ("Café", None));
    assert_eq!(answered("no headers here").references, [] as [&str; 0]);
}

/// The newest mail titled exactly so, in the inbox then in Sent: a later « Re: » of the subject is another mail.
#[dash_test]
fn the_mail_a_subject_names_is_found_in_the_inbox_then_sent() {
    let mut server = Fake::new();
    server.inbox = vec![("re@x", "Re: Mission Kafka"), ("first@x", "Mission  Kafka")];
    let http = |url: &str, body: Option<&str>| server.answer(url, body);
    assert_eq!(
        original_id(&http, "Mission Kafka"),
        Ok(Some("first@x".into()))
    );
    let mut server = Fake::new();
    server.sent = vec![("mine@x", "Mission Kafka")];
    let http = |url: &str, body: Option<&str>| server.answer(url, body);
    assert_eq!(
        original_id(&http, "Mission Kafka"),
        Ok(Some("mine@x".into()))
    );
    assert_eq!(original_id(&http, "Autre"), Ok(None));
}

fn copy() -> Copy {
    Copy {
        message_id: "<ours@klar.im>".into(),
        to: "ada@example.org".into(),
        subject: "Mission Kafka".into(),
        made: "2026-11-12T08:00:00Z".into(),
    }
}

/// He sent it himself: our copy left his Drafts and Sent holds it since. Still in Drafts, ours or his edit of it, is not
/// sent; another mail's copy to the same person with the same subject is not his edit of this one.
#[dash_test]
fn a_copy_gone_from_drafts_and_in_sent_was_sent_by_him() {
    let said = |drafts: Vec<(&'static str, &'static str)>,
                sent: Vec<(&'static str, &'static str)>| {
        let mut server = Fake::new();
        (server.drafts, server.sent) = (drafts, sent);
        let http = |url: &str, body: Option<&str>| server.answer(url, body);
        let got = he_sent(&http, &copy(), &["<other@klar.im>".into()]);
        (got, server.asked.borrow().last().cloned())
    };
    let sent = vec![("his@x", "Mission Kafka")];
    let (got, asked) = said(vec![], sent.clone());
    assert_eq!(got, Ok(true));
    assert_eq!(
        asked,
        Some(
            json!({"inMailbox": "sent", "to": "ada@example.org", "subject": "Mission Kafka",
                    "after": "2026-11-12T08:00:00Z"})
        )
    );
    assert_eq!(
        said(vec![("ours@klar.im", "Mission Kafka")], sent.clone()).0,
        Ok(false)
    );
    assert_eq!(
        said(vec![("edit@x", "Mission Kafka")], sent.clone()).0,
        Ok(false)
    );
    assert_eq!(
        said(vec![("other@klar.im", "Mission Kafka")], sent).0,
        Ok(true)
    );
    assert_eq!(
        said(vec![], vec![("re@x", "Re: Mission Kafka")]).0,
        Ok(false)
    );
    assert_eq!(said(vec![], vec![]).0, Ok(false));
}

/// The mail `submit` makes carries its head whole: the copies, the thread it answers and its files by blob.
#[dash_test]
fn a_submitted_mail_carries_its_thread_and_its_files() {
    let server = Fake::new();
    let http = |url: &str, body: Option<&str>| server.answer(url, body);
    let head = json!({"to": [{"email": "ada@example.org"}], "cc": [{"email": "bob@example.org"}],
        "subject": "Re: Mission Kafka", "inReplyTo": ["m2@example.org"], "references": ["m1@example.org", "m2@example.org"],
        "attachments": [{"blobId": "b1", "type": "application/pdf", "name": "a.pdf", "disposition": "attachment"}]});
    assert_eq!(
        submit(&http, OWN, head, "Bonjour", Some("<p>Bonjour</p>")),
        Ok("e1".into())
    );
    let made = server.made.borrow()[0].clone();
    assert_eq!(made["from"][0]["email"], OWN);
    assert_eq!(made["cc"][0]["email"], "bob@example.org");
    assert_eq!(made["inReplyTo"], json!(["m2@example.org"]));
    assert_eq!(
        made["references"],
        json!(["m1@example.org", "m2@example.org"])
    );
    assert_eq!(made["attachments"][0]["blobId"], "b1");
    assert_eq!(made["bodyValues"]["body"]["value"], "Bonjour");
    assert_eq!(made["bodyValues"]["html"]["value"], "<p>Bonjour</p>");
    assert_eq!(
        upload_url(&http),
        Ok("https://mail.klar.im/jmap/upload/f/".into())
    );
}
