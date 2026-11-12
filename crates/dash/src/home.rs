//! The older app's reads as cards (task rust-nav-polish; @nha, 2026-11-08: « initial contents should be though about »): what
//! `dash handoff` and `dash progress` print for /inbox, `dash feedback count` and `dash cost fit`
//! for /insights (task rust-home-prompt), each item a card with its title, one line and its link, never their Markdown.
//! ponytail: read off their text, the older app's one form of them; a JSON form of each ends the guessing once the older
//! app's reads move to Rust.
use dash_comments::asks::{self, Ask, Inbox, State};

/// One card: what to read or do, one line about it, where it opens.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub title: String,
    pub line: String,
    pub href: Option<String>,
}

/// A part of a read under its heading (`## Drafts to review (6)`); the part before any heading has none.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub title: String,
    pub cards: Vec<Card>,
}

/// A read: its first sentence, then its parts.
#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    pub lead: String,
    pub groups: Vec<Group>,
}

/// A card's title is at most this long; its whole words are in the text form of `dash handoff`.
const TITLE: usize = 160;

/// The read's cards. `link`: the page an item opens by its part's heading and its id (`reactis-intro -> …`,
/// `now-note: …`), before any link the item names.
pub fn cards(text: &str, link: impl Fn(&str, Option<&str>) -> Option<String>) -> Read {
    let mut lines = text.lines().peekable();
    let lead = match lines.peek() {
        Some(first) if !first.starts_with("## ") && !first.starts_with("- ") => {
            sentence(lines.next().unwrap_or_default()).0
        }
        _ => String::new(),
    };
    let mut groups = vec![Group {
        title: String::new(),
        cards: vec![],
    }];
    // the raw words of each item of the part being read: an item's text can run on several lines
    let mut items: Vec<String> = vec![];
    let mut in_item = false;
    let close = |groups: &mut Vec<Group>, items: &mut Vec<String>| {
        let group = groups.last_mut().expect("a part");
        group
            .cards
            .extend(items.drain(..).map(|raw| card(&raw, &group.title, &link)));
    };
    for line in lines {
        if let Some(title) = line.strip_prefix("## ") {
            close(&mut groups, &mut items);
            groups.push(Group {
                title: title.trim().into(),
                cards: vec![],
            });
            in_item = false;
        } else if let Some(item) = line.strip_prefix("- ") {
            items.push(item.into());
            in_item = true;
        } else if line.starts_with(' ') && in_item && !line.trim().is_empty() {
            // an item's own indented line, its command say, is the item's (/code-review of rust-nav-polish)
            let last = items.last_mut().expect("an item");
            last.push(' ');
            last.push_str(line.trim());
        } else if line.trim().is_empty() || line.starts_with(' ') {
            // a part's note (« Essayé : … », under its heading) is for the session that reads the text; a block's own blank
            // line stays in it: a block's paragraphs are one item
        } else if in_item && !shouted(line) {
            let last = items.last_mut().expect("an item");
            last.push(' ');
            last.push_str(line);
        } else {
            items.push(line.into());
            in_item = false;
        }
    }
    close(&mut groups, &mut items);
    groups.retain(|g| !g.cards.is_empty());
    Read { lead, groups }
}

/// A line that opens with a word in capitals and a colon (`WARNING:`) is its own, never the item above's.
fn shouted(line: &str) -> bool {
    line.split_once(':')
        .is_some_and(|(word, _)| word.len() > 1 && word.chars().all(|c| c.is_ascii_uppercase()))
}

fn card(raw: &str, part: &str, link: &impl Fn(&str, Option<&str>) -> Option<String>) -> Card {
    let named = first_link(raw);
    let words = plain(raw);
    let (id, rest, arrow) = match id_of(&words) {
        Some((id, rest, arrow)) => (Some(id), rest, arrow),
        None => (None, words.as_str(), false),
    };
    let (title, line) = if arrow {
        // a draft: to whom, then its id
        (rest.to_string(), id.unwrap_or_default().to_string())
    } else if let Some((title, blocked)) = rest.split_once(" [").filter(|(_, b)| b.ends_with(']')) {
        (title.to_string(), blocked.trim_end_matches(']').to_string())
    } else {
        let (title, line) = sentence(rest);
        (
            title,
            if line.is_empty() {
                id.unwrap_or_default().to_string()
            } else {
                line
            },
        )
    };
    let href = link(part, id).or(named);
    // « Quatre points que seule Babel peut me dire : » opens a list: its colon goes with it
    Card {
        title: cut(title.trim_end_matches([':', ' '])),
        line,
        href,
    }
}

