//! What a draft answers, for its page (task rust-draft-posting; @nha, 2026-10-23: « I'd like to see the ad or profile or
//! something »): the ad from the store's postings/ archive, as the older app's `Dashboard.Postings.of/1` and `parse/1`
//! read it, where it is sent, and its lead's last events, its search note among them.
use serde_json::{Map, Value as Json};

/// An archived ad: its facts in the page's order (Title, Company, Location, Contract), the empty ones left out, its URL, its
/// text.
#[derive(Debug, Default, PartialEq)]
pub struct Posting {
    pub facts: Vec<(&'static str, String)>,
    pub url: Option<String>,
    pub text: String,
}

/// The archive `draft` answers among `postings` (`dash store files postings`: file name to text): one named by the draft's id
/// when the draft names an ad, else the one whose Source is that ad, else its lead's. A draft naming its own ad answers that
/// one, never its lead's.
pub fn archive_of<'a>(draft: &Json, postings: &'a Json) -> Option<&'a str> {
    let files = postings.as_object()?;
    let named = |k: Option<&str>| files.get(&format!("{}.md", k?))?.as_str();
    let lead = draft["opportunity"].as_str();
    let Some(url) = draft["posting"].as_str() else {
        return named(lead);
    };
    // the Source's whole URL: an ad's URL can be the start of another's, a note may follow it
    let sourced = |body: &&str| {
        body.lines().any(|l| {
            (l.strip_prefix("**Source:** ")
                .and_then(|r| r.strip_prefix(url)))
            .is_some_and(|after| after.is_empty() || after.starts_with(char::is_whitespace))
        })
    };
    named(draft["id"].as_str())
        .or_else(|| files.values().filter_map(Json::as_str).find(sourced))
        .or_else(|| named(lead))
}

/// An archive's facts and text: its header's `**Key:** value` lines, else its reader's first line, « # title | company |
/// location | … » (li_jobs.py's contract in its criteria line, freework.py's in the line's fifth field).
pub fn parse(body: &str) -> Posting {
    let (head, text) = match rule(body) {
        Some((h, t)) if header(h) => (h, t.trim()),
        _ => ("", body.trim()),
    };
    let named: Vec<(&str, &str)> = head.lines().filter_map(key_line).collect();
    let get = |k: &str| {
        named
            .iter()
            .rev()
            .find(|(n, _)| *n == k)
            .map(|(_, v)| v.to_string())
    };
    let read = if get("Title").is_some() {
        Default::default()
    } else {
        first_line(text)
    };
    let fact = |k: &'static str, i: usize| get(k).or_else(|| read[i].clone()).map(|v| (k, v));
    Posting {
        facts: [
            ("Title", 0),
            ("Company", 1),
            ("Location", 2),
            ("Contract", 3),
        ]
        .into_iter()
        .filter_map(|(k, i)| fact(k, i))
        .filter(|(_, v)| !v.is_empty())
        .collect(),
        url: get("Source").and_then(|s| s.split_whitespace().next().map(str::to_string)),
        text: text.to_string(),
    }
}

/// Its header and its text, either side of its first `---` line.
fn rule(body: &str) -> Option<(&str, &str)> {
    let mut at = 0;
    for line in body.split_inclusive('\n') {
        if line
            .strip_prefix("---")
            .is_some_and(|r| r.trim().is_empty())
        {
            return Some((&body[..at], &body[at + line.len()..]));
        }
        at += line.len();
    }
    None
}

/// A header names a fact, even an empty one, or is a lone heading: an archive with none keeps a `---` rule of its own text.
fn header(head: &str) -> bool {
    let t = head.trim();
    head.lines().any(|l| key(l).is_some()) || (t.starts_with('#') && !t.contains('\n'))
}

/// A `**Key:**` line's key and what follows it.
fn key(line: &str) -> Option<(&str, &str)> {
    let (k, v) = line.strip_prefix("**")?.split_once(":**")?;
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == ' ';
    (!k.is_empty() && k.chars().all(word)).then_some((k, v.trim()))
}

