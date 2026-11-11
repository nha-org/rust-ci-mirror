use super::parts::card_href;
use crate::component::{Component, Section};
use dioxus::prelude::*;

/// One row of a Table: its cells in its columns' order, and where it opens, if anywhere.
#[derive(Clone, PartialEq, serde::Deserialize)]
pub struct TableRow {
    pub cells: Vec<String>,
    #[serde(default)]
    pub href: Option<String>,
}

/// How a table is shown, as its URL says (task rust-table-sortable; @nha, 2026-11-09: « the component should be a table and
/// that should be sortable etc. design system first. »): `?filter=<words>` its rows holding them, case aside, `&sort=<column>`,
/// `&dir=desc` reversed, `&page=<n>` from 1 (task design-agent-forms; @nha, 2026-11-09: « Is there pagination for agents ?
/// filters ? »). One order a page: ponytail, every table of a page reads the same keys; a key per table id once a page holds
/// two long ones.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Order {
    pub filter: Option<String>,
    pub sort: Option<String>,
    pub desc: bool,
    pub page: usize,
}

/// The keys a table's order takes in the URL: every other key of the query is kept in its links.
const ORDER_KEYS: [&str; 4] = ["filter", "sort", "dir", "page"];

/// The rows a page shows at most before Pagination takes the rest.
pub const PER_PAGE: usize = 50;

/// A query's pairs, each value decoded as a form sends it: `+` a space, `%XX` its byte; a bad escape stays as it is.
fn pairs(query: &str) -> impl Iterator<Item = (&str, String)> {
    query.split('&').filter(|p| !p.is_empty()).map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let bytes = value.as_bytes();
        let (mut out, mut i) = (Vec::with_capacity(bytes.len()), 0);
        while i < bytes.len() {
            let hex = bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok());
            match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
                (b'%', Some(byte)) => {
                    out.push(byte);
                    i += 3;
                }
                (b'+', _) => {
                    out.push(b' ');
                    i += 1;
                }
                (b, _) => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        (key, String::from_utf8_lossy(&out).into_owned())
    })
}

/// A value as a query holds it: every byte but a letter, a digit, `-`, `_`, `.` and `~` escaped.
fn encoded(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b => format!("%{b:02X}"),
        })
        .collect()
}

impl Order {
    /// The order a query names; a page that is not a number, or 0, is the first.
    pub fn of(query: &str) -> Order {
        let mut order = Order {
            page: 1,
            ..Order::default()
        };
        for (key, value) in pairs(query) {
            match key {
                "filter" if !value.trim().is_empty() => order.filter = Some(value.trim().into()),
                "sort" if !value.is_empty() => order.sort = Some(value),
                "dir" => order.desc = value == "desc",
                "page" => order.page = value.trim().parse().ok().filter(|&p| p > 0).unwrap_or(1),
                _ => {}
            }
        }
        order
    }

    /// The link to this order: the query's other keys kept, in their order, then the order's own, a default left out.
    pub fn href(&self, query: &str) -> String {
        let mut kept: Vec<String> = query
            .split('&')
            .filter(|p| !p.is_empty() && !ORDER_KEYS.contains(&p.split('=').next().unwrap_or(p)))
            .map(String::from)
            .collect();
        if let Some(filter) = &self.filter {
            kept.push(format!("filter={}", encoded(filter)));
        }
        if let Some(sort) = &self.sort {
            kept.push(format!("sort={}", encoded(sort)));
            if self.desc {
                kept.push("dir=desc".into());
            }
        }
        if self.page > 1 {
            kept.push(format!("page={}", self.page));
        }
        format!("?{}", kept.join("&"))
    }

    /// Where a click on `column`'s head leads: sorted by it, the same column again reversed, back to the first page, the
    /// filter kept (rust-ui's DataGrid sortable head cell, github.com/rust-ui/ui, MIT, notice kept in NOTICE-rust-ui).
    fn by(&self, column: &str) -> Order {
        let again = self.sort.as_deref() == Some(column);
        Order {
            filter: self.filter.clone(),
            sort: Some(column.into()),
            desc: again && !self.desc,
            page: 1,
        }
    }
}