/// `<id> -> rest` or `<id>: rest`, an id being lower-case words joined by hyphens; true for the arrow.
fn id_of(words: &str) -> Option<(&str, &str, bool)> {
    let at = words.find([':', ' '])?;
    let (id, after) = words.split_at(at);
    let idlike = id.len() > 1
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && id.contains('-');
    if !idlike {
        return None;
    }
    if let Some(rest) = after.strip_prefix(" -> ") {
        return Some((id, rest, true));
    }
    after.strip_prefix(": ").map(|rest| (id, rest, false))
}

/// The first sentence, then the rest; a sentence ends at « . » or « ? » before a space.
fn sentence(words: &str) -> (String, String) {
    let end = [". ", "? "]
        .iter()
        .filter_map(|e| words.find(e).map(|at| at + 1))
        .min();
    match end {
        Some(at) => (
            words[..at].trim_end_matches('.').trim().into(),
            words[at..].trim().into(),
        ),
        None => (words.trim().trim_end_matches('.').into(), String::new()),
    }
}

/// The first link an item names: a Markdown link's target, else a bare http(s) URL.
fn first_link(raw: &str) -> Option<String> {
    if let Some(at) = raw.find("](") {
        let rest = &raw[at + 2..];
        if let Some(end) = rest.find(')') {
            return Some(rest[..end].to_string());
        }
    }
    let at = raw.find("https://").or_else(|| raw.find("http://"))?;
    let url = raw[at..].split_whitespace().next()?;
    Some(url.trim_end_matches(['.', ',', ')', ';']).to_string())
}

/// The item's words with its links, URLs and code marks taken out, its spaces collapsed.
fn plain(raw: &str) -> String {
    let mut out = String::new();
    let mut rest = raw;
    // a Markdown link goes whole: its label (« La tâche ») is the card's link, not its words. Its `[` is the last one before
    // its `](`, so a bracketed note before it (« [blocked by x] ») stays (/code-review of rust-nav-polish).
    while let Some(close) = rest.find("](") {
        let (Some(open), Some(end)) = (
            rest[..close].rfind('['),
            rest[close..].find(')').map(|e| close + e),
        ) else {
            break;
        };
        out += &rest[..open];
        rest = &rest[end + 1..];
    }
    out += rest;
    let words = out
        .split_whitespace()
        .filter(|w| !w.starts_with("http://") && !w.starts_with("https://"));
    words
        .collect::<Vec<_>>()
        .join(" ")
        .replace('`', "")
        .trim_end_matches([',', ' '])
        .to_string()
}

fn cut(title: &str) -> String {
    if title.chars().count() <= TITLE {
        return title.to_string();
    }
    let kept: String = title.chars().take(TITLE - 1).collect();
    format!("{}…", kept.trim_end())
}

/// What a part of `dash handoff` opens: a draft its page, a block or a draft list the review, a sandbox change /approve.
pub fn handoff_link(part: &str, id: Option<&str>) -> Option<String> {
    match (part.split_whitespace().next()?, id) {
        ("Drafts", Some(id)) => Some(format!("/drafts/{id}")),
        ("Blocks", _) | ("Drafts", None) => Some("/review".into()),
        ("Sandbox", _) => Some("/approve".into()),
        _ => None,
    }
}

/// What a task of `dash progress` opens: its page on site_perso's board, whose tasks the daily reads.
pub fn progress_link(_part: &str, id: Option<&str>) -> Option<String> {
    id.map(|id| format!("/projects/site_perso?issue={id}"))
}

