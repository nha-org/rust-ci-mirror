//! Mail's channel « email » (rfc-rust-plugins-ui section 3, the Channel port; task draft-from-is-address): a message's checks
//! before he sees it, and its send, once, logged in mail's own send log. Another plugin reaches it by key through the core,
//! never by import; the host hands in the transport, the clock and the send log.
use crate::thread::{New, SCHEMA, recipients, send_new_once, send_once, thread};
use crate::{OTHERS, OWN};
use dash_core::{Check, Host, Message};

/// The key other plugins send by.
pub const EMAIL: &str = "email";
/// The send log's file in the data folder, the one /mail's reply writes.
pub const FILE: &str = "mail.sqlite";
/// The outbound gate's key: its lines are why a text may not leave.
pub const GATE: &str = "hooks/outbound-gate";

/// An address as a mail's To takes it: one `user@domain.tld`, nothing around it.
fn is_address(a: &str) -> bool {
    let bad = a.contains(char::is_whitespace) || a.contains(['<', '>', ',', ';']);
    a.split_once('@')
        .is_some_and(|(user, domain)| !bad && !user.is_empty() && domain.contains('.'))
}

/// Why the outbound gate keeps `text` from leaving, read by key through the core, as a reply to their mail (neither cold nor a
/// first message): /mail's own reply and new mail ask it here too. A gate that does not answer refuses: nothing leaves
/// unjudged.
pub fn judged(host: &dyn Host, text: &str) -> Option<String> {
    gate(host, text).map(|(why, _)| why)
}

/// Why the gate keeps `text`, and whether it did not answer (a retry, not a refusal).
fn gate(host: &dyn Host, text: &str) -> Option<(String, bool)> {
    match dash_core::context(host, GATE, text) {
        Ok(why) if why.is_empty() => None,
        Ok(why) => Some((why.join("; "), false)),
        Err(e) => Some((format!("{e}: nothing leaves unjudged"), true)),
    }
}

/// Every check of `m`, each passed or failed: the sender one of his addresses, the receivers addresses and, for a reply,
/// those the thread's reply goes to, and the outbound gate open on its words. A gate that does not answer fails it: nothing
/// leaves unjudged.
pub fn checks(host: &dyn Host, m: &Message) -> Vec<Check> {
    let ours = std::iter::once(OWN).chain(OTHERS);
    let sender = (!ours.clone().any(|a| a.eq_ignore_ascii_case(m.from.trim()))).then(|| {
        let ours: Vec<_> = ours.collect();
        format!(
            "« {} » is not one of his addresses ({})",
            m.from,
            ours.join(", ")
        )
    });
    let gate = gate(host, &m.text);
    let (to, unread) = receivers(host, m).map_or((None, false), |(why, r)| (Some(why), r));
    vec![
        Check {
            name: "sender",
            fails: sender,
            retry: false,
        },
        Check {
            name: "receivers",
            fails: to,
            retry: unread,
        },
        Check {
            name: "outbound gate",
            retry: gate.as_ref().is_some_and(|g| g.1),
            fails: gate.map(|g| g.0),
        },
    ]
}

/// Why `m`'s receivers fail: none, one that is no address, or, for a reply, not those its thread's reply goes to; and whether
/// the thread did not read (a retry).
fn receivers(host: &dyn Host, m: &Message) -> Option<(String, bool)> {
    if m.to.is_empty() {
        return Some(("no receiver".into(), false));
    }
    if let Some(a) = m.to.iter().find(|a| !is_address(a)) {
        return Some((format!("« {a} » is no address"), false));
    }
    let key = m.answers.as_deref()?;
    let read = host.signed_in(&m.from).and_then(|http| thread(&*http, key));
    let goes = match read {
        Ok(t) => recipients(&m.from, &t),
        Err(e) => return Some((format!("its thread did not read: {e}"), true)),
    };
    let same = goes.len() == m.to.len()
        && (goes.iter()).all(|g| m.to.iter().any(|t| t.eq_ignore_ascii_case(g)));
    (!same).then(|| {
        let why = format!(
            "the thread's reply goes to {}, not {}",
            goes.join(", "),
            m.to.join(", ")
        );
        (why, false)
    })
}

/// `m` sent once, its checks run again first: the mail made, or why none went. A reply answers its thread's latest mail; the
/// same words to the same mail, or the same key, go once (mail's send log).
pub fn send(host: &dyn Host, m: &Message) -> Result<String, String> {
    if let Some(c) = checks(host, m).into_iter().find(|c| c.fails.is_some()) {
        return Err(format!(
            "{}: {}; nothing sent",
            c.name,
            c.fails.unwrap_or_default()
        ));
    }
    let http = host.signed_in(&m.from)?;
    let mut log = host.open(&host.data(), FILE, &[SCHEMA])?;
    let at = dash_core::iso(host.now_ms().div_euclid(1000));
    // ponytail: plain text only, no HTML part (the design system draws that, and mail takes no crate of it); the older app's
    // outbound log is the binary's to write, so the older app's records miss this send until the drafts move ports it
    let sent = match &m.answers {
        Some(key) => {
            let t = thread(&*http, key)?;
            send_once(&*http, &mut *log, &m.from, &t, &m.text, None, &at)?
        }
        None => {
            let to = m.to.clone();
            let new = New {
                to,
                subject: m.subject.clone(),
                text: m.text.clone(),
                draft: m.key.clone(),
                ..New::default()
            };
            send_new_once(&*http, &mut *log, &m.from, &new, None, &at)?
        }
    };
    if let Err(e) = &sent.logged {
        eprintln!("mail: sent {}, but {FILE} refused its row: {e}", sent.email);
    }
    Ok(sent.email)
}

inventory::submit! { dash_core::Channel { key: EMAIL, checks, send } }
