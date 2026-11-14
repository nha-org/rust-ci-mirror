//! A document's glossary as a view of the knowledge base (task rust-doc-parity-rest; the older app's `Dashboard.Knowledge.view/1`,
//! rfc-dashboard R6): the entries' cells, written by the design system (`dash_surface::glossary::cells`), read by the doc's own
//! reader into each row they replace. The store's entries are the dash's (`crate::knowledge` there).
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// The glossary id a row's anchor names, `g-<id>`: its first word of that form.
fn row_id(row: &Value) -> Option<&str> {
    (row["attrs"]["anchor"].as_str()?.split_whitespace()).find_map(|a| a.strip_prefix("g-"))
}

/// Each glossary row of `body`, every table's, handed to `f`.
fn each_row(v: &mut Value, f: &mut dyn FnMut(&mut Value)) {
    for b in v["content"].as_array_mut().into_iter().flatten() {
        match b["type"] == "row" && row_id(b).is_some() {
            true => f(b),
            false => each_row(b, f),
        }
    }
}

/// The ids of `body`'s glossary rows, in order.
pub fn ids(body: &Value) -> Vec<String> {
    let mut ids = vec![];
    each_row(&mut body.clone(), &mut |r| {
        ids.extend(row_id(r).map(str::to_string))
    });
    ids
}

/// `body` with each glossary row whose id `cells` holds given those cells: `cells` a page whose each cell is a block anchored
/// `g-<id>-<n>` (`dash_surface::glossary::cells`), read by the doc's own reader so its links and marks land as the doc's own.
/// A row `cells` lacks is left as written; a cell with no words there reads as none.
pub fn view(body: &mut Value, cells: &str) -> Result<(), String> {
    let reader = crate::doc::reader_for(Path::new("glossary.html"), cells.as_bytes())?;
    let doc = crate::doc::to_document(&(reader.read)(cells.as_bytes())?)?.to_json();
    let read: HashMap<&str, &Value> = (doc["content"].as_array().into_iter().flatten())
        .filter_map(|b| Some((b["attrs"]["anchor"].as_str()?, &b["content"])))
        .collect();
    each_row(body, &mut |r| {
        let Some(id) = row_id(r).map(str::to_string) else {
            return;
        };
        // its term cell, never empty, says the entry is there: a prefix test would take `agent-loop`'s for `agent`'s
        if !read.contains_key(format!("g-{id}-0").as_str()) {
            return;
        }
        let mine = r["content"].as_array_mut().into_iter().flatten();
        for (n, cell) in mine.take(3).enumerate() {
            let words = read
                .get(format!("g-{id}-{n}").as_str())
                .map(|c| (*c).clone());
            cell["content"] = words.unwrap_or_else(|| Value::Array(vec![]));
        }
    });
    Ok(())
}
