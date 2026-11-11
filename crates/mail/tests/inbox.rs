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

/// What a replay gives: the listing, then each request as it was asked.
type Replay = (Result<Listed, String>, Vec<(String, Option<Value>)>);

/// The recorded answers in turn, and each request as it was asked.
fn replayed(what: &Asked) -> Replay {
    replayed_from(recorded(), what)
}

fn replayed_from(fixture: Value, what: &Asked) -> Replay {
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
    assert_eq!(e["bar"]["props"]["range"], "51–100 of 359");
    let first = &e["list-0"];
    let count = first["props"]["count"].clone();
    assert_eq!(
        first,
        &serde_json::json!({"type": "thread-row", "props": {"from": "Sender 1", "subject": "Subject 1",
            "snippet": "Preview of mail 1", "at": "20:28", "unread": true,
            "href": "/mail/t/brq?account=nicolas@klar.im", "count": count, "starred": false}})
    );
    assert_eq!(e["list-49"]["props"]["at"], "5 Nov");
    assert_eq!(e["bar"]["props"]["newer"], "/mail", "page 1 says no page");
    assert_eq!(e["bar"]["props"]["older"], "/mail?page=3");
    assert_eq!(
        e["page"]["props"]["accounts"].as_array().map(Vec::len),
        Some(1),
        "his own alone"
    );

    let both = [OWN.to_string(), "support@klar.im".into()];
    let page: Value = serde_json::from_str(&tree("support@klar.im", &both, &second, &read, today))
        .expect("a tree");
    let e = &page["elements"];
    let switch = &e["page"]["props"]["accounts"];
    assert_eq!(switch[0]["href"], "/mail");
    assert_eq!(switch[1]["current"], true);
    // task mail-no-cards (@nha, 2026-11-09: « cards are nonsense here »): threads are rows, the switch and the pager links
    assert_eq!(
        (
            e["list"]["type"].as_str(),
            e["page"]["type"].as_str(),
            e["bar"]["type"].as_str()
        ),
        (Some("list"), Some("mail-shell"), Some("mail-bar"))
    );
    assert_eq!(
        e["list-0"]["props"]["href"],
        "/mail/t/brq?account=support@klar.im"
    );
    assert_eq!(
        e["bar"]["props"]["older"],
        "/mail?account=support@klar.im&page=3"
    );
    assert_eq!(
        e["page"]["props"]["compose"],
        "/mail/compose?account=support@klar.im"
    );
    assert_eq!(
        e["page"]["props"]["hidden"],
        serde_json::json!([["account", "support@klar.im"]])
    );
}