/// A cell as a number, when it is written as one: digits first, so « inf » and « NaN » stay words.
fn number(cell: &str) -> Option<f64> {
    let cell = cell.trim();
    let digit = cell
        .trim_start_matches(['-', '+', '.'])
        .starts_with(|c: char| c.is_ascii_digit());
    cell.parse().ok().filter(|n: &f64| digit && n.is_finite())
}

/// Two cells compared as their column holds them (rust-ui's DataGrid compares each column by its own type): numbers by
/// value and before any words, words by their letters, case aside. A total order, so a column of « 2 », « 10 » and « 1a »
/// sorts instead of panicking (/code-review). ponytail: « 10 KB » is words; units parsed once a column needs them.
fn compared(a: &str, b: &str) -> std::cmp::Ordering {
    match (number(a), number(b)) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

/// A table as its order shows it: the shown page's rows, that page and the count of pages, the rows its filter keeps and
/// all of them.
pub struct Arranged {
    pub shown: Vec<TableRow>,
    pub page: usize,
    pub pages: usize,
    pub matched: usize,
    pub total: usize,
}

/// The rows a table shows for `order`: those holding its filter, sorted by its column, if it has one (stable, so ties keep
/// the page's order), then its page's slice; the page kept within the last.
pub fn arranged(columns: &[String], rows: &[TableRow], order: &Order, per_page: usize) -> Arranged {
    let total = rows.len();
    let words = order.filter.as_ref().map(|f| f.to_lowercase());
    // references sorted, the shown page's rows alone cloned (/code-review)
    let mut rows: Vec<&TableRow> = (rows.iter())
        .filter(|r| {
            words
                .as_ref()
                .is_none_or(|w| r.cells.iter().any(|c| c.to_lowercase().contains(w)))
        })
        .collect();
    if let Some(j) = order
        .sort
        .as_ref()
        .and_then(|s| columns.iter().position(|c| c == s))
    {
        // reversed by its comparison, never by the list, so ties keep the page's order both ways (/code-review)
        rows.sort_by(|a, b| match order.desc {
            false => compared(&a.cells[j], &b.cells[j]),
            true => compared(&b.cells[j], &a.cells[j]),
        });
    }
    let (matched, per_page) = (rows.len(), per_page.max(1));
    let pages = matched.div_ceil(per_page).max(1);
    let page = order.page.clamp(1, pages);
    let shown = rows
        .into_iter()
        .skip((page - 1) * per_page)
        .take(per_page)
        .cloned()
        .collect();
    Arranged {
        shown,
        page,
        pages,
        matched,
        total,
    }
}

impl Arranged {
    /// What both forms say first of a table (its caption, the text's first line): its count, its filter, its sort and its
    /// page among several (task design-agent-forms).
    pub fn line(&self, columns: &[String], order: &Order) -> String {
        let rows = |n: usize| match n {
            1 => "1 row".to_string(),
            n => format!("{n} rows"),
        };
        let mut parts = vec![match &order.filter {
            Some(words) => format!("{} of {} match « {words} »", self.matched, rows(self.total)),
            None => rows(self.total),
        }];
        if let Some(sort) = order.sort.as_ref().filter(|s| columns.contains(s)) {
            let way = if order.desc {
                "descending"
            } else {
                "ascending"
            };
            parts.push(format!("sorted by {sort}, {way}"));
        }
        if self.pages > 1 {
            parts.push(format!("page {} of {}", self.page, self.pages));
        }
        parts.join("; ") + "."
    }

    /// The next page's URL, past the line, so an agent reads a long table a page at a time: relative to the page read, as
    /// every link of it is. The HTML keeps it for its text alone (`.sr-only`): the pager shows him the way.
    pub fn next(&self, order: &Order, query: &str) -> Option<String> {
        (self.page < self.pages).then(|| {
            Order {
                page: self.page + 1,
                ..order.clone()
            }
            .href(query)
        })
    }
}

inventory::submit! {
    Component {
        slug: "table",
        classes: &["table"],
        tags: &["table"],
        name: "Table",
        used: "The comment queue at /comments, /settings' stores: a head click sorts by its column, again reverses it; past its rows a page, Pagination pages it and a field filters it, all in the URL; its caption, the text's first line too, says the total, and the text the next page's URL.",
        example: Some(|query| {
            let row = |cells: [&str; 3], href: Option<&str>| TableRow {
                cells: cells.map(String::from).to_vec(),
                href: href.map(String::from),
            };
            rsx! {
                Table { id: "c-table", columns: vec!["Comment".into(), "Page".into(), "Since".into()],
                    rows: vec![
                        row(["a table, not cards", "/comments", "2026-11-09 10:00"], Some("/comments#c-a")),
                        row(["the old app's", "/x", "2026-11-09 11:00"], None),
                        row(["sortable, design system first", "/settings", "2026-11-09 09:30"], Some("/settings")),
                        row(["Agent view reads it", "/design", "2026-11-10 08:15"], Some("/design")),
                        row(["a pager for long queues", "/comments", "2026-11-10 12:40"], None),
                    ],
                    query: query.to_string(), per_page: 3,
                }
            }
        }),
        formed: true,
        sections: &[Section {
            family: "data",
            id: "table",
            title: "Table and filter bar",
            after: "data",
            rows: &[
                // three a page, so its filter field and pager show: ?filter=applied&page=2 as an agent asks it
                ("Leads, filtered and paged", |query| table_rows("ds-leads", LEADS, Some(query), 3)),
                // unqueried: one filter field a page, the leads' (/code-review)
                ("Empty", |_| table_rows("ds-no-lead", &[], None, 3)),
            ],
        }],
        ..Component::BARE
    }
}

/// A queue or a list of records, a row each (task rust-comments-table; @nha, 2026-11-09: « there is nothing card-like to
/// comments. they are a queue. a table. »): `columns` its head. A row's first cell is its link, kept only where a card's
/// would be (`card_href`), which takes the whole row's tap (components.css); its text form a record per row, its fields a
/// tab apart. Each head is a link that sorts by its column, again reverses it, `aria-sort` on the one sorted; past
/// `per_page` rows, Pagination pages it and a filter field above keeps the rows holding its words; all in the URL (`query`,
/// the page's), and its caption says them with the total, the next page's URL for the text (`Arranged::line`, `next`).
/// With no `query` (a form's answer, whose links would GET its POST path), every row under plain heads (/code-review).
#[component]
pub fn Table(
    #[props(into)] id: String,
    columns: Vec<String>,
    rows: Vec<TableRow>,
    #[props(default, into)] query: Option<String>,
    #[props(default = PER_PAGE)] per_page: usize,
) -> Element {
    let order = Order::of(query.as_deref().unwrap_or_default());
    let per_page = query.as_ref().map_or(rows.len(), |_| per_page);
    let arranged = arranged(&columns, &rows, &order, per_page);
    let line = query.as_ref().map(|_| arranged.line(&columns, &order));
    let next = query.as_ref().and_then(|q| arranged.next(&order, q));
    let Arranged {
        ref shown,
        page,
        pages,
        ..
    } = arranged;
    // a GET form drops the URL's query: every other key rides along hidden, the page back to the first
    let kept: Vec<(String, String)> = query.as_deref().map_or(vec![], |q| {
        pairs(q)
            .filter(|(k, _)| !["filter", "page"].contains(k))
            .map(|(k, v)| (k.to_string(), v))
            .collect()
    });
    let filtered = query.is_some() && (rows.len() > per_page || order.filter.is_some());
    let cell = |row: &TableRow, j: usize, text: &str| match (j, card_href(&row.href)) {
        (0, Some(href)) => rsx! { a { href: "{href}", "{text}" } },
        _ => rsx! { "{text}" },
    };
    let head = columns.iter().map(|c| {
        let sorted = (order.sort.as_ref() == Some(c)).then_some(if order.desc {
            "descending"
        } else {
            "ascending"
        });
        match &query {
            Some(query) => {
                let href = order.by(c).href(query);
                rsx! { th { scope: "col", "aria-sort": sorted, a { href, "{c}" } } }
            }
            None => rsx! { th { scope: "col", "{c}" } },
        }
    });
    rsx! {
        if filtered {
            form { class: "table-filter", method: "get", role: "search",
                for (k, v) in kept { input { r#type: "hidden", name: k, value: v } }
                input { class: "input", r#type: "search", name: "filter", value: order.filter.clone().unwrap_or_default(),
                    placeholder: "Filter rows", "aria-label": "Filter the table" }
            }
        }
        table { id: "{id}", class: "table",
            if let Some(line) = line {
                caption { "{line}"
                    if let Some(next) = next { span { class: "sr-only", " Next page: {next}" } }
                }
            }
            thead { tr { {head} } }
            tbody {
                for row in shown.iter() {
                    tr {
                        for (j, text) in row.cells.iter().enumerate() { td { {cell(row, j, text)} } }
                    }
                }
            }
        }
        if pages > 1 {
            Pagination { id: "{id}-pages", query: query.unwrap_or_default(), page, pages }
        }
    }
}

inventory::submit! {
    Component {
        slug: "pagination",
        classes: &["pagination-ellipsis", "pagination", "pagination-content", "pagination-link"],
        tags: &[],
        ..Component::BARE
    }
}

/// The pages of a long list: DioxusLabs/dioxus-components' Pagination (MIT OR Apache-2.0, notice kept in
/// NOTICE-dioxus-components), its markup kept: a navigation of links, the shown one `aria-current`, Previous and Next where
/// there is one, the first, the last and the shown one's neighbours, an ellipsis between. Each link keeps the rest of
/// `query`, so the sort holds across pages.
#[component]
pub fn Pagination(
    #[props(into)] id: String,
    #[props(into)] query: String,
    page: usize,
    pages: usize,
) -> Element {
    let order = Order::of(&query);
    let at = |p: usize| {
        Order {
            page: p,
            ..order.clone()
        }
        .href(&query)
    };
    let mut items = vec![];
    if page > 1 {
        let href = at(page - 1);
        items.push(rsx! { li { a { class: "pagination-link", href, "aria-label": "Go to previous page", "data-kind": "previous", "Previous" } } });
    }
    let mut last = 0;
    for p in (1..=pages).filter(|&p| p == 1 || p == pages || p.abs_diff(page) <= 1) {
        if p > last + 1 {
            items.push(
                rsx! { li { span { class: "pagination-ellipsis", "aria-hidden": "true", "…" } } },
            );
        }
        let (href, current) = (at(p), (p == page).then_some("page"));
        items.push(rsx! { li { a { class: "pagination-link", href, "aria-current": current, "data-active": p == page, "{p}" } } });
        last = p;
    }
    if page < pages {
        let href = at(page + 1);
        items.push(rsx! { li { a { class: "pagination-link", href, "aria-label": "Go to next page", "data-kind": "next", "Next" } } });
    }
    rsx! {
        nav { id: "{id}", class: "pagination", role: "navigation", "aria-label": "pagination",
            ul { class: "pagination-content", {items.into_iter()} }
        }
    }
}

/// The leads as a Table, in the page's order (`query`, none for every row under plain heads), `per_page` rows a page; the
/// empty line when no row is left, the filter's doing included (/code-review).
pub(crate) fn table_rows(
    id: &'static str,
    rows: &[(&str, &str, &str)],
    query: Option<&str>,
    per_page: usize,
) -> Element {
    let columns = ["Company", "Stage", "Rate"].map(String::from).to_vec();
    let rows: Vec<TableRow> = (rows.iter())
        .map(|&(a, b, c)| TableRow {
            cells: [a, b, c].map(String::from).to_vec(),
            href: None,
        })
        .collect();
    let order = Order::of(query.unwrap_or_default());
    let empty = arranged(&columns, &rows, &order, per_page).matched == 0;
    let query = query.map(String::from);
    rsx! {
        Table { id, columns, rows, query, per_page }
        if empty { div { class: "ds-empty", "No lead matches this filter." } }
    }
}

/// The Table section's leads.
const LEADS: &[(&str, &str, &str)] = &[
    ("Northwind", "Applied", "600"),
    ("Globex", "Screening", "700"),
    ("Umbrella Labs", "Interviewing", "650"),
    ("Initech", "Applied", "550"),
    ("Hooli", "Signed", "800"),
    ("Acme", "Screening", "620"),
    ("Stark", "Applied", "750"),
    ("Wayne", "Applied", "680"),
];
