//! Done-when 2 of rust-mail-parity, its design-system half: each part of /mail, /mail/t/<thread> and /mail/compose named
//! by the design-system component that draws it, and found in the page's HTML as dash serve draws it (dash_surface's screen,
//! the one renderer). Run with `--no-capture` to read the table: element, component, what the HTML holds of it, how many.
use dash_mail::{Asked, OWN, compose_tree, list, thread, thread_tree, tree};
use dash_meter::memory::TestBudget;
use serde_json::{Value, json};

/// The inbox's recorded answers (inbox.json) with the account's folders and labels, as the server gives them.
fn inbox_html() -> String {
    let mut fixture: Value = serde_json::from_str(include_str!("inbox.json")).expect("the fixture");
    let boxes = json!(["Mailbox/get", {"accountId": "f", "list": [
        {"id": "a", "name": "Inbox", "role": "inbox", "unreadThreads": 150, "totalThreads": 382},
        {"id": "d", "name": "Drafts", "role": "drafts", "unreadThreads": 0, "totalThreads": 11},
        {"id": "m", "name": "Marketing", "unreadThreads": 297, "totalThreads": 300}]}, "m"]);
    let answers = fixture["inbox"]["methodResponses"]
        .as_array_mut()
        .expect("answers");
    answers.push(boxes);
    let http = |url: &str, body: Option<&str>| -> Result<String, String> {
        let asked: Option<Value> = body.map(|b| serde_json::from_str(b).expect("JSON"));
        Ok(match (url, asked) {
            (dash_mail::SESSION, None) => fixture["session"].to_string(),
            (_, Some(b)) if b["methodCalls"][0][0] == "Mailbox/query" => {
                fixture["mailbox"].to_string()
            }
            _ => fixture["inbox"].to_string(),
        })
    };
    let inbox = Asked::of(None, None, None);
    let accounts = [OWN.to_string(), "support@klar.im".into()];
    let page = tree(OWN, &accounts, &inbox, &list(&http, &inbox), 0);
    dash_surface::screen::page("Mail", &page).expect("the list's HTML")
}

fn thread_html() -> String {
    let fixture: Value = serde_json::from_str(include_str!("thread.json")).expect("the fixture");
    let http = |url: &str, body: Option<&str>| -> Result<String, String> {
        Ok(match (url, body) {
            (dash_mail::SESSION, None) => fixture["session"].to_string(),
            _ => fixture["thread"].to_string(),
        })
    };
    let read = thread(&http, "brq");
    let page = thread_tree(OWN, &[OWN.into()], "brq", &read, None);
    dash_surface::screen::page("Mail", &page).expect("the thread's HTML")
}

fn compose_html() -> String {
    let page = compose_tree(OWN, &[OWN.into()], &[], None, "p1");
    dash_surface::screen::page("Mail", &page).expect("Compose's HTML")
}

#[test]
fn every_part_of_mail_is_a_design_system_component() {
    let _budget = TestBudget::new();
    let (list, thread, compose) = (inbox_html(), thread_html(), compose_html());
    // element of :7799's mail, its page, the design-system component (rust/crates/surface/src/components.rs or documents.rs),
    // the mark of it in the HTML, and how many the page must hold at least
    let parts: [(&str, &str, &str, &str, usize); 17] = [
        (
            "Account switcher",
            "list",
            "MailShell + DropdownMenuItem",
            r#"class="mail-account dropdown-menu""#,
            1,
        ),
        (
            "Account switcher entries",
            "list",
            "DropdownMenuItem",
            r#"class="dropdown-menu-item""#,
            2,
        ),
        (
            "Compose button",
            "list",
            "MailShell",
            r#"class="button mail-compose""#,
            1,
        ),
        (
            "Folders and labels",
            "list",
            "MailShell (Folder)",
            r#"class="mail-folder"#,
            8,
        ),
        (
            "A label, its count",
            "list",
            "MailShell (Folder kind label)",
            r#"<span class="name">Marketing</span><span class="count">297</span>"#,
            1,
        ),
        (
            "Search",
            "list",
            "MailShell + Input",
            r#"class="form mail-search" role="search""#,
            1,
        ),
        (
            "List bar: refresh, range, pager",
            "list",
            "MailBar",
            r#"class="mail-bar" role="toolbar" aria-label="List""#,
            1,
        ),
        (
            "Thread list rows",
            "list",
            "ThreadRow",
            r#"class="thread-row"#,
            50,
        ),
        ("Row star", "list", "ThreadRow", r#"class="star"#, 50),
        (
            "Reading pane head",
            "thread",
            "MailHead + Badge",
            r#"class="mail-head""#,
            1,
        ),
        (
            "Reading pane mails",
            "thread",
            "MailFold (Fold)",
            r#"details class="fold""#,
            3,
        ),
        (
            "Sender avatar",
            "thread",
            "MailFold + Avatar",
            r#"class="avatar""#,
            3,
        ),
        (
            "Mail body",
            "thread",
            "MailFold + Frame",
            r#"class="frame"#,
            1,
        ),
        (
            "Reply box",
            "thread",
            "Composer (editor)",
            r#"class="editor""#,
            1,
        ),
        (
            "Reply formatting bar",
            "thread",
            "Composer + ToolbarButton",
            r#"class="toolbar-button editor-format""#,
            6,
        ),
        (
            "Compose From to Subject",
            "compose",
            "Composer (Field)",
            r#"class="composer-field""#,
            5,
        ),
        (
            "Compose formatting bar",
            "compose",
            "Composer + ToolbarButton",
            r#"class="toolbar-button editor-format""#,
            6,
        ),
    ];
    let mut missing = vec![];
    println!("element\tpage\tcomponent\tmark in the HTML\tfound");
    for (element, page, component, mark, least) in parts {
        let html = match page {
            "list" => &list,
            "thread" => &thread,
            _ => &compose,
        };
        let found = html.matches(mark).count();
        println!("{element}\t{page}\t{component}\t{mark}\t{found}");
        if found < least {
            missing.push(format!("{element}: {found} of {mark}, {least} at least"));
        }
    }
    for (mark, title) in [
        ("bold", "Bold"),
        ("italic", "Italic"),
        ("link", "Insert link"),
        ("ul", "Bulleted list"),
        ("ol", "Numbered list"),
        ("quote", "Quote"),
    ] {
        let button = format!(r#"title="{title}" aria-label="{title}" data-format="{mark}""#);
        println!(
            "Formatting button {title}\tcompose\tToolbarButton\t{button}\t{}",
            compose.matches(&button).count()
        );
        if !compose.contains(&button) {
            missing.push(button);
        }
    }
    assert!(missing.is_empty(), "{missing:#?}");
}