/// Done-when 3 of rust-mail-design-parts: every row of /mail is /design?tab=mail's thread row, in the page's HTML and its
/// text form, both drawn from the tree by the design system as dash serve draws them (@nha, 2026-11-09: « Are the elements
/// actually used in /mail ? they do not match the previous app /mail »).
#[test]
fn each_row_is_the_thread_row_in_html_and_text() {
    let _budget = TestBudget::new();
    let inbox = Asked::of(None, None, None);
    let (read, _) = replayed(&inbox);
    let today = dash_core::days("2026-11-08").expect("a day");
    let tree = tree(OWN, &[OWN.into()], &inbox, &read, today);
    let page: Value = serde_json::from_str(&tree).expect("a tree");
    let rows = page["elements"]["list"]["children"]
        .as_array()
        .expect("the list's rows");
    assert_eq!(rows.len(), 50);
    assert!(
        rows.iter()
            .all(|id| page["elements"][id.as_str().expect("an id")]["type"] == "thread-row"),
        "every row a thread row"
    );

    let html = dash_surface::screen::page("Mail", &tree).expect("the page");
    let first = &html[html.find(r#"<a id="list-0" class="thread-row unread" href="/mail/t/brq?account=nicolas@klar.im">"#).expect(&html)..];
    let first = &first[..first.find("</a>").expect("its end")];
    assert!(
        first.contains(r#"<span role="img" class="star" aria-label="Not starred">"#),
        "{first}"
    );
    assert!(
        first.ends_with(concat!(
            r#"<span class="words"><span class="topic">Subject 1</span>"#,
            r#"<span class="gist"> - Preview of mail 1</span></span><span class="at">20:28</span>"#
        )),
        "{first}"
    );
    assert_eq!(
        html.matches(r#"class="thread-row"#).count(),
        50,
        "a row each, no card"
    );
    assert_eq!(
        html.matches(r#"class="thread-row""#).count(),
        14,
        "the $seen ones on grey"
    );
    assert!(
        !html.contains(r#"class="item issue-row"#),
        "no generic row left"
    );

    let text = dash_surface::text(&tree).expect("the text");
    assert!(
        text.contains("Sender 1\tSubject 1\tPreview of mail 1\t20:28\t"),
        "{text}"
    );
    assert!(
        text.contains("\tunread\t\t/mail/t/brq?account=nicolas@klar.im\n"),
        "{text}"
    );
    assert_eq!(text.matches("\tread\t\t/mail/t/").count(), 14, "{text}");
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
    assert_eq!(e["bar"]["props"]["range"], "51–100 of 359");
    assert_eq!(e["page"]["props"]["search"], "grant");
    assert_eq!(
        e["page"]["props"]["hidden"],
        serde_json::json!([["box", "sent"]])
    );
    assert_eq!(e["bar"]["props"]["older"], "/mail?box=sent&q=grant&page=3");

    let (_, asked) = replayed(&Asked::of(Some("starred"), None, None));
    assert_eq!(asked.len(), 2, "a keyword, no folder to find");
    assert_eq!(
        asked[1].1.as_ref().expect("a POST")["methodCalls"][0][1]["filter"]["hasKeyword"],
        "$flagged"
    );
    assert_eq!(
        Asked::of(Some("label:a/b"), Some(""), Some(0)),
        Asked::of(None, None, None),
        "a label's id is JMAP's characters alone"
    );
    let (_, asked) = replayed(&Asked::of(Some("label:j"), None, None));
    assert_eq!(asked.len(), 2, "a label's id, no folder to find");
    assert_eq!(
        asked[1].1.as_ref().expect("a POST")["methodCalls"][0][1]["filter"]["inMailbox"],
        "j"
    );
    let (_, asked) = replayed(&Asked::of(Some("all"), Some("grant"), None));
    assert_eq!(
        asked[1].1.as_ref().expect("a POST")["methodCalls"][0][1]["filter"],
        serde_json::json!({"text": "grant"}),
        "All mail: every folder"
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

/// Done-when 2 of rust-mail-parity, the side column: the folders in the older app's order with their counts (unread; Drafts,
/// all), then his labels by name, each linking to its list; the switch, Compose and the search in the frame; a starred
/// conversation's star and its count of mails on its row; « me » for his own mail.
#[test]
fn the_side_column_lists_folders_then_labels_and_rows_carry_star_and_count() {
    let _budget = TestBudget::new();
    let mut fixture = recorded();
    let boxes = serde_json::json!(["Mailbox/get", {"accountId": "f", "list": [
        {"id": "a", "name": "Inbox", "role": "inbox", "unreadThreads": 150, "totalThreads": 382},
        {"id": "d", "name": "Drafts", "role": "drafts", "unreadThreads": 0, "totalThreads": 11},
        {"id": "s", "name": "Spam", "role": "junk", "unreadThreads": 147, "totalThreads": 200},
        {"id": "t", "name": "Sent Items", "role": "sent", "unreadThreads": 4, "totalThreads": 90},
        {"id": "m", "name": "Marketing", "role": null, "unreadThreads": 297, "totalThreads": 300},
        {"id": "k", "name": "Klar specimens", "unreadThreads": 37, "totalThreads": 40}]}, "m"]);
    let answers = fixture["inbox"]["methodResponses"]
        .as_array_mut()
        .expect("answers");
    answers.push(boxes);
    // the latest conversation's oldest mail starred, its sender his own address
    let all = answers
        .iter_mut()
        .find(|a| a[2] == "all")
        .expect("every mail");
    let brq = all[1]["list"]
        .as_array_mut()
        .expect("its list")
        .iter_mut()
        .find(|m| m["threadId"] == "brq")
        .expect("brq");
    brq["keywords"]["$flagged"] = true.into();
    let shown = answers
        .iter_mut()
        .find(|a| a[2] == "e")
        .expect("the shown mails");
    let first = shown[1]["list"]
        .as_array_mut()
        .expect("its list")
        .iter_mut()
        .find(|m| m["threadId"] == "brq")
        .expect("brq");
    first["from"][0]["email"] = OWN.into();
    let inbox = Asked::of(None, None, None);
    let (read, _) = replayed_from(fixture, &inbox);
    let tree = tree(
        OWN,
        &[OWN.into(), "support@klar.im".into()],
        &inbox,
        &read,
        0,
    );
    let page: Value = serde_json::from_str(&tree).expect("a tree");
    let shell = &page["elements"]["page"]["props"];
    let folders: Vec<(String, u64, String)> =
        (shell["folders"].as_array().expect("folders").iter())
            .map(|f| {
                (
                    f["name"].as_str().unwrap_or_default().into(),
                    f["count"].as_u64().unwrap_or(0),
                    f["href"].as_str().unwrap_or_default().into(),
                )
            })
            .collect();
    let name = |n: &str, c: u64, h: &str| (n.to_string(), c, h.to_string());
    assert_eq!(
        folders,
        [
            name("Inbox", 150, "/mail"),
            name("Starred", 0, "/mail?box=starred"),
            name("Sent", 0, "/mail?box=sent"),
            name("Drafts", 11, "/mail?box=drafts"),
            name("Spam", 200, "/mail?box=spam"),
            name("Trash", 0, "/mail?box=trash"),
            name("All mail", 0, "/mail?box=all"),
            name("Klar specimens", 37, "/mail?box=label%3Ak"),
            name("Marketing", 297, "/mail?box=label%3Am"),
        ]
    );
    assert_eq!(shell["folders"][0]["current"], true);
    let row = &page["elements"]["list-0"]["props"];
    assert_eq!(
        (row["from"].as_str(), row["starred"].as_bool()),
        (Some("me"), Some(true))
    );
    assert!(row["count"].as_u64().is_some_and(|n| n >= 1), "{row}");

    let html = dash_surface::screen::page("Mail", &tree).expect("the page");
    for part in [
        r#"<details class="mail-account dropdown-menu">"#,
        r#"<a class="dropdown-menu-item" role="menuitem" id="page-account-1" href="/mail?account=support@klar.im">"#,
        r#"<a class="button mail-compose" href="/mail/compose">"#,
        r#"<a class="mail-folder busy" href="/mail" aria-current="page">"#,
        r#"<span class="name">Klar specimens</span><span class="count">37</span>"#,
        r#"<form class="form mail-search" role="search" method="get" action="/mail">"#,
        r#"<span role="img" class="star on" aria-label="Starred">"#,
        r#"<div class="mail-bar" role="toolbar" aria-label="List" id="bar">"#,
    ] {
        assert!(html.contains(part), "{part} in {html}");
    }
    let text = dash_surface::text(&tree).expect("the text");
    for line in [
        "Account: nicolas@klar.im\n",
        "Switch to: support@klar.im /mail?account=support@klar.im\n",
        "Compose: /mail/compose\n",
        "Inbox\t150\t/mail\tshown\n",
        "Marketing\t297\t/mail?box=label%3Am\t\n",
        "Search mail: GET /mail?q=<words>\n",
        "1–50 of 359, Older /mail?page=2\n",
        "\tstarred\t/mail/t/brq",
    ] {
        assert!(text.contains(line), "{line:?} in {text}");
    }
}

/// /code-review of rust-mail-parity: the side column's read failing never takes the list down, and a folder whose role the
/// column does not name (archive) is listed with the labels, reachable.
#[test]
fn a_failed_folder_read_leaves_the_list_and_an_archive_is_reachable() {
    let _budget = TestBudget::new();
    let mut fixture = recorded();
    let answers = fixture["inbox"]["methodResponses"]
        .as_array_mut()
        .expect("answers");
    answers.push(serde_json::json!(["error", {"type": "unknownMethod"}, "m"]));
    let inbox = Asked::of(None, None, None);
    let (read, _) = replayed_from(fixture, &inbox);
    let read = read.expect("the list, its folders' read failed");
    assert_eq!((read.rows.len() as u64, read.boxes.len()), (PAGE, 0));

    let archive = dash_mail::Mailbox {
        id: "z".into(),
        name: "Archive".into(),
        role: "archive".into(),
        unread: 2,
        total: 9,
    };
    let links = dash_mail::folder_links(OWN, &[archive], "inbox");
    let last = links.last().expect("a line");
    assert_eq!(
        (last["name"].as_str(), last["href"].as_str()),
        (Some("Archive"), Some("/mail?box=label%3Az"))
    );
}