/// `dash feedback count` and `dash cost fit` as /insights' parts (task rust-home-prompt; @nha, 2026-11-09: « We miss an
/// insight section / analytics probably too (to port and improve) »): a block between blank lines is a part, its first line
/// its title unless that line is a table's head. A table's row is a card, its first column the title and every other one
/// « head: value »; another line splits at its first wide gap (`cost fit`'s columns) or else its first colon.
pub fn facts(text: &str) -> Read {
    let mut groups = vec![];
    for block in text.split("\n\n") {
        let mut lines = block.lines().filter(|l| !l.trim().is_empty()).peekable();
        let title = match lines.peek() {
            Some(first) if !first.contains(" | ") => {
                lines.next().unwrap_or_default().trim().to_string()
            }
            _ => String::new(),
        };
        let mut head: Vec<&str> = vec![];
        let mut cards = vec![];
        for line in lines {
            // a row whose last cell is empty ends in « |», the table trimmed (/code-review of rust-home-prompt)
            let card = if line.contains(" | ") || line.trim_end().ends_with(" |") {
                let cells: Vec<&str> = line.split('|').map(str::trim).collect();
                if head.is_empty() {
                    head = cells;
                    continue;
                }
                let pairs = (head.iter().zip(&cells).skip(1)).map(|(h, c)| format!("{h}: {c}"));
                (cells[0].to_string(), pairs.collect::<Vec<_>>().join(" · "))
            } else {
                head.clear();
                let line = line.trim();
                let fields: Vec<&str> = (line.split("  ").map(str::trim))
                    .filter(|f| !f.is_empty())
                    .collect();
                match (fields.split_first(), line.split_once(": ")) {
                    (Some((first, rest)), _) if !rest.is_empty() => {
                        (first.to_string(), rest.join(" · "))
                    }
                    (_, Some((title, rest))) => (title.to_string(), rest.to_string()),
                    _ => (line.to_string(), String::new()),
                }
            };
            cards.push(Card {
                title: cut(&card.0),
                line: card.1,
                href: None,
            });
        }
        // a one-line block is its words, never a part that would read « Nothing. » (/code-review of rust-home-prompt)
        match (title.is_empty(), cards.is_empty()) {
            (true, true) => {}
            (false, true) => groups.push(Group {
                title: String::new(),
                cards: vec![Card {
                    title: cut(&title),
                    line: String::new(),
                    href: None,
                }],
            }),
            _ => groups.push(Group { title, cards }),
        }
    }
    Read {
        lead: String::new(),
        groups,
    }
}

/// His inbox's asks, kept beside the comment queue in `comments.sqlite` (dash_comments::asks; task rust-inbox-triage): opened
/// here, a leaf, so /inbox and the check pass both read it with no module loop.
pub fn asks_log(dir: &std::path::Path) -> Result<dash_store::oplog::Replica, String> {
    let db = dash_store::connect(&dir.join("comments.sqlite")).map_err(|e| e.to_string())?;
    db.execute_batch(asks::SCHEMA).map_err(|e| e.to_string())?;
    dash_store::oplog::Replica::new(db, "dash").map_err(|e| e.to_string())
}

/// /inbox's « Answered for you » (task rust-chat-questions-tracked): his chat questions answered, the newest answer first,
/// each its question over its answer, read here beside the asks so /inbox and the comment wiring keep no module loop.
pub fn answered_for_you(dir: &std::path::Path) -> Result<Read, String> {
    // a read never makes the store: a folder with no queue has no answers yet
    let rows = match dir.join("comments.sqlite").exists() {
        true => dash_comments::load(&crate::watch::log(dir)?)?,
        false => Default::default(),
    };
    let cards = (dash_comments::answered_newest(&rows, dash_comments::CHAT, ANSWERED_SHOWN)
        .into_iter())
    .map(|r| Card {
        title: r.text.clone(),
        line: r.answer.clone().unwrap_or_default(),
        href: None,
    })
    .collect();
    Ok(Read {
        lead: String::new(),
        groups: vec![Group {
            title: String::new(),
            cards,
        }],
    })
}

/// The answers /inbox shows, at most: the newest.
const ANSWERED_SHOWN: usize = 20;

/// /inbox's « Waiting on you »: the shown asks as cards, each its plain line, why him and the agent's pick, opening the thing
/// itself; its lead the shown count and how many more wait their turn or are held.
pub fn waiting(dir: &std::path::Path) -> Result<Read, String> {
    // a read never makes the store: a folder with no queue has no asks yet
    if !dir.join("comments.sqlite").exists() {
        return Ok(drawn(&Inbox::default()));
    }
    Ok(drawn(&asks::load(&asks_log(dir)?)?))
}

