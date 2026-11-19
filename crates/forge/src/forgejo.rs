//! Codeberg's forge: Forgejo's REST API (`/api/v1`, Gitea's shape) through curl, its token from `bin/secret get codeberg
//! CODEBERG_TOKEN` (task rust-forge-codeberg; @nha, 2026-11-14: « whatsnr has moved to codeberg »). What `dash land` and a
//! reviewer read and do on a Codeberg PR: its open PRs with head, checks and mergeability, a squash merge at a head, the
//! files, the verdict comment. Each call goes through `call`, so a test answers from recorded answers.
use crate::checks::Rows;
use dash_core::tasks::{Pr, Review, verdict_of};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

/// One API call: method, path under `/api/v1/repos/<owner>/<name>/`, JSON body. Its HTTP status and text; Err: no answer.
pub type Call = Box<dyn Fn(&str, &str, Option<&str>) -> Result<(u16, String), String>>;

const CALL: Duration = Duration::from_secs(60);

pub struct Forgejo {
    /// `owner/name`.
    pub repo: String,
    pub call: Call,
    /// Its checkout on this Mac, whose worktrees the landing's guard reads (`dash_forge::guard`): the projects' table's
    /// folder for `repo`, None when it names none, or in a test, which never reaches the live checkouts.
    pub checkout: Option<std::path::PathBuf>,
}

/// One open PR as the lane reads it.
#[derive(Debug, PartialEq)]
pub struct Open {
    pub pr: Pr,
    pub branch: String,
    pub title: String,
    pub draft: bool,
    /// Forgejo's own read: false while it conflicts with its base.
    pub mergeable: bool,
    pub checks: Rows,
}

/// A Codeberg PR's URL, `[http[s]://]codeberg.org/<owner>/<name>/pulls/<n>[/files…]`: its repository and number.
pub fn pr_at(url: &str) -> Option<(String, u64)> {
    let bare = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = bare.strip_prefix("codeberg.org/")?;
    let mut parts = rest.split('/');
    let (owner, name, pulls, n) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let n = n.split(['#', '?']).next()?.parse().ok()?;
    (pulls == "pulls").then(|| (format!("{owner}/{name}"), n))
}

impl Forgejo {
    /// On codeberg.org with `bin/secret`'s token, under `repo`, the checkout holding bin/secret.
    pub fn codeberg(checkout: &Path, repo: &str) -> Result<Self, String> {
        let secret = checkout.join("bin/secret").display().to_string();
        let args = ["get", "codeberg", "CODEBERG_TOKEN"];
        let token = dash_process::run(&secret, &args, None, Duration::from_secs(20))?;
        let token = token.trim().to_string();
        if token.is_empty() {
            return Err("no CODEBERG_TOKEN in bin/secret's codeberg".into());
        }
        let base = format!("https://codeberg.org/api/v1/repos/{repo}/");
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        // a checkout this Mac lacks holds no worktree: None, never a guard run in a missing folder (/code-review)
        let checkout = (home.zip(dash_core::remote_folder(repo)))
            .map(|(h, f)| h.join(f))
            .filter(|p| p.is_dir());
        Ok(Forgejo {
            checkout,
            repo: repo.into(),
            call: Box::new(move |method, path, body| {
                curl(&token, method, &format!("{base}{path}"), body)
            }),
        })
    }

