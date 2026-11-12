//! A mail draft's send, the part read from the draft alone (task outbound-gate, `dash mail send <id>`): whether it may go
//! now, who it goes to, its subject and the thread it answers. The draft is the older app's export (`dash model draft <id>`),
//! whose facts carry its rules (`sendable`, `held-until`, `attach-missing`). Ported from klar-mail's send_locked, recipient,
//! cc and build; the binary brings the contact cards, the gate, the mail server and the logs.

use serde_json::Value as Json;

/// The one mailbox a draft's send leaves from.
pub const FROM: &str = "nicolas@klar.im";
/// A line only he can fill (his birth date, his address): lower case on purpose, so the draft still reaches him.
const UNFILLED: &str = "[à compléter]";

fn said<'a>(d: &'a Json, field: &str) -> &'a str {
    d[field].as_str().unwrap_or_default()
}

/// Why draft `d` does not go now, before any address or text is read: not in the outbox, held for its send-on day, not a
/// mail, or from another mailbox, which is his to send (comet-elodie-cv-en, meant for another address, left from here on
/// 2026-10-15 after he had sent it himself; @nha, 2026-10-15: « the source of truth needs to be the EDN »).
pub fn refused(d: &Json) -> Option<String> {
    let id = said(d, "id");
    if d["sendable"] != true {
        let status = said(d, "status");
        return Some(format!(
            "{id} is {status}, not in the outbox (private/dashboard/bin/dash outbox)"
        ));
    }
    if let Some(day) = d["held-until"].as_str() {
        return Some(format!("{id} goes out on {day} (:send-on), not before"));
    }
    if said(d, "channel") != "email" {
        let channel = said(d, "channel");
        return Some(format!("{id} is a {channel} draft, not a mail"));
    }
    match d["from"].as_str() {
        Some(from) if from != FROM => Some(format!(
            "{id} goes from {from} (:from), not {FROM}: his to send"
        )),
        _ => None,
    }
}

/// Why its text does not go: a line still his to fill.
pub fn unfilled(d: &Json, text: &str) -> Option<String> {
    text.contains(UNFILLED).then(|| {
        format!(
            "{} still has « {UNFILLED} »: his to fill on the draft page",
            said(d, "id")
        )
    })
}

/// The app's verdict on a file the draft should carry, `Err` when it gave none: an app older than this check is never a
/// pass. `Ok(Some(why))`: a file is missing.
pub fn attach_missing(d: &Json) -> Result<Option<String>, String> {
    match d.get("attach-missing") {
        None => Err(format!(
            "{}: the app gave no attach-missing verdict (older than this command?)",
            said(d, "id")
        )),
        Some(v) => Ok(v.as_str().map(String::from)),
    }
}

/// Each contact card's name, lower case, with its first address: `dash model contacts`'s list.
pub fn cards(contacts: &Json) -> Vec<(String, String)> {
    let first = |c: &Json| match &c["email"] {
        Json::Array(list) => list.iter().find_map(address_of),
        one => address_of(one),
    };
    (contacts.as_array().into_iter().flatten())
        .filter_map(|c| Some((c["name"].as_str()?.to_lowercase(), first(c)?)))
        .collect()
}

fn address_of(v: &Json) -> Option<String> {
    v.as_str().filter(|a| a.contains('@')).map(String::from)
}

/// The addresses a text holds, in order: someone, one « @ », a dotted host.
fn addresses(text: &str) -> Vec<String> {
    let cut = |c: char| c.is_whitespace() || "<>(),;:=«»\"'[]".contains(c);
    text.split(cut)
        .map(|t| t.trim_matches('.'))
        .filter(|t| {
            let Some((user, host)) = t.split_once('@') else {
                return false;
            };
            !user.is_empty() && host.contains('.') && !host.contains('@') && !host.contains("..")
        })
        .map(String::from)
        .collect()
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `text` holds `word` whole: « William » is not « Williams ».
fn holds_whole(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(word_char) && !after.is_some_and(word_char)
    })
}

/// The cards' addresses of the people `text` names.
fn named(text: &str, cards: &[(String, String)]) -> Vec<String> {
    let text = text.to_lowercase();
    (cards.iter())
        .filter(|(name, _)| !name.is_empty() && holds_whole(&text, name))
        .map(|(_, address)| address.clone())
        .collect()
}

/// Who a draft's mail goes to.
#[derive(Clone, Debug, PartialEq)]
pub struct To {
    pub to: String,
    pub cc: Vec<String>,
}

/// What follows each « cc » of `s`, in any case, a whole word, then a colon or a space of any kind: « Cc: x@y », « (cc x@y) »,
/// « ,cc x@y » and « cc » before a new line all name a copy, or that address became the To (/code-review, twice).
fn after_cc(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(2))
        .filter(|&i| b[i..i + 2].eq_ignore_ascii_case(b"cc"))
        .filter(|&i| b[i + 2] == b':' || b[i + 2].is_ascii_whitespace())
        // « c » is one byte, so `i` and `i + 2` are character boundaries
        .filter(|&i| !s[..i].chars().next_back().is_some_and(word_char))
        .map(|i| s[i + 2..].trim_start_matches(|c: char| c == ':' || c.is_whitespace()))
        .collect()
}