/// A `**Key:** value` line's key and value, a value said.
fn key_line(line: &str) -> Option<(&str, &str)> {
    key(line).filter(|(_, v)| !v.is_empty())
}

/// An older archive's Title, Company, Location and Contract from its reader's first line, read from the end since a title may
/// hold « | »; a shorter line written by hand (« title | company | location ») from the front.
fn first_line(text: &str) -> [Option<String>; 4] {
    let first = text.lines().next().unwrap_or("");
    let line: Vec<&str> = (first.strip_prefix('#'))
        .filter(|r| r.starts_with(char::is_whitespace) && r.contains('|'))
        .map_or(vec![], |r| r.trim().split(" | ").collect());
    let criteria = ["\"Employment type\":", "\"Type d’emploi\":"]
        .iter()
        .find_map(|k| {
            let rest = text.split_once(k)?.1.trim_start().strip_prefix('"')?;
            Some(rest.split_once('"')?.0.to_string())
        });
    let n = line.len();
    // li_jobs.py's heading of 4 fields or more, its link under it; a shorter one reads from the front (/code-review)
    let linkedin = n >= 4
        && (text.lines().nth(1)).is_some_and(|l| l.starts_with("https://www.linkedin.com/jobs/"));
    let (title, rest, contract) = if linkedin {
        let cut = n.saturating_sub(3);
        (&line[..cut], &line[cut..], criteria)
    } else if n >= 5 {
        (
            &line[..n - 4],
            &line[n - 4..],
            Some(line[n - 1].to_string()),
        )
    } else {
        (&line[..n.min(1)], &line[n.min(1)..], None)
    };
    let title = (!title.is_empty()).then(|| title.join(" | "));
    let at = |i: usize| rest.get(i).map(|s| s.to_string());
    [title, at(0), at(1), contract]
}

/// Lead `id` among `leads` (`dash store files opportunities`: file name to its JSON text), its id a keyword (« ~:id »): a
/// file that does not read is another lead's, never a reason to lose this one (/code-review).
pub fn lead(leads: &Json, id: &str) -> Option<Map<String, Json>> {
    leads.as_object()?.values().find_map(|text| {
        let lead: Json = serde_json::from_str(text.as_str()?).ok()?;
        let lead = lead.as_object()?;
        (crate::keyword(lead, "id").as_deref() == Some(id)).then(|| lead.clone())
    })
}

/// Where a form draft is sent: its lead's application page, else the first link of its `where`; `None` for any other channel.
pub fn apply(draft: &Json, lead: Option<&Map<String, Json>>) -> Option<String> {
    if draft["channel"] != "form" {
        return None;
    }
    let page = lead.and_then(|l| l.get("urls")?.get("apply")?.as_str());
    let said = || {
        let w = draft["where"].as_str()?;
        let at = w.find("https://").or_else(|| w.find("http://"))?;
        let end = |c: char| c.is_whitespace() || ",)»\"".contains(c);
        let url = w[at..]
            .split(end)
            .next()?
            .trim_end_matches(['.', ';', ':', '!', '?', '\'']);
        Some(url)
    };
    page.or_else(said).map(str::to_string)
}

