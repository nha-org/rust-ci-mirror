//! A mail draft's send, the part read from the draft alone (task outbound-gate, `dash mail send <id>`): whether it may go
//! now, who it goes to, its subject and the thread it answers. The draft is the older app's export (`dash model draft <id>`),
//! whose facts carry its rules (`sendable`, `held-until`, `attach-missing`). Ported from klar-mail's send_locked, recipient,
//! cc and build; the binary brings the contact cards, the gate, the mail server and the logs.

use serde_json::Value as Json;

/// The one mailbox a draft's send leaves from.
pub const FROM: &str = "nicolas@klar.im";
/// His own addresses: a mail between two of them names nobody (`DraftContext`'s @his), and none is ever his own copy.
pub const HIS: [&str; 3] = [
    "nicolas@klar.im",
    "nicolas.ha.pro@gmail.com",
    "maltsevaha@gmail.com",
];
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

/// The MIME type a file attached to a mail goes as, by its extension.
pub fn mime(file: &str) -> &'static str {
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

/// One recipient as written, validated whole, never extracted (review of PR 1780: three rounds of a parser that cut an address
/// out of text sent to « brien@x.com » for « o'brien@x.com », to « old@x.com » for « "old@x.com" <new@y.com> »). `entry`,
/// trimmed of whitespace alone, is exactly `addr`, or a display name then `addr` between angle brackets, the name never read. `addr` is a dot-atom local
/// part (RFC 5322's atext, `'` `=` `*` and a backtick included; a letter beyond ASCII as RFC 6531 allows; no quoted local
/// part) @ two or more domain labels. None for anything else: nothing is trimmed, cut or taken from a part of it.
pub(crate) fn recipient(entry: &str) -> Option<&str> {
    let entry = entry.trim();
    let addr = match entry.strip_suffix('>') {
        Some(rest) => {
            let (name, addr) = rest.rsplit_once('<')?;
            (!name.contains(['<', '>'])).then_some(addr)?
        }
        None => entry,
    };
    let (local, domain) = addr.split_once('@')?;
    let atext = |c: char| c.is_alphanumeric() || "!#$%&'*+-/=?^_`{|}~".contains(c);
    let atoms = |s: &str| s.split('.').all(|a| !a.is_empty() && a.chars().all(atext));
    let label = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && l.chars().all(|c| c.is_alphanumeric() || c == '-')
            && !l.starts_with('-')
            && !l.ends_with('-')
    };
    let domain_ok = domain.contains('.') && domain.split('.').all(label);
    (atoms(local) && domain_ok).then_some(addr)
}

