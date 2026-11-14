//! The drafts ready to go and who sends each (`dash outbox`, task rust-daily-today), and the daily's « N to send » line. Read
//! from the older app's export (`dash model drafts`), whose facts carry its rules: `ready` (approved or pre-approved, not a
//! step update the app sends itself), `by-claude`, `banned`, `attach-missing`, `held-until`. Its lines are the older
//! `Dashboard.Views.outbox`'s, word for word, so every caller reading them reads the same.
use dash_core::registry::{Line, Slot};
use serde_json::{Map, Value as Json};

/// A draft `dash outbox` lists.
#[derive(Clone, Debug, PartialEq)]
pub struct Ready {
    pub id: String,
    line: String,
}

/// What goes now, oldest approval first, then what waits for its day, the first day first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outbox {
    pub now: Vec<Ready>,
    pub later: Vec<Ready>,
}

impl Outbox {
    /// The older app's lines, one a draft.
    pub fn lines(&self) -> String {
        let all = self.now.iter().chain(&self.later);
        all.map(|r| r.line.as_str()).collect::<Vec<_>>().join("\n")
    }
}

fn text<'a>(m: &'a Map<String, Json>, key: &str) -> Option<&'a str> {
    m.get(key).and_then(Json::as_str).filter(|s| !s.is_empty())
}

/// The export's drafts as `dash outbox` lists them; a refusal when it is not a list of maps.
pub fn outbox(export: &Json) -> Result<Outbox, String> {
    let list = export.as_array().ok_or("drafts: not a list")?;
    let mut ready = vec![];
    for d in list {
        let m = d.as_object().ok_or("drafts: an entry that is not a map")?;
        // an export that predates its ready fact would list nothing, read as « nothing to send »: refused instead
        match m.get("ready") {
            Some(Json::Bool(true)) => ready.push(m),
            Some(Json::Bool(false)) => {}
            _ => {
                return Err(format!(
                    "drafts: {} says not whether it is ready to send",
                    text(m, "id").unwrap_or("a draft")
                ));
            }
        }
    }
    // a stable sort, as Enum.sort_by: ties keep the export's order
    ready.sort_by_key(|m| text(m, "approved-on").unwrap_or_default());
    let (later, now): (Vec<_>, Vec<_>) = ready
        .into_iter()
        .partition(|m| text(m, "held-until").is_some());
    let mut later: Vec<_> = later
        .into_iter()
        .map(|m| (text(m, "held-until"), m))
        .collect();
    later.sort_by_key(|(day, _)| *day);
    let row = |m: &Map<String, Json>, line: String| Ready {
        id: text(m, "id").unwrap_or_default().to_string(),
        line,
    };
    Ok(Outbox {
        now: now.into_iter().map(|m| row(m, goes(m))).collect(),
        later: later
            .into_iter()
            .map(|(day, m)| {
                let (id, channel, to) = named(m);
                let day = day.unwrap_or_default();
                row(m, format!("{id}  {channel}  goes out on {day}  {to}"))
            })
            .collect(),
    })
}

fn named(m: &Map<String, Json>) -> (&str, &str, &str) {
    let id = text(m, "id").unwrap_or_default();
    (
        id,
        text(m, "channel").unwrap_or("?"),
        text(m, "to").unwrap_or_default(),
    )
}

/// A draft that goes now: when it was approved, then who sends it, or why the send refuses it.
fn goes(m: &Map<String, Json>) -> String {
    let (id, channel, to) = named(m);
    let on = match text(m, "approved-on") {
        Some(day) => format!("approuvé {day}"),
        None => "blocs approuvés".into(),
    };
    let claude = m.get("by-claude") == Some(&Json::Bool(true));
    let banned: Vec<&str> = (m
        .get("banned")
        .and_then(Json::as_array)
        .into_iter()
        .flatten())
    .filter_map(Json::as_str)
    .collect();
    let why = banned.join(" ; ");
    let how = match text(m, "attach-missing") {
        Some(missing) if claude => format!(
            "refusé à l'envoi (pièce jointe) : {missing} ; repasse-le en pending, ajoute le fichier à :attach, il réapprouve (--no-attachment seulement si le mail part vraiment sans)"
        ),
        _ if claude && !banned.is_empty() => {
            format!("refusé à l'envoi (banned-phrases.edn) : {why}")
        }
        _ if claude && channel == "email" => format!(
            "Claude l'envoie : private/dashboard/bin/dash mail send {id} (envoie, journalise, marque envoyé)"
        ),
        _ if claude => "Claude l'envoie (MCP), puis le marque envoyé".into(),
        _ if !banned.is_empty() => format!("à lui ({channel}), phrase interdite : {why}"),
        _ => format!("à lui ({channel})"),
    };
    format!("{id}  {channel}  {on}  -> {how}  {to}")
}

/// Where « N to send » leads: /drafts showing the outbox's drafts that go now.
pub const TO_SEND: &str = "/drafts?show=to-send";

