//! `dash task add --impact`: what a Rust task retires and meets, counted when it is filed (task rust-task-impact; @nha,
//! 2026-11-12: « do we need something like "expected impact" or something ? for example /documents allows moving documents to
//! rust. which is a big deal imo. »). Each item is an Elixir module (`Dashboard.Documents`) or a route of the older app's router
//! (`/documents`), whose file's lines are counted, or a row of rfc-rust-unified's section 20d by its number (`10`, `3a`,
//! `20b`). `none` alone names a task that retires and meets nothing. `dash orch next` orders equal priorities by it.
use dash_core::tasks::Impact;
use std::path::{Path, PathBuf};

/// The older app's code, its modules; its router is `parity`'s.
const LIB: &str = "private/dashboard/lib";
use crate::parity::ROUTER;

use crate::layers::IMPACT as HOW;

/// `text` resolved against the repo at `repo`: each module or route to its file and lines, each row to the 20d table; or why
/// not, naming the item and the closest choices.
pub fn counted(repo: &Path, text: &str) -> Result<Impact, String> {
    // `none` reads no repo: the common add pays no tree scan (/code-review)
    match items(text)[..] {
        ["none"] => Ok(Impact::default()),
        _ => Older::read(repo).counted(text),
    }
}

/// The items `--impact` names, apart by spaces or commas.
fn items(text: &str) -> Vec<&str> {
    (text.split(|c: char| c.is_whitespace() || c == ','))
        .filter(|w| !w.is_empty())
        .collect()
}

/// What an impact is checked against, read once: the older app's modules with their file's lines, its routes, and
/// rfc-rust-unified.
pub struct Older {
    modules: Vec<(String, PathBuf, u32)>,
    routes: Vec<crate::parity::Route>,
    rfc: String,
    rows: Vec<String>,
}

impl Older {
    pub fn read(repo: &Path) -> Older {
        let rfc = std::fs::read_to_string(repo.join(crate::layers::RFCS[0])).unwrap_or_default();
        Older {
            modules: modules(repo),
            routes: crate::parity::routes(
                &std::fs::read_to_string(repo.join(ROUTER)).unwrap_or_default(),
            ),
            rows: dash_surface::documents::conformance_rows(&rfc),
            rfc,
        }
    }

    /// `none` is answered by the free `counted` before any read.
    fn counted(&self, text: &str) -> Result<Impact, String> {
        let items = items(text);
        if items.is_empty() {
            return Err(format!("--impact names nothing: {HOW}"));
        }
        // a repo read wrong says so, never that each item is unknown (/code-review)
        if self.rows.is_empty() {
            return Err(format!(
                "section 20d's rows could not be read from {}",
                crate::layers::RFCS[0]
            ));
        }
        if self.modules.is_empty() {
            return Err(format!(
                "the older app's modules could not be read under {LIB}"
            ));
        }
        let mut impact = Impact::default();
        let mut files: Vec<(&PathBuf, u32)> = vec![];
        for item in items {
            if Impact::is_row(item) {
                if !self.rows.iter().any(|r| r == item) {
                    return Err(format!(
                        "{item} is no row of rfc-rust-unified's section 20d, whose rows are {}",
                        self.rows.join(" ")
                    ));
                }
                if !impact.rows.iter().any(|r| r == item) {
                    impact.rows.push(item.into());
                }
                continue;
            }
            let named = match item.starts_with('/') {
                // each route under its scopes' paths and aliases, whole (/code-review)
                true => {
                    let mut all: Vec<String> = (self.routes.iter())
                        .filter(|r| r.path == item)
                        .map(|r| r.module.clone())
                        .collect();
                    all.sort();
                    all.dedup();
                    all
                }
                false => vec![item.to_string()],
            };
            if named.is_empty() {
                return Err(format!(
                    "{item} is no route of the older app's router ({ROUTER})"
                ));
            }
            for module in &named {
                // its full name, else the one module whose name ends with it: two are refused, named (/code-review)
                let ends = |m: &String| m.ends_with(&format!(".{module}"));
                let exact: Vec<_> = self.modules.iter().filter(|(m, ..)| m == module).collect();
                let found = match exact.is_empty() {
                    true => self.modules.iter().filter(|(m, ..)| ends(m)).collect(),
                    false => exact,
                };
                let file = match found[..] {
                    [(_, file, lines)] => (file, *lines),
                    [] => return Err(format!("{module} is no module of the older app ({LIB})")),
                    _ => {
                        let all: Vec<&str> = found.iter().map(|(m, ..)| m.as_str()).collect();
                        return Err(format!(
                            "{module} names {}: write the one it retires",
                            all.join(" and ")
                        ));
                    }
                };
                if !files.contains(&file) {
                    files.push(file);
                }
            }
            if !impact.retires.iter().any(|r| r == item) {
                impact.retires.push(item.into());
            }
        }
        impact.lines = files.iter().map(|(_, lines)| lines).sum();
        Ok(impact)
    }

    /// What an open Rust task's impact may be, from its words and its layer: each older-app module its title, why or
    /// done-when names, and the 20d row of its layer's section; `none` when neither names one. What `dash task style`
    /// proposes for a task with no impact kept, never written by itself.
    pub fn guessed(&self, words: &str, layer: Option<&str>) -> String {
        // its own, not `folds::MODULE`: folds calls this module, and the gate refuses a loop
        static NAMED: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(r"\b(?:Dashboard|DashboardWeb|PublicSite)(?:\.[A-Z]\w*)+")
                .expect("a regex")
        });
        let mut items: Vec<String> = vec![];
        for m in NAMED.find_iter(words).map(|m| m.as_str().to_string()) {
            if self.modules.iter().any(|(d, ..)| *d == m) && !items.contains(&m) {
                items.push(m);
            }
        }
        let row = (layer.and_then(|l| l.strip_prefix("rfc:")))
            .and_then(|id| dash_surface::documents::section_number(&self.rfc, id))
            .filter(|n| self.rows.contains(n));
        items.extend(row);
        match items.is_empty() {
            true => "none".into(),
            false => items.join(" "),
        }
    }
}