    /// A 2xx answer's JSON; any other status is an Err naming it and Forgejo's message.
    fn ask(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
        let body = body.map(Value::to_string);
        let (status, text) = (self.call)(method, path, body.as_deref())?;
        if !(200..300).contains(&status) {
            let said: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let why = said["message"].as_str().unwrap_or(text.trim());
            return Err(format!("{method} {}/{path}: {status} {why}", self.repo));
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// Its open PRs, each with its head's checks. ponytail: the first 50, as no repository of ours holds more open.
    pub fn open(&self) -> Result<Vec<Open>, String> {
        let listed = self.ask("GET", "pulls?state=open&limit=50", None)?;
        let pulls = listed.as_array().ok_or("pulls: not a list")?;
        pulls.iter().map(|p| self.open_pr(p)).collect()
    }

    fn open_pr(&self, p: &Value) -> Result<Open, String> {
        let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
        let head = text(&p["head"]["sha"]);
        Ok(Open {
            checks: self.checks(&head)?,
            pr: Pr {
                number: p["number"].as_u64().ok_or("a PR with no number")?,
                url: text(&p["html_url"]),
                head,
                merged: p["merged"] == true,
            },
            branch: text(&p["head"]["ref"]),
            title: text(&p["title"]),
            draft: p["draft"] == true,
            mergeable: p["mergeable"] == true,
        })
    }

    /// `sha`'s commit statuses, Forgejo Actions' and any other runner's, in gh's words: `[context, SUCCESS|PENDING|...]`.
    pub fn checks(&self, sha: &str) -> Result<Rows, String> {
        let rows = self.pages(&format!("commits/{sha}/status"), Some("statuses"))?;
        Ok((rows.iter())
            .map(|s| {
                let state = match s["status"].as_str().unwrap_or_default() {
                    // Forgejo's combined state counts a warning as no failure
                    "warning" => "NEUTRAL".to_string(),
                    other => other.to_uppercase(),
                };
                (s["context"].as_str().unwrap_or_default().to_string(), state)
            })
            .collect())
    }

    /// Every item of a paged list, `PAGE` a page, the list under `key` when the answer is an object; none (`null`, a commit
    /// with no status) is an empty list (/code-review: Forgejo pages its lists, Codeberg at 50 at most).
    fn pages(&self, path: &str, key: Option<&str>) -> Result<Vec<Value>, String> {
        const PAGE: usize = 50;
        let mut all = vec![];
        for page in 1.. {
            let sep = if path.contains('?') { '&' } else { '?' };
            let answer = self.ask("GET", &format!("{path}{sep}page={page}&limit={PAGE}"), None)?;
            let items = key.map_or(&answer, |k| &answer[k]);
            let items = match items {
                Value::Null => vec![],
                Value::Array(items) => items.clone(),
                _ => return Err(format!("{path}: not a list")),
            };
            let last = items.len() < PAGE;
            all.extend(items);
            if last {
                break;
            }
        }
        Ok(all)
    }

    fn pull(&self, n: u64) -> Result<Value, String> {
        self.ask("GET", &format!("pulls/{n}"), None)
    }

    /// Its head and title, from one read.
    pub fn head_title(&self, n: u64) -> Result<(String, String), String> {
        let p = self.pull(n)?;
        let head = p["head"]["sha"].as_str().ok_or("a PR with no head")?;
        Ok((head.into(), p["title"].as_str().unwrap_or_default().into()))
    }

    /// Its head branch and its base branch by name, and whether its head is a fork's (another repository than its base's):
    /// what the landing's guard reads the checkout's worktrees by.
    pub fn branches(&self, n: u64) -> Result<(String, String, bool), String> {
        let p = self.pull(n)?;
        let name = |side: &str| p[side]["ref"].as_str().map(String::from);
        let (head, base) = (name("head").zip(name("base")))
            .ok_or_else(|| format!("PR {n} names no head or base branch"))?;
        let fork = p["head"]["repo_id"] != p["base"]["repo_id"];
        Ok((head, base, fork))
    }

    /// `LandReads::heads`: the PR's head and its branch's own head, read apart; a fork's branch is not ours to read.
    pub fn heads(&self, n: u64) -> Result<(String, Result<String, String>), String> {
        let p = self.pull(n)?;
        let head = (p["head"]["sha"].as_str())
            .ok_or("a PR with no head")?
            .to_string();
        if p["head"]["repo_id"] != p["base"]["repo_id"] {
            return Ok((head, Err("a fork's branch, not read at origin".into())));
        }
        let branch = p["head"]["ref"].as_str().unwrap_or_default();
        // git refuses a space or ? in a branch name, never # or %, which a URL path reads as its own
        let path = branch.replace('%', "%25").replace('#', "%23");
        let read = self.ask("GET", &format!("branches/{path}"), None);
        let at = read.and_then(|b| {
            (b["commit"]["id"].as_str().map(String::from)).ok_or_else(|| format!("no {branch}"))
        });
        Ok((head, at))
    }

    /// `LandReads::mergeable` in gh's words.
    pub fn mergeable(&self, n: u64) -> Result<String, String> {
        Ok(match self.pull(n)?["mergeable"] == true {
            true => "MERGEABLE".into(),
            false => "CONFLICTING".into(),
        })
    }

    /// A squash merge of exactly `pr.head`, its branch deleted. Forgejo's refusals in gh's words, as `dash_orch::land` reads
    /// them: a conflict « cannot be cleanly created », a moved head « Head branch was modified ». Any other refusal (405: still
    /// checking, a check required, a draft) is a plain failure, nothing merged (/code-review: never read as a conflict).
    pub fn merge(&self, pr: &Pr) -> Result<(), String> {
        let body =
            json!({"Do": "squash", "head_commit_id": pr.head, "delete_branch_after_merge": true});
        match self.ask("POST", &format!("pulls/{}/merge", pr.number), Some(&body)) {
            Ok(_) => Ok(()),
            // Forgejo's own words, after `<method> <repo>/<path>: `: a repository named conflict-x is no conflict
            Err(e)
                if (e.split_once(": ").map(|(_, why)| why.to_lowercase()))
                    .is_some_and(|why| why.contains("conflict")) =>
            {
                Err(format!("the merge commit cannot be cleanly created: {e}"))
            }
            Err(e) if e.contains(": 409 ") => Err(format!("Head branch was modified: {e}")),
            Err(e) => Err(e),
        }
    }

    /// Every file the PR changes, a moved one by its old name too, as GitHub's `sends` reads them.
    pub fn files(&self, n: u64) -> Result<Vec<String>, String> {
        let files = self.pages(&format!("pulls/{n}/files"), None)?;
        let names = files
            .iter()
            .flat_map(|f| [&f["filename"], &f["previous_filename"]]);
        Ok(names
            .filter_map(|name| name.as_str().filter(|s| !s.is_empty()).map(String::from))
            .collect())
    }

    /// The newest verdict comment, when it names `pr.head`. ponytail: one on an older head covers nothing, so main merged in
    /// after a pass needs a new one; GitHub's reads the commits between.
    pub fn verdict(&self, pr: &Pr) -> Result<Option<Review>, String> {
        let comments = self.pages(&format!("issues/{}/comments", pr.number), None)?;
        let last = (comments.iter().rev()).find_map(|c| verdict_of(c["body"].as_str()?));
        Ok(last.and_then(|(review, head)| (head == pr.head).then_some(review)))
    }

    pub fn comment(&self, n: u64, body: &str) -> Result<(), String> {
        let path = format!("issues/{n}/comments");
        self.ask("POST", &path, Some(&json!({"body": body})))
            .map(drop)
    }
}

/// One call by curl, the token in its environment, never its arguments; the status from curl's last line.
fn curl(token: &str, method: &str, url: &str, body: Option<&str>) -> Result<(u16, String), String> {
    let mut args = vec![
        "-sS",
        "-X",
        method,
        "--variable",
        "%CODEBERG_TOKEN",
        "--expand-header",
        "Authorization: token {{CODEBERG_TOKEN}}",
        "-H",
        "Accept: application/json",
        "-w",
        "\n%{http_code}",
    ];
    if body.is_some() {
        args.extend([
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
        ]);
    }
    args.push(url);
    let said = dash_process::answer_fed(
        "curl",
        &args,
        None,
        &[("CODEBERG_TOKEN", token)],
        body,
        CALL,
    );
    let (status, text, err) = said.map_err(String::from)?;
    let (text, code) = text
        .trim_end()
        .rsplit_once('\n')
        .unwrap_or(("", text.trim()));
    match (status.success(), code.parse()) {
        (true, Ok(code)) => Ok((code, text.to_string())),
        _ => Err(format!("curl {method} {url}: {}", err.trim())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Forgejo answering from tests/fixtures/forgejo, codeberg.org/forgejo/forgejo's answers recorded on 2026-11-14 and trimmed
    /// to the fields read (comments-15270.json's two verdicts added by hand after its one recorded comment); each call kept
    /// in `asked`, a POST answered 200 unless `refuse` says otherwise.
    fn recorded(asked: Rc<RefCell<Vec<String>>>, refuse: Option<(u16, &'static str)>) -> Forgejo {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/forgejo");
        let file = move |name: &str| {
            std::fs::read_to_string(dir.join(name)).map_err(|e| format!("{name}: {e}"))
        };
        Forgejo {
            checkout: None,
            repo: "forgejo/forgejo".into(),
            call: Box::new(move |method, path, body| {
                asked
                    .borrow_mut()
                    .push(format!("{method} {path} {}", body.unwrap_or_default()));
                let text = match (method, path) {
                    ("POST", _) => {
                        return Ok(refuse.map_or((200, "{}".into()), |(s, t)| (s, t.into())));
                    }
                    (_, "pulls?state=open&limit=50") => file("pulls.json")?,
                    (
                        _,
                        "commits/2c7efd830af0c1be3bce11e3d3441fd8aba21d18/status?page=1&limit=50",
                    ) => file("status-15270.json")?,
                    (
                        _,
                        "commits/97382279c40184b45a7bff798a52be59bce5924a/status?page=1&limit=50",
                    ) => file("status-15269.json")?,
                    (_, "commits/green/status?page=1&limit=50") => file("status-success.json")?,
                    (_, "commits/red/status?page=1&limit=50") => file("status-failure.json")?,
                    (_, "issues/15270/comments?page=1&limit=50") => file("comments-15270.json")?,
                    // 50 files on the first page, a send path moved away on the second
                    (_, "pulls/7/files?page=1&limit=50") => serde_json::to_string(
                        &(0..50).map(|i| json!({"filename": format!("src/{i}.js")})).collect::<Vec<_>>(),
                    )
                    .expect("json"),
                    (_, "pulls/7/files?page=2&limit=50") => {
                        r#"[{"filename":"old/jmap.rs","previous_filename":"rust/crates/mail/src/jmap.rs"}]"#.into()
                    }
                    (_, "commits/none/status?page=1&limit=50") => {
                        r#"{"state":"","statuses":null}"#.into()
                    }
                    _ => return Ok((404, r#"{"message":"The target couldn't be found."}"#.into())),
                };
                Ok((200, text))
            }),
        }
    }

    /// Done-when 1: the open PRs listed with head, checks and mergeability, from recorded answers.
    #[dash_test]
    fn open_prs_are_listed_with_their_head_checks_and_mergeability() {
        let forge = recorded(Rc::default(), None);
        let open = forge.open().expect("listed");
        let said: Vec<_> = open
            .iter()
            .map(|o| (o.pr.number, o.pr.head.as_str(), o.mergeable))
            .collect();
        assert_eq!(
            said,
            [
                (15270, "2c7efd830af0c1be3bce11e3d3441fd8aba21d18", true),
                (15269, "97382279c40184b45a7bff798a52be59bce5924a", false),
            ]
        );
        assert_eq!(
            open[0].pr.url,
            "https://codeberg.org/forgejo/forgejo/pulls/15270"
        );
        assert_eq!(open[0].branch, "fix-actions-id-token-url-subpath");
        assert_eq!(open[0].checks.len(), 18);
        assert!(open[0].checks.contains(&(
            "testing / test-unit (pull_request)".into(),
            "PENDING".into()
        )));
        assert!(open[0].checks.contains(&(
            "issue-labels / cascade (pull_request_target)".into(),
            "SKIPPED".into()
        )));
        let states = |sha| {
            forge
                .checks(sha)
                .expect("read")
                .into_iter()
                .map(|(_, s)| s)
                .collect::<Vec<_>>()
        };
        assert!(states("green").iter().all(|s| crate::green::is_green(s)));
        assert!(states("red").contains(&"FAILURE".to_string()));
    }

    /// Done-when 2: a squash merge names the head its checks covered; Forgejo's refusals read as gh's, which land acts on.
    #[dash_test]
    fn a_merge_squashes_at_the_head_and_its_refusals_read_as_gh_s() {
        let pr = Pr {
            number: 15270,
            url: String::new(),
            head: "abc".into(),
            merged: false,
        };
        let asked = Rc::default();
        recorded(Rc::clone(&asked), None)
            .merge(&pr)
            .expect("merged");
        assert_eq!(
            asked.borrow().as_slice(),
            [
                r#"POST pulls/15270/merge {"Do":"squash","delete_branch_after_merge":true,"head_commit_id":"abc"}"#
            ]
        );
        let refused = |answer| {
            recorded(Rc::default(), Some(answer))
                .merge(&pr)
                .unwrap_err()
        };
        assert!(
            refused((409, r#"{"message":"head out of date"}"#))
                .contains("Head branch was modified")
        );
        assert!(
            refused((409, r#"{"message":"merge failed because of conflict"}"#))
                .contains("cannot be cleanly created")
        );
        // the word in the repository's name is no conflict: only Forgejo's message reads
        let named = Forgejo {
            checkout: None,
            repo: "me/conflict-tool".into(),
            call: Box::new(|_, _, _| Ok((403, r#"{"message":"forbidden"}"#.into()))),
        };
        assert!(
            !named
                .merge(&pr)
                .unwrap_err()
                .contains("cannot be cleanly created")
        );
        let later = refused((405, r#"{"message":"Please try again later"}"#));
        assert!(
            !later.contains("cannot be cleanly created") && later.contains("try again later"),
            "{later}"
        );
        // a head with no status yet is no check, never a read error
        assert_eq!(recorded(Rc::default(), None).checks("none"), Ok(vec![]));
    }

    /// The review hold's read: every page of files, a moved one by its old name, so a send path past the first 50 holds it.
    #[dash_test]
    fn files_past_the_first_page_and_old_names_reach_the_send_paths() {
        let files = recorded(Rc::default(), None).files(7).expect("listed");
        assert_eq!(files.len(), 52);
        assert_eq!(
            crate::sends(files.iter().map(String::as_str)),
            ["rust/crates/mail/src/jmap.rs"]
        );
    }

    /// Done-when 2: the verdict comment `dash review verdict` posts is read back at the head it names, never another.
    #[dash_test]
    fn the_newest_verdict_comment_covers_only_the_head_it_names() {
        let forge = recorded(Rc::default(), None);
        let at = |head: &str| Pr {
            number: 15270,
            url: String::new(),
            head: head.into(),
            merged: false,
        };
        assert_eq!(
            forge.verdict(&at("h2")).expect("read"),
            Some(Review::Fix("the test is missing".into()))
        );
        assert_eq!(forge.verdict(&at("h1")).expect("read"), None);
    }

    #[dash_test]
    fn a_codeberg_pr_url_names_its_repository_and_number() {
        assert_eq!(
            pr_at("https://codeberg.org/whatsnr/whatsnr-webapp/pulls/12"),
            Some(("whatsnr/whatsnr-webapp".into(), 12))
        );
        assert_eq!(
            pr_at("https://codeberg.org/whatsnr/whatsnr-webapp/pulls/12#issuecomment-3"),
            Some(("whatsnr/whatsnr-webapp".into(), 12))
        );
        // its tabs, plain http or no scheme name the same PR, never a GitHub one (/code-review of #1623)
        for url in [
            "https://codeberg.org/whatsnr/whatsnr-webapp/pulls/12/files",
            "http://codeberg.org/whatsnr/whatsnr-webapp/pulls/12",
            "codeberg.org/whatsnr/whatsnr-webapp/pulls/12/",
        ] {
            assert_eq!(
                pr_at(url),
                Some(("whatsnr/whatsnr-webapp".into(), 12)),
                "{url}"
            );
        }
        assert_eq!(pr_at("https://github.com/nha-org/site_perso/pull/12"), None);
        assert_eq!(
            pr_at("https://codeberg.org/whatsnr/whatsnr-webapp/issues/12"),
            None
        );
    }
}
