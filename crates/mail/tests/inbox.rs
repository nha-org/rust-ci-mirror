//! The inbox read on klar's server, replayed: inbox.json holds its answers as recorded on 2026-11-09 (session, the inbox's
//! id, its first 50 conversations of 359 with their threads' mails), each subject, sender and preview replaced, and the
//! shown mails' list reversed, so the order shown is the query's and never the list's.
use dash_mail::{Asked, Listed, OWN, PAGE, accounts, list, tree};
use dash_meter::memory::TestBudget;
use serde_json::Value;
use std::cell::RefCell;

fn recorded() -> Value {
    serde_json::from_str(include_str!("inbox.json")).expect("the fixture")
}

/// The recorded answers in turn, and each request as it was asked.
fn replayed(what: &Asked) -> (Result<Listed, String>, Vec<(String, Option<Value>)>) {
    replayed_from(recorded(), what)
}

fn replayed_from(
    fixture: Value,
    what: &Asked,
) -> (Result<Listed, String>, Vec<(String, Option<Value>)>) {
    let asked = RefCell::new(vec![]);
    let http = |url: &str, body: Option<&str>| {
        let body = body.map(|b| serde_json::from_str::<Value>(b).expect("JSON asked"));
        let answer = match (url, &body) {
            (dash_mail::SESSION, None) => &fixture["session"],
            ("https://mail.klar.im/jmap/", Some(b))
                if b["methodCalls"][0][0] == "Mailbox/query" =>
            {
                &fixture["mailbox"]
            }
            ("https://mail.klar.im/jmap/", Some(_)) => &fixture["inbox"],
            _ => panic!("asked {url}"),
        };
        asked.borrow_mut().push((url.to_string(), body));
        Ok(answer.to_string())
    };
    let read = list(&http, what);
    (read, asked.into_inner())
}

#[test]
fn the_inbox_lists_50_to_a_page_newest_first() {
    let _budget = TestBudget::new();
    let (read, asked) = replayed(&Asked::of(None, None, None));
    let read = read.expect("the inbox");
    assert_eq!((read.rows.len() as u64, read.total), (PAGE, 359));
    assert_eq!(read.rows[0].subject, "Subject 1");
    assert_eq!(read.rows[0].at, "2026-11-08T20:28:45Z");
    assert!(
        read.rows.windows(2).all(|w| w[0].at >= w[1].at),
        "newest first"
    );
    assert_eq!(
        (read.rows[0].from.as_str(), read.rows[0].thread.as_str()),
        ("Sender 1", "brq")
    );
    assert_eq!(
        read.rows.iter().filter(|r| !r.unread).count(),
        14,
        "the $seen ones"
    );

    assert_eq!(asked.len(), 3);
    let query = &asked[2].1.as_ref().expect("a POST")["methodCalls"][0][1];
    assert_eq!(
        query["filter"]["inMailbox"], "a",
        "the inbox the server named"
    );
    assert_eq!(
        (query["limit"].as_u64(), query["position"].as_u64()),
        (Some(50), Some(0))
    );
    assert_eq!(query["sort"][0]["isAscending"], false);
    assert_eq!(query["accountId"], "f");
    let (_, asked) = replayed(&Asked::of(Some("inbox"), Some(" "), Some(3)));
    assert_eq!(
        asked[2].1.as_ref().expect("a POST")["methodCalls"][0][1]["position"],
        100
    );
}

