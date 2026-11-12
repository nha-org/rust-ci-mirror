//! A document's glossary rows as the knowledge base's entries show them (task rust-doc-parity-rest; the older app's
//! `Dashboard.Knowledge.view/1`): each entry's term linked to /k/<id>, its meaning and its source, a paragraph a cell anchored
//! `g-<id>-<n>`, which `dash_ingest::glossary::view` reads into the doc's rows.
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Each `href="…"` of `html` given to `to`, which returns it as it should be.
fn hrefs(html: &str, to: &dyn Fn(&str) -> String) -> String {
    let (mut out, mut rest) = (String::new(), html);
    while let Some(at) = rest.find("href=\"") {
        let (before, after) = rest.split_at(at + 6);
        out.push_str(before);
        let end = after.find('"').unwrap_or(after.len());
        out.push_str(&to(&after[..end]));
        rest = &after[end..];
    }
    out + rest
}

/// An entry's HTML as a doc shows it, `Dashboard.Knowledge.local/2` and `Docs.links/2`: a link to an entry the doc has no row
/// for (`here`) goes to that entry's page, /k/<id>; a link relative to its home, a file beside it, goes to that file's page.
fn linked(html: &str, home: &str, here: &HashSet<&str>) -> String {
    hrefs(html, &|href| match href.strip_prefix("#g-") {
        Some(id) if !here.contains(id) => format!("/k/{id}"),
        Some(_) => href.to_string(),
        None if href.is_empty() || href.starts_with(['#', '/']) || href.contains(':') => {
            href.into()
        }
        None => match home.rsplit_once('/') {
            Some((folder, _)) => format!("/v/{folder}/{href}"),
            None => format!("/v/{href}"),
        },
    })
}

/// The cells of each of `ids` that `found` holds, each entry a record (`term`, `meaning`, `source` as HTML, `home` its doc), as
/// a page of paragraphs anchored `g-<id>-<n>`: the term linked to /k/<id>, then the meaning and the source, their links read
/// against `ids`, the doc's rows.
pub fn cells(ids: &[String], found: &HashMap<String, Value>) -> String {
    let here: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let rows: String = (ids.iter().filter_map(|id| Some((id, found.get(id)?))))
        .map(|(id, e)| {
            let home = e["home"].as_str().unwrap_or_default();
            let say = |k: &str| e[k].as_str().unwrap_or_default().to_string();
            let term = format!(r#"<a href="/k/{id}">{}</a>"#, say("term"));
            let all = [
                term,
                linked(&say("meaning"), home, &here),
                linked(&say("source"), home, &here),
            ];
            (all.iter().enumerate())
                .map(|(n, cell)| format!(r#"<p id="g-{id}-{n}">{cell}</p>"#))
                .collect::<String>()
        })
        .collect();
    format!("<!doctype html><meta charset=utf-8><main>{rows}</main>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_s_links_go_to_its_page_or_its_home_s_file() {
        let _budget = dash_meter::memory::TestBudget::new();
        let here: HashSet<&str> = ["ask"].into();
        let html = r##"<a href="#g-ask">a</a> <a href="#g-point6">p</a> <a href="#bars">b</a> <a href="cape.html#x">c</a> <a href="https://x.fr/y">d</a>"##;
        assert_eq!(
            linked(html, "private/docs/asks.html", &here),
            r##"<a href="#g-ask">a</a> <a href="/k/point6">p</a> <a href="#bars">b</a> <a href="/v/private/docs/cape.html#x">c</a> <a href="https://x.fr/y">d</a>"##
        );
    }
}