/// The lead's last `n` events up to `today`, newest first, the last written first on a shared day: each its day, its words cut
/// at 80 characters, and its source when a web link, else `None` (the page links the lead).
/// ponytail: a ref to a store file (a mail) is not linked to its viewer, as the older app's `Markdown.viewer_path/1` did; the
/// lead's page holds it.
pub fn recent(
    lead: &Map<String, Json>,
    today: &str,
    n: usize,
) -> Vec<(String, String, Option<String>)> {
    let events = lead
        .get("timeline")
        .and_then(Json::as_array)
        .into_iter()
        .flatten();
    let mut dated: Vec<(String, String, Option<String>)> = events
        .filter_map(|e| {
            // an ISO day, its time ignored as the older page did: same-day events stay in their written order
            let iso = |d: &&str| {
                (d.bytes().enumerate()).all(|(i, b)| {
                    if i == 4 || i == 7 {
                        b == b'-'
                    } else {
                        b.is_ascii_digit()
                    }
                })
            };
            let day = e["date"].as_str()?.get(..10).filter(iso)?.to_string();
            let words = match &e["event"] {
                Json::String(s) => s.clone(),
                Json::Null => String::new(),
                other => other.to_string(),
            };
            let source = (e["ref"].as_str())
                .filter(|r| {
                    (r.starts_with("https://") || r.starts_with("http://")) && !r.contains(' ')
                })
                .map(str::to_string);
            (day.as_str() <= today).then(|| (day, cut(&words), source))
        })
        .collect();
    dated.reverse();
    dated.sort_by(|a, b| b.0.cmp(&a.0));
    dated.truncate(n);
    dated
}