#[test]
fn the_page_shows_each_conversation_and_the_pages_around_it() {
    let _budget = TestBudget::new();
    let second = Asked::of(None, None, Some(2));
    let (read, _) = replayed(&second);
    let today = dash_core::days("2026-11-08").expect("a day");
    let page: Value =
        serde_json::from_str(&tree(OWN, &[OWN.into()], &second, &read, today)).expect("a tree");
    let e = &page["elements"];
    assert_eq!(
        e["note"]["props"]["text"],
        "Inbox of nicolas@klar.im: 51 to 100 of 359."
    );
    let first = &e["list-0"]["props"];
    assert_eq!(first["title"], "Subject 1");
    assert_eq!(
        first["line"],
        "Sender 1 · 20:28 UTC · unread · Preview of mail 1"
    );
    assert_eq!(first["href"], "/mail/t/brq?account=nicolas@klar.im");
    assert_eq!(
        e["list-49"]["props"]["line"]
            .as_str()
            .map(|l| l.contains("2026-11-05")),
        Some(true)
    );
    assert_eq!(
        e["pages-0"]["props"]["href"], "/mail",
        "page 1 says no page"
    );
    assert_eq!(e["pages-1"]["props"]["href"], "/mail?page=3");
    assert!(e.get("accounts").is_none(), "one address, no switch");

    let both = [OWN.to_string(), "support@klar.im".into()];
    let page: Value = serde_json::from_str(&tree("support@klar.im", &both, &second, &read, today))
        .expect("a tree");
    let e = &page["elements"];
    assert_eq!(e["accounts-0"]["props"]["href"], "/mail");
    assert_eq!(e["accounts-1"]["props"]["line"], "shown");
    assert_eq!(
        e["list-0"]["props"]["href"],
        "/mail/t/brq?account=support@klar.im"
    );
    assert_eq!(
        e["pages-1"]["props"]["href"],
        "/mail?account=support@klar.im&page=3"
    );
}