/// The names after each « cc » of its `to`: « Elodie COLIN (Comet), cc Camille » names Camille.
fn cc_names(to: &str) -> Vec<&str> {
    after_cc(to)
        .into_iter()
        .map(|rest| rest.split(',').next().unwrap_or_default().trim())
        .filter(|n| !n.is_empty())
        .collect()
}

/// The draft's addresses. To: the first address in its `to`, else in its `where`, neither his own nor a copy's, else the
/// card of the person `to` names before any comma. Copies: each « cc x@y » of `where`, and the card of each « cc <name> » of
/// `to`. A named copy with no card stops the send: comet-elodie-cv-en left without its copy on 2026-10-15, the address only
/// in another card's notes.
pub fn addressed(d: &Json, cards: &[(String, String)]) -> Result<To, String> {
    let (id, to, place) = (said(d, "id"), said(d, "to"), said(d, "where"));
    let names = cc_names(to);
    let missing: Vec<&str> = (names.iter().copied())
        .filter(|n| named(n, cards).is_empty())
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "no contact card with an :email for cc {} ({id})",
            missing.join(", ")
        ));
    }
    let mut cc: Vec<String> = (after_cc(place).into_iter())
        .filter_map(|rest| {
            addresses(rest.split_whitespace().next()?)
                .into_iter()
                .next()
        })
        .collect();
    cc.extend(names.iter().flat_map(|n| named(n, cards)));
    let theirs = |a: &String| a != FROM && !cc.contains(a);
    let mut written = addresses(to);
    if written.is_empty() {
        written = addresses(place);
    }
    let first = (written.into_iter().find(theirs)).or_else(|| {
        let before_comma = to.split(',').next().unwrap_or_default();
        named(before_comma, cards).into_iter().find(theirs)
    });
    match first {
        Some(to) => Ok(To { to, cc }),
        None => Err(format!(
            "no address for {id}: not in :to, :where, nor on the contact card of « {to} »"
        )),
    }
}

/// The thread its `where` names: « sujet « … » » or « fil « … » ».
pub fn thread_named(place: &str) -> Option<&str> {
    ["sujet « ", "fil « "].into_iter().find_map(|open| {
        let rest = &place[place.find(open)? + open.len()..];
        let title = &rest[..rest.find(" »")?];
        (!title.is_empty() && !title.contains('»')).then_some(title)
    })
}

fn is_reply(subject: &str) -> bool {
    let s = subject.trim_start();
    s.get(..2).is_some_and(|re| re.eq_ignore_ascii_case("re"))
        && s[2..].trim_start().starts_with(':')
}

/// `subject` as a reply's: its own when it already opens on « Re: ».
pub fn re(subject: &str) -> String {
    match is_reply(subject) {
        true => subject.to_string(),
        false => format!("Re: {subject}"),
    }
}

/// `subject` less its « Re: », what the mail it answers is titled.
pub fn base(subject: &str) -> &str {
    match is_reply(subject) {
        true => subject.trim_start()[2..].trim_start()[1..].trim_start(),
        false => subject,
    }
}