/// The recipients a `to` writes, split on `,` and `;` alone: each entry `recipient` validates, the others none. The latest
/// pass keys a draft's parties by it: an entry that fails is no party, never a guessed one.
pub(crate) fn addresses(text: &str) -> Vec<String> {
    (text.split([',', ';']))
        // a copy is one of its parties too, its « cc » mark read as `addressed` reads it (/code-review)
        .map(|e| copy_of(e.trim()).unwrap_or(e))
        .filter_map(recipient)
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

/// The cards' addresses of the people `text` names, each through the same validator as a written one: a card holding junk
/// refuses the send, naming the card (review of PR 1780).
pub(crate) fn named(text: &str, cards: &[(String, String)]) -> Result<Vec<String>, String> {
    let text = text.to_lowercase();
    (cards.iter())
        .filter(|(name, _)| !name.is_empty() && holds_whole(&text, name))
        .map(|(name, address)| {
            recipient(address).map(String::from).ok_or(format!(
                "the contact card of « {name} » holds « {address} », no address"
            ))
        })
        .collect()
}

/// Who a draft's mail goes to.
#[derive(Clone, Debug, PartialEq)]
pub struct To {
    pub to: String,
    pub cc: Vec<String>,
}

/// An entry's copy mark: « cc », any case, then a colon or a space, and what follows it (/code-review, twice).
pub(crate) fn copy_of(entry: &str) -> Option<&str> {
    let head = entry.get(..2).filter(|h| h.eq_ignore_ascii_case("cc"))?;
    let rest = &entry[head.len()..];
    rest.starts_with(|c: char| c == ':' || c.is_whitespace())
        .then(|| rest.trim_start_matches(|c: char| c == ':' || c.is_whitespace()))
}

/// The draft's addresses, read from its `to` alone, split on `,` and `;` (review of PR 1780: the send path validates, it never
/// extracts). Each entry is an address (`recipient`: one written alone or after a name between angle brackets), a copy (« cc » before one), or a name, which
/// its contact card answers (structured data, never cut out of text). An entry holding « @ » that is no address refuses the
/// send, named: nothing is taken from a part of it. To: the first address written, neither his own nor a copy's, else the
/// card of the first name. A named copy with no card stops the send: comet-elodie-cv-en left without its copy on 2026-10-15.
/// `where` is free text: never read for an address.
pub fn addressed(d: &Json, cards: &[(String, String)]) -> Result<To, String> {
    let (id, to) = (said(d, "id"), said(d, "to"));
    let (mut tos, mut names, mut cc) = (vec![], vec![], vec![]);
    for entry in to
        .split([',', ';'])
        .map(str::trim)
        .filter(|e| !e.is_empty())
    {
        let (copy, rest) = copy_of(entry).map_or((false, entry), |rest| (true, rest));
        let written = match rest.contains('@') {
            true => Some(recipient(rest).ok_or(format!(
                "« {entry} » is no address: write it you@example.org or Name <you@example.org> ({id})"
            ))?),
            false => None,
        };
        match (copy, written) {
            (true, Some(a)) => cc.push(a.to_string()),
            // one card a name: more than one refuses, named, never copies all nor picks one (review of PR 1780)
            (true, None) => match named(rest, cards).map_err(|e| format!("{e} ({id})"))?[..] {
                [] => {
                    return Err(format!(
                        "no contact card with an :email for cc {rest} ({id})"
                    ));
                }
                [ref one] => cc.push(one.clone()),
                ref many => {
                    return Err(format!(
                        "cc {rest} names more than one contact card ({}): write the address ({id})",
                        many.join(", ")
                    ));
                }
            },
            (false, Some(a)) => tos.push(a.to_string()),
            (false, None) => names.push(rest),
        }
    }
    // each copy once, his own addresses never among them (review of PR 1780)
    let his = |a: &String| HIS.iter().any(|h| a.eq_ignore_ascii_case(h));
    cc.retain(|a| !his(a));
    let mut seen = std::collections::BTreeSet::new();
    cc.retain(|a| seen.insert(a.to_lowercase()));
    // without case, as the dedup: « Bob@x.org » and « cc bob@x.org » are one person, mailed once (review of PR 1780)
    let theirs = |a: &String| !his(a) && !seen.contains(&a.to_lowercase());
    let first = match tos.into_iter().find(theirs) {
        Some(a) => Some(a),
        None => {
            let name = names.first().copied().unwrap_or_default();
            let found = named(name, cards).map_err(|e| format!("{e} ({id})"))?;
            found.into_iter().find(theirs)
        }
    };
    match first {
        Some(to) => Ok(To { to, cc }),
        None => Err(format!(
            "no address for {id}: not in :to, nor on the contact card of « {to} »"
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
    use dash_sdk::{dash_meter, dash_test};
    use serde_json::json;

    /// Review of PR 1780: every case three reviews raised comes out exactly as written or refused, never a third address; a
    /// display name is never read; a list of two splits.
    #[dash_test]
    fn a_recipient_is_validated_whole_never_extracted() {
        for (entry, want) in [
            ("o'brien@x.com", Some("o'brien@x.com")),
            ("a=b@x.com", Some("a=b@x.com")),
            ("\"old@x.com\" <new@y.com>", Some("new@y.com")),
            ("\"x@evil.com\"@good.com", None),
            ("*ops@x.com", Some("*ops@x.com")),
            ("`team", None),
            ("a@b.org@c.com", None),
            ("a@corp.fr!evil.com", None),
            ("medium.com/@jane.doe", Some("medium.com/@jane.doe")),
            ("x.com/u@y.org", Some("x.com/u@y.org")),
            ("mailto:a@b.c", None),
            ("a@b.c.", None),
            ("josé.martin@x.fr", Some("josé.martin@x.fr")),
            ("a&b@corp.com", Some("a&b@corp.com")),
            ("sam@example.com!", None),
            ("deciding on <x@y.org>.", None),
            ("Sam <sam@example.com>", Some("sam@example.com")),
        ] {
            assert_eq!(recipient(entry), want, "{entry}");
        }
        assert_eq!(
            addresses("Sam <sam@example.com>; al@x.org"),
            ["sam@example.com", "al@x.org"]
        );
    }

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
    #[dash_test]
    fn a_draft_not_ready_is_refused_with_why() {
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
    #[dash_test]
    fn a_missing_file_verdict_is_read_and_its_absence_refused() {
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

    /// klar-mail's recipient and cc, from `to` alone: an address written wins, else the card of the person named first; a copy
    /// by address or by name; his own address and a copy's are never the receiver; `where` is never read for one, and an entry
    /// holding « @ » that is no address refuses the send (review of PR 1780).
    #[dash_test]
    fn the_addresses_come_from_the_draft_then_the_cards() {
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
        // `where` is free text: no address is ever read from it
        assert!(of("Bob", "mail à bob@example.org, cc carl@example.org.").is_err());
        assert_eq!(
            of(
                "bob@example.org, cc carl@example.org; acc zed@example.org",
                ""
            ),
            Err("« acc zed@example.org » is no address: write it you@example.org or Name <you@example.org> (x)".into())
        );
        assert_eq!(
            of(
                "bob@example.org, cc carl@example.org; Zed <zed@example.org>",
                ""
            ),
            to("bob@example.org", &["carl@example.org"])
        );
        // « Cc: », any case, a colon or a tab after it: still a copy, never the To
        assert_eq!(
            of("Cc: camille@comet.co, Elodie <elodie@comet.co>", ""),
            to("elodie@comet.co", &["camille@comet.co"])
        );
        assert_eq!(
            of("CC\tcarl@example.org; bob@example.org", ""),
            to("bob@example.org", &["carl@example.org"])
        );
        assert_eq!(
            of("Elodie COLIN (Comet), Cc: Camille", "mail"),
            to("elodie.c@comet.co", &["camille@comet.co"])
        );
        assert_eq!(
            of("nicolas@klar.im, bob@example.org", ""),
            to("bob@example.org", &[])
        );
        // nothing is cut out of an entry: one holding « @ » that is no address refuses, named
        assert_eq!(
            of("Sam B. (Example grants), mailto:sam@example.com", ""),
            Err(
                "« mailto:sam@example.com » is no address: write it you@example.org or Name <you@example.org> (x)"
                    .into()
            )
        );
        assert_eq!(
            of("Williams", ""),
            Err("no address for x: not in :to, nor on the contact card of « Williams »".into())
        );
        assert_eq!(
            of("Ada <ada@example.org>, cc Zed", ""),
            Err("no contact card with an :email for cc Zed (x)".into())
        );
        assert!(of("No Mail", "").is_err());
        // a card's address passes the same validator: a card holding junk refuses the send, naming the card
        let junk = cards(&json!([{"name": "Junk Co", "email": "mailto:j@x.org"}]));
        let by = |to: &str| addressed(&json!({"id": "x", "to": to, "where": ""}), &junk);
        let said = "the contact card of « junk co » holds « mailto:j@x.org », no address (x)";
        assert_eq!(by("Junk Co"), Err(said.into()));
        assert_eq!(by("bob@example.org, cc Junk Co"), Err(said.into()));
        // a cc name two cards answer refuses, naming them; copies come once; his own address is never a copy
        let two = cards(&json!([
            {"name": "Ben", "email": "ben@x.org"},
            {"name": "Ben", "email": "ben@y.org"},
            {"name": "Carl", "email": "carl@example.org"},
        ]));
        let with = |to: &str| addressed(&json!({"id": "x", "to": to, "where": ""}), &two);
        assert_eq!(
            with("bob@example.org, cc Ben"),
            Err("cc Ben names more than one contact card (ben@x.org, ben@y.org): write the address (x)".into())
        );
        assert_eq!(
            with("Bob@example.org, ann@example.org, cc bob@example.org"),
            Ok(To {
                to: "ann@example.org".into(),
                cc: vec!["bob@example.org".into()]
            }),
            "a copy is never also the receiver, whatever its case"
        );
        assert_eq!(
            with("bob@example.org, cc Carl, cc carl@example.org, cc nicolas@klar.im"),
            Ok(To {
                to: "bob@example.org".into(),
                cc: vec!["carl@example.org".into()]
            })
        );
    }

    /// A reply's subject comes from what it answers; a first mail's is its own, else his name.
    #[dash_test]
    fn a_reply_takes_no_subject_of_its_own() {
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
