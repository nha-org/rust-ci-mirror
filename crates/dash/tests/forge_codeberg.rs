//! Task rust-forge-codeberg, done-when 2: `dash land`'s core and a reviewer's verdict on a Codeberg PR, as on a GitHub one. A
//! Forgejo standing in for codeberg.org answers the calls `dash_forge::pull::Landing` makes; `dash_orch::land::land` runs on
//! those reads as the binary's `land_pr` hands them, and the reviewer's brief and verdict go through the same `Landing`.
use dash_core::tasks::{Pr, Review, review_needed, verdict_comment};
use dash_forge::forgejo::Forgejo;
use dash_forge::pull::Landing;
use dash_meter::dash_test;
use dash_orch::land::{LandReads, Landed, land};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

const URL: &str = "https://codeberg.org/whatsnr/whatsnr-webapp/pulls/12";
const HEAD: &str = "4f2a9c1e";

/// PR 12 of whatsnr/whatsnr-webapp at `HEAD`, its Forgejo Actions run green; comments and POSTs kept in `seen`.
fn codeberg(seen: Rc<RefCell<Vec<String>>>) -> Landing {
    let comments = Rc::new(RefCell::new(Vec::<Value>::new()));
    let call = move |method: &str, path: &str, body: Option<&str>| {
        let body: Value = body.map_or(Value::Null, |b| serde_json::from_str(b).expect("json"));
        if method == "POST" {
            seen.borrow_mut().push(format!("{method} {path} {body}"));
        }
        let answer = match (method, path) {
            ("GET", "pulls/12") => json!({"number": 12, "title": "[feat][whatsnr-map] Map pins",
                "mergeable": true, "head": {"sha": HEAD, "ref": "task/whatsnr-map", "repo_id": 7},
                "base": {"ref": "main", "repo_id": 7}}),
            ("GET", "branches/task/whatsnr-map") => json!({"commit": {"id": HEAD}}),
            ("GET", p) if p == format!("commits/{HEAD}/status?page=1&limit=50") => {
                json!({"state": "success",
                "statuses": [{"context": "ci / test (pull_request)", "status": "success"}]})
            }
            ("GET", "pulls/12/files?page=1&limit=50") => json!([{"filename": "src/map.js"}]),
            ("GET", "issues/12/comments?page=1&limit=50") => {
                Value::Array(comments.borrow().clone())
            }
            ("POST", "issues/12/comments") => {
                comments.borrow_mut().push(body);
                json!({})
            }
            ("POST", "pulls/12/merge") => json!({}),
            _ => return Ok((404, r#"{"message":"The target couldn't be found."}"#.into())),
        };
        Ok((200, answer.to_string()))
    };
    Landing::Codeberg {
        forge: Forgejo {
            repo: "whatsnr/whatsnr-webapp".into(),
            call: Box::new(call),
        },
        number: 12,
    }
}

/// `land_pr`'s `GhLand` on a Landing: its recheck is the review hold, the task flagged review true.
struct Reads<'a>(&'a Landing);

impl LandReads for Reads<'_> {
    fn heads(&mut self) -> Result<(String, Result<String, String>), String> {
        self.0.heads()
    }
    fn mergeable(&mut self) -> Result<String, String> {
        self.0.mergeable()
    }
    fn recheck(&mut self, head: &str) -> Result<(), String> {
        // the number as land_pr's land_checks reads it from the URL
        let number = dash_orch::land::pr_number(URL)?;
        let pr = Pr {
            number,
            url: URL.into(),
            head: head.into(),
            merged: false,
        };
        let why = review_needed(&self.0.sends(&pr)?, true).expect("held");
        match self.0.verdict(&pr)? {
            Some(Review::Pass(_)) => Ok(()),
            other => Err(format!("{why}: {other:?}")),
        }
    }
    fn pause(&mut self) {}
}

#[dash_test]
fn a_codeberg_pr_is_reviewed_then_squash_merged_by_land() {
    let seen = Rc::new(RefCell::new(vec![]));
    let at = codeberg(Rc::clone(&seen));
    let pr = Pr {
        number: 12,
        url: URL.into(),
        head: HEAD.into(),
        merged: false,
    };
    // the reviewer's pane is told to post its verdict on the PR's URL, which `dash review verdict` reads through Landing
    let brief = dash_orch::review_brief(
        "whatsnr-map",
        &at.title().expect("title"),
        &pr,
        "its task says review true",
    );
    assert!(
        brief.contains(&format!("dash review verdict {URL} pass")),
        "{brief}"
    );

    let land_it = |at: &Landing| {
        let mut said = vec![];
        let landed = land(
            URL,
            &at.title().expect("title"),
            &mut Reads(at),
            |_| match at.checks() {
                Ok(rows) if rows.iter().all(|(_, s)| s == "SUCCESS") => {
                    (0, String::new(), String::new())
                }
                other => (1, String::new(), format!("{other:?}")),
            },
            |head| at.gate(head),
            |url, head| {
                at.merge(&Pr {
                    number: dash_orch::land::pr_number(url)?,
                    url: URL.into(),
                    head: head.into(),
                    merged: false,
                })
            },
            |id, _, _| Ok(format!(" ({id})")),
            |_| at.files(),
            || at.deploy(),
            || Ok(String::new()),
            |_| {},
            |line| said.push(line.to_string()),
        );
        (landed, said)
    };
    // held: no reviewer's pass yet
    let (landed, _) = land_it(&at);
    let refused = landed.expect_err("held");
    assert!(
        refused.1.contains("its task says review true"),
        "{refused:?}"
    );
    assert!(
        seen.borrow().is_empty(),
        "nothing merged: {:?}",
        seen.borrow()
    );

    // the reviewer's pass at the head, as dash review verdict posts it, then land squash-merges that head
    at.comment(&verdict_comment(&Review::Pass("clean".into()), HEAD))
        .expect("posted");
    let (landed, said) = land_it(&at);
    assert_eq!(landed, Ok(Landed::Merged), "{said:#?}");
    let merge = format!(
        r#"POST pulls/12/merge {{"Do":"squash","delete_branch_after_merge":true,"head_commit_id":"{HEAD}"}}"#
    );
    assert_eq!(seen.borrow().last(), Some(&merge));
    assert!(
        said.contains(&"land: closed whatsnr-map (whatsnr-map)".to_string()),
        "{said:#?}"
    );
    assert!(
        said.iter()
            .any(|l| l.contains("dash-gate not run: it is site_perso's")),
        "{said:#?}"
    );
}