/// The mail's subject. A reply's comes from what it answers, never its own, which would skip its threading: the subject of
/// the mail its `reply-to` names (`answered`), else the thread its `where` names, each as a « Re: ». A first mail's is its
/// own. None of them: « Nicolas Ha ».
pub fn subject(d: &Json, answered: Option<&str>) -> String {
    let some = |s: &str| Some(s.trim()).filter(|s| !s.is_empty()).map(String::from);
    let found = match (answered, thread_named(said(d, "where"))) {
        (Some(theirs), _) => some(theirs).map(|t| re(&t)),
        (None, Some(title)) => Some(re(title)),
        (None, None) => some(said(d, "subject")),
    };
    found.unwrap_or_else(|| "Nicolas Ha".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;
    use serde_json::json;

    fn cards_() -> Vec<(String, String)> {
        cards(&json!([
            {"name": "Elodie Colin", "email": ["elodie.c@comet.co", "e@old.example"]},
            {"name": "Camille", "email": "camille@comet.co"},
            {"name": "William", "email": "will@example.org"},
            {"name": "No Mail"},
        ]))
    }

    fn draft() -> Json {
        json!({"id": "x", "status": "approved", "sendable": true, "channel": "email", "held-until": null,
               "attach-missing": null, "to": "ada@example.org", "where": "", "body": "Bonjour"})
    }

    /// The draft's own facts refuse a send before anything is read or asked: its status, its day, its channel, its mailbox.
    #[test]
    fn a_draft_not_ready_is_refused_with_why() {
        let _budget = TestBudget::new();
        assert_eq!(refused(&draft()), None);
        let with = |k: &str, v: Json| {
            let mut d = draft();
            d[k] = v;
            refused(&d).unwrap_or_default()
        };
        assert_eq!(
            with("sendable", json!(false)),
            "x is approved, not in the outbox (private/dashboard/bin/dash outbox)"
        );
        assert_eq!(
            with("held-until", json!("2026-11-20")),
            "x goes out on 2026-11-20 (:send-on), not before"
        );
        assert_eq!(
            with("channel", json!("linkedin")),
            "x is a linkedin draft, not a mail"
        );
        assert_eq!(
            with("from", json!("other@example.org")),
            "x goes from other@example.org (:from), not nicolas@klar.im: his to send"
        );
        assert_eq!(with("from", json!(FROM)), "");
        assert!(unfilled(&draft(), "Né le [à compléter]").is_some());
        assert_eq!(unfilled(&draft(), "Bonjour"), None);
    }

    /// No verdict on its files is never a pass.
    #[test]
    fn a_missing_file_verdict_is_read_and_its_absence_refused() {
        let _budget = TestBudget::new();
        assert_eq!(attach_missing(&draft()), Ok(None));
        let mut d = draft();
        d["attach-missing"] = json!("names a CV and carries none");
        assert_eq!(
            attach_missing(&d),
            Ok(Some("names a CV and carries none".into()))
        );
        d.as_object_mut().expect("a map").remove("attach-missing");
        assert!(attach_missing(&d).is_err());
    }

    /// klar-mail's recipient and cc: an address written wins, else the card of the person named before the comma; a copy by
    /// address in `where` or by name in `to`; his own address and a copy's are never the receiver.
    #[test]
    fn the_addresses_come_from_the_draft_then_the_cards() {
        let _budget = TestBudget::new();
        let of = |to: &str, place: &str| {
            addressed(&json!({"id": "x", "to": to, "where": place}), &cards_())
        };
        let to = |a: &str, cc: &[&str]| {
            Ok(To {
                to: a.into(),
                cc: cc.iter().map(|c| c.to_string()).collect(),
            })
        };
        assert_eq!(of("Ada <ada@example.org>", ""), to("ada@example.org", &[]));
        assert_eq!(
            of("Elodie COLIN (Comet), cc Camille", "mail"),
            to("elodie.c@comet.co", &["camille@comet.co"])
        );
        assert_eq!(
            of("Bob", "mail à bob@example.org, cc carl@example.org."),
            to("bob@example.org", &["carl@example.org"])
        );
        assert_eq!(
            of(
                "Elodie (Comet)",
                "mail (cc camille@comet.co) à elodie@comet.co"
            ),
            to("elodie@comet.co", &["camille@comet.co"])
        );
        assert_eq!(
            of(
                "Bob",
                "bob@example.org,cc carl@example.org; acc zed@example.org"
            ),
            to("bob@example.org", &["carl@example.org"])
        );
        // « Cc: », any case, a colon, a tab or a new line after it: still a copy, never the To
        assert_eq!(
            of("Elodie (Comet)", "Cc: camille@comet.co, à elodie@comet.co"),
            to("elodie@comet.co", &["camille@comet.co"])
        );
        assert_eq!(
            of(
                "Bob",
                "CC\tcarl@example.org\ncc\nzed@example.org bob@example.org"
            ),
            to("bob@example.org", &["carl@example.org", "zed@example.org"])
        );
        assert_eq!(
            of("Elodie COLIN (Comet), Cc: Camille", "mail"),
            to("elodie.c@comet.co", &["camille@comet.co"])
        );
        assert_eq!(
            of("Bob", "de nicolas@klar.im à bob@example.org"),
            to("bob@example.org", &[])
        );
        assert_eq!(
            of("Williams", ""),
            Err(
                "no address for x: not in :to, :where, nor on the contact card of « Williams »"
                    .into()
            )
        );
        assert_eq!(
            of("Ada <ada@example.org>, cc Zed", ""),
            Err("no contact card with an :email for cc Zed (x)".into())
        );
        assert!(of("No Mail", "").is_err());
    }

    /// A reply's subject comes from what it answers; a first mail's is its own, else his name.
    #[test]
    fn a_reply_takes_no_subject_of_its_own() {
        let _budget = TestBudget::new();
        assert_eq!(
            thread_named("réponse, sujet « Mission Kafka » (mail)"),
            Some("Mission Kafka")
        );
        assert_eq!(thread_named("fil « Re: CV »"), Some("Re: CV"));
        assert_eq!(thread_named("son fil LinkedIn"), None);
        let first = json!({"subject": "Le PDF", "where": "mail"});
        assert_eq!(subject(&first, None), "Le PDF");
        assert_eq!(subject(&json!({"where": "mail"}), None), "Nicolas Ha");
        let threaded = json!({"subject": "x", "where": "sujet « y »"});
        assert_eq!(subject(&threaded, None), "Re: y");
        assert_eq!(subject(&threaded, Some("Re: z")), "Re: z");
        assert_eq!(subject(&first, Some("z")), "Re: z");
        assert_eq!(subject(&first, Some(" ")), "Nicolas Ha");
        assert_eq!(
            (re("CV"), re("RE : CV"), re("re: CV")),
            ("Re: CV".into(), "RE : CV".into(), "re: CV".into())
        );
        assert_eq!(
            (base("Re: CV"), base("RE :  CV"), base("Reply")),
            ("CV", "CV", "Reply")
        );
    }
}