/// An event's words as one line: an empty one still says so, a long one cut with « … ».
fn cut(words: &str) -> String {
    let t = words.trim();
    match t.chars().count() {
        0 => "(no summary)".into(),
        n if n > 80 => t.chars().take(79).chain(['…']).collect(),
        _ => t.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use serde_json::json;

    /// The archive of /d/xpertdirect-ml-rag-intro's lead as `dash archive-posting` wrote it, 2026-11-10.
    const LINKEDIN: &str = "# xpertdirect-ml-engineer-llm-rag\n\n**Archived:** 2026-11-10 (the ad can be pulled at any time; this copy is what we\nprepare interviews against)\n**Source:** https://www.linkedin.com/jobs/view/4487144354/\n\n---\n\n# Senior Machine Learning Engineer (LLM & RAG Systems) | XpertDirect | European Union | 32 applicants\nhttps://www.linkedin.com/jobs/view/4487144354/\n{\"Seniority level\": \"Mid-Senior level\", \"Employment type\": \"Contract\"}\n\nRemote (EU) | Munich-based Company\n";

    #[dash_test]
    fn a_linkedin_archive_reads_its_facts_from_its_first_line_and_criteria() {
        let p = parse(LINKEDIN);
        let facts: Vec<(&str, &str)> = p.facts.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(
            facts,
            [
                (
                    "Title",
                    "Senior Machine Learning Engineer (LLM & RAG Systems)"
                ),
                ("Company", "XpertDirect"),
                ("Location", "European Union"),
                ("Contract", "Contract"),
            ]
        );
        assert_eq!(
            p.url.as_deref(),
            Some("https://www.linkedin.com/jobs/view/4487144354/")
        );
        assert!(
            p.text.starts_with("# Senior Machine") && p.text.ends_with("Munich-based Company"),
            "{}",
            p.text
        );
    }

    #[dash_test]
    fn a_header_naming_its_facts_wins_and_a_freework_line_gives_its_fifth_field() {
        let watch = "# x\n\n**Source:** https://a.example/1 a note\n**Title:** Lead dev | Rust\n**Company:** Acme\n\n---\n\nThe ad.";
        let p = parse(watch);
        assert_eq!(
            p.facts,
            [
                ("Title", "Lead dev | Rust".into()),
                ("Company", "Acme".into())
            ]
        );
        assert_eq!(
            (p.url.as_deref(), p.text.as_str()),
            (Some("https://a.example/1"), "The ad.")
        );
        let freework = "# y\n\n**Source:** https://f.example/2\n\n---\n\n# Tech Lead | COEXYA | Guyancourt | 2026-09-29 | contractor\nhttps://f.example/2\n";
        let p = parse(freework);
        assert_eq!(p.facts[0], ("Title", "Tech Lead".into()));
        assert_eq!(p.facts[3], ("Contract", "contractor".into()));
        // no header: a rule inside the ad's own text stays its text; a header of empty keys is still one
        assert_eq!(parse("an ad\n---\nmore").text, "an ad\n---\nmore");
        assert_eq!(parse("**Archived:**\n---\nThe ad").text, "The ad");
        // a short heading over a LinkedIn link reads from the front, a link under prose is no heading (/code-review)
        let criteria = "\n\"Employment type\": \"Contract\"";
        let short = parse(&format!(
            "# Rust dev | Acme\nhttps://www.linkedin.com/jobs/view/1/{criteria}"
        ));
        assert_eq!(
            short.facts,
            [("Title", "Rust dev".into()), ("Company", "Acme".into())]
        );
        assert!(
            parse(&format!(
                "Prose\nhttps://www.linkedin.com/jobs/view/1/{criteria}"
            ))
            .facts
            .is_empty()
        );
    }

    #[dash_test]
    fn a_draft_naming_its_ad_answers_that_one_before_its_leads() {
        let postings = json!({"lead.md": "lead's", "other.md": "**Source:** https://a.example/12\n", "mine.md": "**Source:** https://a.example/1\n"});
        let draft = json!({"id": "d", "opportunity": "lead", "posting": "https://a.example/1"});
        assert_eq!(
            archive_of(&draft, &postings),
            Some("**Source:** https://a.example/1\n")
        );
        assert_eq!(
            archive_of(&json!({"opportunity": "lead"}), &postings),
            Some("lead's")
        );
        assert_eq!(archive_of(&json!({"opportunity": "none"}), &postings), None);
    }

    #[dash_test]
    fn a_form_goes_to_its_leads_application_page_else_the_link_its_where_names() {
        let lead = json!({"urls": {"apply": "https://a.example/apply"}});
        let form = json!({"channel": "form", "where": "Easy Apply, https://w.example/jobs/1/, champ message"});
        assert_eq!(
            apply(&form, lead.as_object()).as_deref(),
            Some("https://a.example/apply")
        );
        assert_eq!(
            apply(&form, None).as_deref(),
            Some("https://w.example/jobs/1/")
        );
        assert_eq!(apply(&json!({"channel": "email"}), lead.as_object()), None);
    }

    #[dash_test]
    fn the_leads_last_events_come_newest_first_none_planned() {
        let lead = json!({"timeline": [
            {"date": "2026-11-09", "event": "first", "ref": "https://a.example/1"},
            {"date": "2026-11-10", "event": "search note", "ref": "mail/x.eml"},
            {"date": "2026-11-10", "event": "x".repeat(90)},
            {"date": "2026-11-20", "event": "a planned call"},
            {"date": "2026-11-01", "event": ""},
            {"date": "10/11/2026", "event": "no ISO day"},
        ]});
        let got = recent(lead.as_object().unwrap(), "2026-11-11", 3);
        assert_eq!(got[0].0, "2026-11-10");
        assert_eq!(got[0].1.chars().count(), 80);
        assert_eq!(got[1], ("2026-11-10".into(), "search note".into(), None));
        assert_eq!(
            got[2],
            (
                "2026-11-09".into(),
                "first".into(),
                Some("https://a.example/1".into())
            )
        );
        let all = recent(lead.as_object().unwrap(), "2026-11-11", 9);
        assert_eq!((all.len(), all[3].1.as_str()), (4, "(no summary)"));
    }

    #[dash_test]
    fn a_lead_is_found_past_a_file_that_does_not_read() {
        let mine = r#"{"id": "~:mine", "x": 1}"#;
        let leads =
            json!({"2026-11-a.edn": "{half", "2026-11-b.edn": "[1]", "2026-11-c.edn": mine});
        assert_eq!(lead(&leads, "mine").map(|l| l["x"].clone()), Some(json!(1)));
        assert_eq!(lead(&leads, "other"), None);
    }
}