/// Each module the older app defines, with its file and the file's lines, counted in the one read.
fn modules(repo: &Path) -> Vec<(String, PathBuf, u32)> {
    let mut found = vec![];
    let mut folders = vec![repo.join(LIB)];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|e| e == "ex") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let lines = text.lines().count() as u32;
                for line in text.lines() {
                    let name = line.trim_start().strip_prefix("defmodule ");
                    if let Some(name) = name.and_then(|n| n.strip_suffix(" do")) {
                        found.push((name.trim().to_string(), path.clone(), lines));
                    }
                }
            }
        }
    }
    // read_dir's order is the disk's: sorted, so every machine reads the same
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;

    /// A repo holding a router, two modules and rfc-rust-unified.
    fn repo() -> Option<dash_meter::TestDir> {
        // the real RFC, which the public mirror lacks: this crate holds no HTML (dash-gate's the_dash_crate_holds_no_html)
        let rfc = dash_meter::monorepo()?.join(crate::layers::RFCS[0]);
        let dir = dash_meter::TestDir::new("impact");
        let put = |path: &str, text: &str| {
            let path = dir.0.join(path);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("folders");
            std::fs::write(path, text).expect("written");
        };
        put(
            ROUTER,
            "defmodule DashboardWeb.Router do\n  scope \"/\", DashboardWeb do\n    live \"/documents\", DocumentsLive\n    \
             get \"/documents/:id\", DocumentController, :show\n  end\n  \
             scope \"/admin\", DashboardWeb.Admin do\n    live \"/documents\", DocumentsLive\n  end\nend\n",
        );
        put(
            "private/dashboard/lib/dashboard_web/live/documents_live.ex",
            "defmodule DashboardWeb.DocumentsLive do\n  1\n  2\nend\n",
        );
        put(
            "private/dashboard/lib/dashboard/documents.ex",
            "defmodule Dashboard.Documents do\nend\n",
        );
        put(
            crate::layers::RFCS[0],
            &std::fs::read_to_string(rfc).expect("rfc-rust-unified"),
        );
        Some(dir)
    }

    /// Done-when 1 of rust-task-impact: the modules and routes it retires and the 20d rows it meets, counted; an unknown one
    /// refused, named; `none` alone is a task that retires and meets nothing.
    #[test]
    fn an_impact_counts_the_elixir_lines_it_retires_and_the_20d_rows_it_meets() {
        let _budget = TestBudget::new();
        let Some(repo) = repo() else {
            return;
        };
        let repo = repo.0.as_path();
        let older = Older::read(repo);
        assert!(
            older
                .rows
                .starts_with(&["1".into(), "2".into(), "3".into(), "3a".into()]),
            "{:?}",
            older.rows
        );
        let impact = counted(repo, "/documents, Dashboard.Documents 10 3a").expect("counted");
        assert_eq!(impact.retires, ["/documents", "Dashboard.Documents"]);
        // DocumentsLive's 4 lines and Documents' 2: the route's module found whole, under its scope's alias
        assert_eq!(impact.lines, 6);
        assert_eq!(impact.rows, ["10", "3a"]);
        assert_eq!(
            impact.said(),
            "retires 6 Elixir lines (/documents, Dashboard.Documents), meets 2 20d row(s) (10, 3a)"
        );
        // a module named twice, by its route and its own name, counts once
        let twice = counted(repo, "/documents DashboardWeb.DocumentsLive").expect("counted");
        assert_eq!(twice.lines, 4);
        assert_eq!(counted(repo, "none"), Ok(Impact::default()));
        let refused = |text: &str| counted(repo, text).expect_err(text);
        assert!(refused("").starts_with("--impact names nothing"));
        assert!(
            refused("3z").contains("rows are 1 2 3 3a 3b 4"),
            "{}",
            refused("3z")
        );
        assert!(refused("/nowhere").starts_with("/nowhere is no route"));
        assert!(refused("Dashboard.Nothing").starts_with("Dashboard.Nothing is no module"));
        // two modules end with the short name: refused, both named
        std::fs::write(
            repo.join("private/dashboard/lib/dashboard_web/live/admin.ex"),
            "defmodule DashboardWeb.Admin.DocumentsLive do\nend\n",
        )
        .expect("written");
        // a scoped route is its whole path, its module its scopes' aliases joined
        assert_eq!(counted(repo, "/admin/documents").expect("scoped").lines, 2);
        let both = refused("DocumentsLive");
        assert!(
            both.contains("names DashboardWeb.Admin.DocumentsLive and DashboardWeb.DocumentsLive"),
            "{both}"
        );
        // a repo read wrong says so, never that each item is unknown
        let empty = dash_meter::TestDir::new("impact-empty");
        let wrong = counted(&empty.0, "10").expect_err("no repo");
        assert!(
            wrong.starts_with("section 20d's rows could not be read"),
            "{wrong}"
        );
        // what `dash task style` proposes: the modules its words name, its layer's row
        let words = "Port Dashboard.Documents and Dashboard.Gone to Rust";
        assert_eq!(
            older.guessed(words, Some("rfc:ui-lib")),
            "Dashboard.Documents 10"
        );
        assert_eq!(
            older.guessed("nothing", Some("rfc:organising-work")),
            "none"
        );
    }
}