/// The drafts plugin's line in the daily's « today » slot: how many drafts go now. The binary brings the count: `None` while its
/// first read has not ended.
pub struct ToSend<F>(pub F);

impl<F: Fn() -> Result<Option<usize>, String>> Slot for ToSend<F> {
    fn line(&self) -> Line {
        Line {
            says: (self.0)().map(|n| n.map(|n| format!("{n} to send"))),
            what: "drafts approved, not yet sent".into(),
            href: TO_SEND.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use serde_json::json;

    /// Done-when 3 of rust-daily-today: the older `Dashboard.Views.outbox`'s lines, word for word, on a fixture of each case
    /// its tests held (draft_send_gate_test, send_on_test, the live outbox of 2026-11-09).
    #[dash_test]
    fn the_outbox_lists_the_older_lines() {
        let export = json!([
            {"id": "sent-one", "status": "sent", "ready": false, "channel": "email", "to": "X"},
            {"id": "epm-ruby-reply", "ready": true, "by-claude": true, "channel": "linkedin",
             "approved-on": "2026-11-03", "to": "Ruby Ryder (EPM Scientific)", "banned": []},
            {"id": "odhcom-audrey-accepte", "ready": true, "by-claude": true, "channel": "linkedin",
             "to": "Audrey (FreelanceRepublik)", "banned": []},
            {"id": "mirakl-ai-intro", "ready": true, "by-claude": false, "channel": "form",
             "approved-on": "2026-10-23", "to": "Mirakl", "banned": []},
            {"id": "banned-mail", "ready": true, "by-claude": true, "channel": "email",
             "approved-on": "2026-10-31", "to": "Guillaume, guillaume@example.com", "banned": ["autant le dire"]},
            {"id": "his-mail", "ready": true, "by-claude": false, "channel": "email",
             "approved-on": "2026-10-31", "to": "G", "banned": ["autant le dire", "n'hésitez pas"]},
            {"id": "attach-mail", "ready": true, "by-claude": true, "channel": "email",
             "approved-on": "2026-10-31", "to": "Jobs", "banned": [], "attach-missing": "says « CV » with no :attach"},
            {"id": "b-follow", "ready": true, "by-claude": true, "channel": "email",
             "approved-on": "2026-10-30", "to": "B", "banned": []},
            {"id": "later-mail", "ready": true, "by-claude": true, "channel": "email",
             "approved-on": "2026-10-24", "held-until": "2026-10-26", "to": "Guillaume", "banned": []},
            {"id": "sooner", "ready": true, "by-claude": true, "held-until": "2026-10-25", "to": "S"},
        ]);
        let listed = outbox(&export).expect("read");
        assert_eq!(
            listed.lines().lines().collect::<Vec<_>>(),
            [
                "odhcom-audrey-accepte  linkedin  blocs approuvés  -> Claude l'envoie (MCP), puis le marque envoyé  Audrey (FreelanceRepublik)",
                "mirakl-ai-intro  form  approuvé 2026-10-23  -> à lui (form)  Mirakl",
                "b-follow  email  approuvé 2026-10-30  -> Claude l'envoie : private/dashboard/bin/dash mail send b-follow (envoie, journalise, marque envoyé)  B",
                "banned-mail  email  approuvé 2026-10-31  -> refusé à l'envoi (banned-phrases.edn) : autant le dire  Guillaume, guillaume@example.com",
                "his-mail  email  approuvé 2026-10-31  -> à lui (email), phrase interdite : autant le dire ; n'hésitez pas  G",
                "attach-mail  email  approuvé 2026-10-31  -> refusé à l'envoi (pièce jointe) : says « CV » with no :attach ; repasse-le en pending, ajoute le fichier à :attach, il réapprouve (--no-attachment seulement si le mail part vraiment sans)  Jobs",
                "epm-ruby-reply  linkedin  approuvé 2026-11-03  -> Claude l'envoie (MCP), puis le marque envoyé  Ruby Ryder (EPM Scientific)",
                "sooner  ?  goes out on 2026-10-25  S",
                "later-mail  email  goes out on 2026-10-26  Guillaume",
            ]
        );
        let ids: Vec<&str> = listed.now.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids.len(), 7, "{ids:?}");
        assert!(outbox(&json!({})).is_err() && outbox(&json!([1])).is_err());
        let older =
            outbox(&json!([{"id": "old", "status": "approved"}])).expect_err("no ready fact");
        assert!(older.contains("old says not whether"), "{older}");
        assert_eq!(outbox(&json!([])).expect("empty").lines(), "");
    }

    #[dash_test]
    fn to_send_says_the_count_and_opens_the_list() {
        let line = ToSend(|| Ok(Some(3))).line();
        assert_eq!(
            (line.says, line.href.as_str()),
            (Ok(Some("3 to send".into())), TO_SEND)
        );
        // still reading or failed, the line still knows what it counts and where it opens
        let reading = ToSend(|| Ok(None)).line();
        assert_eq!((reading.says, reading.href.as_str()), (Ok(None), TO_SEND));
        assert!(ToSend(|| Err("down".into())).line().says.is_err());
    }
}