#[test]
fn a_refusal_is_said_and_the_switch_keeps_his_own_first() {
    let _budget = TestBudget::new();
    let refused =
        |_: &str, _: Option<&str>| Err("jmap: mail.klar.im refused the password".to_string());
    let inbox = Asked::of(None, None, None);
    let page = tree(OWN, &[OWN.into()], &inbox, &list(&refused, &inbox), 0);
    assert!(
        page.contains("Inbox of nicolas@klar.im: jmap: mail.klar.im refused the password"),
        "{page}"
    );
    let failed = |url: &str, _: Option<&str>| match url {
        dash_mail::SESSION => Ok(recorded()["session"].to_string()),
        _ => Ok(r#"{"methodResponses":[["error",{"type":"accountNotFound"},"b"]]}"#.into()),
    };
    assert_eq!(
        list(&failed, &inbox),
        Err("jmap: b failed: accountNotFound".into())
    );
    assert_eq!(
        accounts(|a| a == "support@klar.im"),
        [OWN, "support@klar.im"]
    );
    assert_eq!(accounts(|_| false), [OWN]);
}

#[test]
fn a_folder_and_a_search_the_thread_page_links_to_are_asked_of_the_server() {
    let _budget = TestBudget::new();
    let sent = Asked::of(Some("sent"), Some("  grant  "), Some(2));
    let (read, asked) = replayed(&sent);
    assert_eq!(
        asked[1].1.as_ref().expect("a POST")["methodCalls"][0][1]["filter"]["role"],
        "sent"
    );
    let filter = &asked[2].1.as_ref().expect("a POST")["methodCalls"][0][1]["filter"];
    assert_eq!(filter["operator"], "AND");
    assert_eq!(filter["conditions"][0]["inMailbox"], "a");
    assert_eq!(filter["conditions"][1]["text"], "grant");
    let page: Value =
        serde_json::from_str(&tree(OWN, &[OWN.into()], &sent, &read, 0)).expect("a tree");
    let e = &page["elements"];
    assert_eq!(
        e["note"]["props"]["text"],
        "Sent of nicolas@klar.im matching « grant »: 51 to 100 of 359."
    );
    assert_eq!(
        e["pages-1"]["props"]["href"],
        "/mail?box=sent&q=grant&page=3"
    );

    let (_, asked) = replayed(&Asked::of(Some("starred"), None, None));
    assert_eq!(asked.len(), 2, "a keyword, no folder to find");
    assert_eq!(
        asked[1].1.as_ref().expect("a POST")["methodCalls"][0][1]["filter"]["hasKeyword"],
        "$flagged"
    );
    assert_eq!(
        Asked::of(Some("label:_TrainSpam"), Some(""), Some(0)),
        Asked::of(None, None, None)
    );
    assert_eq!(
        Asked::of(None, Some("a&b c"), None).q.as_deref(),
        Some("a&b c")
    );
}

/// Gmail marks a conversation unread when any of its mails is, not only the one shown.
#[test]
fn a_thread_with_an_older_mail_unread_is_unread() {
    let _budget = TestBudget::new();
    let mut fixture = recorded();
    let inbox = Asked::of(None, None, None);
    let (read, _) = replayed_from(fixture.clone(), &inbox);
    let seen = read
        .expect("the inbox")
        .rows
        .into_iter()
        .find(|r| !r.unread)
        .expect("a read one");
    let all = &mut fixture["inbox"]["methodResponses"][3][1]["list"];
    all.as_array_mut()
        .expect("the threads' mails")
        .push(serde_json::json!({"id": "older", "threadId": seen.thread, "keywords": {}}));
    let (read, _) = replayed_from(fixture, &inbox);
    let row = read
        .expect("the inbox")
        .rows
        .into_iter()
        .find(|r| r.thread == seen.thread);
    assert_eq!(row.map(|r| r.unread), Some(true));
    assert_eq!(
        Asked::of(None, None, Some(u64::MAX)).page,
        dash_mail::LAST,
        "no position overflows"
    );
}

/// rust-daily-today: a reply waiting is an unread conversation of the inbox holding a mail of his Sent folder; a newsletter he
/// never wrote in is not one, nor a read one.
#[test]
fn replies_waiting_count_unread_conversations_he_wrote_in() {
    let _budget = TestBudget::new();
    let session = recorded()["session"].clone();
    let asked = RefCell::new(vec![]);
    let answers = |boxes: serde_json::Value| {
        let http = |url: &str, body: Option<&str>| -> Result<String, String> {
            let body = body.map(|b| serde_json::from_str::<Value>(b).expect("JSON asked"));
            let answer = match &body {
                None => session.clone(),
                Some(b) if b["methodCalls"][0][0] == "Mailbox/query" => boxes.clone(),
                Some(_) => serde_json::json!({"methodResponses": [
                    ["Email/query", {"ids": ["a1", "n1"]}, "q"],
                    ["Email/get", {"list": []}, "e"],
                    ["Thread/get", {"list": []}, "t"],
                    ["Email/get", {"list": [
                        {"id": "a0", "threadId": "ta", "mailboxIds": {"SENT": true}},
                        {"id": "a1", "threadId": "ta", "mailboxIds": {"INBOX": true}},
                        {"id": "a2", "threadId": "ta", "mailboxIds": {"SENT": true}},
                        {"id": "n1", "threadId": "tn", "mailboxIds": {"INBOX": true}}
                    ]}, "all"]
                ]}),
            };
            asked.borrow_mut().push((url.to_string(), body));
            Ok(answer.to_string())
        };
        dash_mail::replies_waiting(&http)
    };
    let both = serde_json::json!({"methodResponses": [
        ["Mailbox/query", {"ids": ["INBOX"]}, "i"], ["Mailbox/query", {"ids": ["SENT"]}, "s"]]});
    assert_eq!(
        answers(both),
        Ok(1),
        "one conversation he wrote in, counted once"
    );
    let unread = asked.borrow()[2].1.clone().expect("a body");
    assert_eq!(
        unread["methodCalls"][0][1]["filter"]["conditions"][1],
        serde_json::json!({"notKeyword": "$seen"})
    );
    let no_sent = serde_json::json!({"methodResponses": [
        ["Mailbox/query", {"ids": ["INBOX"]}, "i"], ["Mailbox/query", {"ids": []}, "s"]]});
    assert!(
        answers(no_sent).is_err(),
        "no Sent folder is said, never a zero"
    );
}