pub fn drawn(inbox: &Inbox) -> Read {
    let n = |s: State| inbox.asks.values().filter(|a| a.state == s).count();
    let mut shown: Vec<&Ask> = inbox
        .asks
        .values()
        .filter(|a| a.state == State::Shown)
        .collect();
    shown.sort_by_key(|a| {
        (
            a.deadline.clone().unwrap_or_else(|| "9999".into()),
            a.raised_ms,
        )
    });
    let mut lead = format!("{} wait on you", shown.len());
    let (more, held) = (n(State::Checked), n(State::Held));
    if more > 0 {
        lead.push_str(&format!(", {more} more ready wait their turn"));
    }
    if held > 0 {
        lead.push_str(&format!(", {held} held behind a gate of yours"));
    }
    // sent back for what they lack: counted, so nothing leaves his page unsaid (/code-review)
    let back = n(State::Raised);
    if back > 0 {
        lead.push_str(&format!(", {back} went back to their askers"));
    }
    let cards = (shown.iter())
        .map(|a| {
            let mut line = format!(
                "his because: {} · pick: {}",
                a.why.as_deref().unwrap_or(""),
                a.pick
            );
            if !a.sent_by.is_empty() {
                line.push_str(&format!(" · sent by: {}", a.sent_by));
            }
            Card {
                title: a.what.clone(),
                line,
                href: Some(a.link.clone()),
            }
        })
        .collect();
    Read {
        lead,
        groups: vec![Group {
            title: String::new(),
            cards,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    const HANDOFF: &str = "61 things wait on Nicolas. Page: http://100.72.158.115:7788/, first card. File: private/store/handoff.md
To read: Week 45, 2 November to 8 November. http://100.72.158.115:7788/insights#read

## His three steps (À toi) (1)
- Connecte Datadog à Claude Code (Mac : navigateur). [La tâche](http://100.72.158.115:7788/projects/site_perso/issue/datadog-trial) Dans cette session : `/mcp`, choisis `plugin:datadog:mcp`.

## Later (Plus tard) (1)
- Crée le Developer ID G2 de graal (Mac : .csr). [Le portail Apple](https://developer.apple.com/account)  (from 2027-01-04)
WARNING: ready for him to send, on no step: mirakl-ai-intro (form, Mirakl)

## Sandbox to apply (1)
  Essayé : la règle qui manquait est écrite en .candidate.
- sandbox.sb.candidate ou un script du bac à sable: make -C ~/repo/site_perso sandbox/apply

## Drafts to review (1)
- reactis-guillaume-sujet -> Guillaume Aulagner (Business Manager, REACTIS Group)

## Blocks to validate once (1)
- contrat-modele-demande: Bonjour Samir,

Mon CAPE n'est pas encore signé, https://calendly.com/x
";

    fn card(title: &str, line: &str, href: Option<&str>) -> Card {
        Card {
            title: title.into(),
            line: line.into(),
            href: href.map(String::from),
        }
    }

    /// Done-when 1 of rust-nav-polish: each step a card, its title, one line and its link, no Markdown left.
    #[test]
    fn each_step_of_the_handoff_is_a_card_with_its_link() {
        let _budget = TestBudget::new();
        let read = cards(HANDOFF, handoff_link);
        assert_eq!(read.lead, "61 things wait on Nicolas");
        let group = |at: usize| (read.groups[at].title.as_str(), &read.groups[at].cards);
        assert_eq!(
            group(0),
            (
                "",
                &vec![card(
                    "To read: Week 45, 2 November to 8 November",
                    "",
                    Some("http://100.72.158.115:7788/insights#read")
                )]
            )
        );
        assert_eq!(
            group(1),
            (
                "His three steps (À toi) (1)",
                &vec![card(
                    "Connecte Datadog à Claude Code (Mac : navigateur)",
                    "Dans cette session : /mcp, choisis plugin:datadog:mcp.",
                    Some("http://100.72.158.115:7788/projects/site_perso/issue/datadog-trial")
                )]
            )
        );
        let later = &read.groups[2].cards;
        assert_eq!(
            later[0],
            card(
                "Crée le Developer ID G2 de graal (Mac : .csr)",
                "(from 2027-01-04)",
                Some("https://developer.apple.com/account")
            )
        );
        assert_eq!(
            later[1].title,
            "WARNING: ready for him to send, on no step: mirakl-ai-intro (form, Mirakl)",
            "its own card"
        );
        assert_eq!(
            read.groups[3].cards[0].href.as_deref(),
            Some("/approve"),
            "the note under the heading is no card"
        );
        assert_eq!(read.groups[3].cards.len(), 1);
        assert_eq!(
            read.groups[4].cards,
            vec![card(
                "Guillaume Aulagner (Business Manager, REACTIS Group)",
                "reactis-guillaume-sujet",
                Some("/drafts/reactis-guillaume-sujet")
            )]
        );
        let block = &read.groups[5].cards[0];
        assert_eq!(
            (block.title.as_str(), block.href.as_deref()),
            (
                "Bonjour Samir, Mon CAPE n'est pas encore signé",
                Some("/review")
            ),
            "its lines one text, its link the review"
        );
        let every = read.groups.iter().flat_map(|g| &g.cards);
        assert!(
            every
                .clone()
                .all(|c| !c.title.contains("](") && !c.line.contains('`')),
            "no Markdown left: {read:?}"
        );
    }

    #[test]
    fn each_task_of_the_daily_is_a_card_on_its_board() {
        let _budget = TestBudget::new();
        let daily = "Forward progress, today: review. PM: still relevant? Keep it.\n- now-note: dash now prints « what is going on » from the model; /now.txt serves it\n- now-router: A judged router replaces klar-mail's regex routes [blocked by now-jev-judge]\n";
        let read = cards(daily, progress_link);
        assert_eq!(read.lead, "Forward progress, today: review");
        assert_eq!(
            read.groups[0].cards,
            vec![
                card(
                    "dash now prints « what is going on » from the model; /now.txt serves it",
                    "now-note",
                    Some("/projects/site_perso?issue=now-note")
                ),
                card(
                    "A judged router replaces klar-mail's regex routes",
                    "blocked by now-jev-judge",
                    Some("/projects/site_perso?issue=now-router")
                ),
            ]
        );
        assert_eq!(
            cards("", progress_link),
            Read {
                lead: String::new(),
                groups: vec![]
            }
        );
        assert_eq!(cut(&"a".repeat(200)).chars().count(), TITLE);
        assert_eq!(
            cards("- Quatre points :\n", progress_link).groups[0].cards[0].title,
            "Quatre points"
        );
        // /code-review of rust-nav-polish: a note in brackets before a link stays; an item's indented line is its own
        let noted = cards(
            "- now-x: Title [blocked by y]\n- Connecte X (Mac). [La tâche](http://h/x)\n  Dans cette session : /mcp\n",
            progress_link,
        );
        assert_eq!(noted.groups[0].cards[0].line, "blocked by y");
        assert_eq!(plain("a [b] c [d](http://e) f"), "a [b] c f");
        assert_eq!(
            noted.groups[0].cards[1],
            card(
                "Connecte X (Mac)",
                "Dans cette session : /mcp",
                Some("http://h/x")
            )
        );
    }

    /// Done-when 3 of rust-home-prompt: /insights ports `dash feedback count`'s tables and `dash cost fit`'s lines as cards.
    #[test]
    fn the_analytics_tables_are_parts_of_cards() {
        let _budget = TestBudget::new();
        let count = "His feedback, cut at 2026-11-09 (section 3)\nSource | Holds | Counts as feedback\nThe comment queue | 981 entries | 942 items\nItems: 1,820 (942 queue entries)\n\nWeek | Commits | PRs merged\n45, 2 to 8 Nov | 623 | 296\n";
        let read = facts(count);
        assert_eq!(read.groups.len(), 2);
        assert_eq!(
            read.groups[0].title,
            "His feedback, cut at 2026-11-09 (section 3)"
        );
        assert_eq!(
            read.groups[0].cards,
            [
                card(
                    "The comment queue",
                    "Holds: 981 entries · Counts as feedback: 942 items",
                    None
                ),
                card("Items", "1,820 (942 queue entries)", None),
            ]
        );
        assert_eq!(read.groups[1].title, "");
        assert_eq!(
            read.groups[1].cards,
            [card(
                "45, 2 to 8 Nov",
                "Commits: 623 · PRs merged: 296",
                None
            )]
        );
        let fit = "cost fit: 457 finished tasks, per estimate point: 6.7M tokens\n  1 points  25 tasks  6.9M tokens (5.0M to 8.3M)\n  transcripts read 2026-11-09T10:50:15Z\n";
        let read = facts(fit);
        assert_eq!(
            read.groups[0].title,
            "cost fit: 457 finished tasks, per estimate point: 6.7M tokens"
        );
        assert_eq!(
            read.groups[0].cards,
            [
                card("1 points", "25 tasks · 6.9M tokens (5.0M to 8.3M)", None),
                card("transcripts read 2026-11-09T10:50:15Z", "", None),
            ]
        );
    }

    /// /code-review of rust-home-prompt: a row with an empty last cell stays a row, and a one-line block is a card of its words,
    /// never an empty part.
    #[test]
    fn empty_cells_and_one_line_blocks_are_kept() {
        let _budget = TestBudget::new();
        let read = facts("T\nSource | Holds\nFoo |\nBar | 2\n\ncost fit: no finished task yet\n");
        assert_eq!(
            read.groups[0].cards,
            [card("Foo", "Holds: ", None), card("Bar", "Holds: 2", None)]
        );
        assert_eq!(
            (read.groups[1].title.as_str(), &read.groups[1].cards),
            ("", &vec![card("cost fit: no finished task yet", "", None)])
        );
    }
}
