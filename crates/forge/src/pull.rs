//! The one PR `dash land` and `dash review verdict` act on: site_perso's on GitHub through gh, or a Codeberg one through Forgejo
//! (task rust-forge-codeberg), told apart by its URL. The binary's land hands these reads to `dash_orch::land` as they are.
use crate::forgejo::{self, Forgejo};
use crate::{GitHub, Rows, red::Red};
use dash_core::tasks::{Forge, MergedGate, Pr, Review};
use dash_process as process;
use std::time::Duration;

const CALL: Duration = Duration::from_secs(60);

/// gh's `pr <verb>` on `pr`, then `args`, through the forge's own gh in its repository, as every GitHub read is (`GitHub::gh`).
fn gh(forge: &GitHub, pr: &str, args: &[&str]) -> Result<String, String> {
    forge.gh(&[&args[..2], &[pr][..], &args[2..]].concat())
}

pub enum Landing {
    /// `pr` as `dash land` was given it: a number, `#n`, its URL or its branch.
    GitHub {
        forge: GitHub,
        pr: String,
    },
    Codeberg {
        forge: Forgejo,
        number: u64,
    },
}

impl Landing {
    /// A codeberg.org PR URL through Forgejo, its token read now; any other word through `github`.
    pub fn of(github: GitHub, pr: &str) -> Result<Self, String> {
        Ok(match forgejo::pr_at(pr) {
            Some((repo, number)) => Landing::Codeberg {
                forge: Forgejo::codeberg(&github.repo, &repo)?,
                number,
            },
            // never handed to gh, which would read a bare number as site_perso's PR
            None if pr.contains("codeberg.org") => return Err(format!("{pr}: no Codeberg PR URL")),
            None => Landing::GitHub {
                forge: github,
                pr: pr.into(),
            },
        })
    }

    /// Its title, which names its task: `dash review verdict`'s own-session guard refuses on an unread one (/code-review).
    pub fn title(&self) -> Result<String, String> {
        let said = match self {
            Landing::GitHub { forge, pr } => gh(
                forge,
                pr,
                &["pr", "view", "--json", "title", "-q", ".title"],
            ),
            Landing::Codeberg { forge, number } => forge.head_title(*number).map(|t| t.1),
        };
        said.map(|t| t.trim().to_string())
    }

    /// Its head and title from one read, all `dash review verdict` needs.
    pub fn head_title(&self) -> Result<(String, String), String> {
        match self {
            Landing::GitHub { forge, pr } => {
                let q = ".headRefOid + \" \" + .title";
                let read = gh(
                    forge,
                    pr,
                    &["pr", "view", "--json", "headRefOid,title", "-q", q],
                )?;
                let (head, title) = read.trim().split_once(' ').unwrap_or((read.trim(), ""));
                Ok((head.into(), title.into()))
            }
            Landing::Codeberg { forge, number } => forge.head_title(*number),
        }
    }

    /// `LandReads::heads`: the PR's head, and its branch's read at origin, which sees a push at once where the PR may not yet.
    pub fn heads(&self) -> Result<(String, Result<String, String>), String> {
        let (forge, pr) = match self {
            Landing::Codeberg { forge, number } => return forge.heads(*number),
            Landing::GitHub { forge, pr } => (forge, pr),
        };
        let q = ".headRefOid + \" \" + (.isCrossRepository | tostring) + \" \" + .headRefName";
        let read = gh(
            forge,
            pr,
            &[
                "pr",
                "view",
                "--json",
                "headRefOid,isCrossRepository,headRefName",
                "-q",
                q,
            ],
        )?;
        let mut words = read.split_whitespace();
        let (Some(head), Some(fork), Some(branch)) = (words.next(), words.next(), words.next())
        else {
            return Err(format!("no head in {:?}", read.trim()));
        };
        // a fork's branch is not origin's: a branch of the same name there is another commit
        if fork == "true" {
            return Ok((
                head.to_string(),
                Err("a fork's branch, not read at origin".into()),
            ));
        }
        let at = format!("refs/heads/{branch}");
        let branch = process::run(
            "git",
            &["ls-remote", "origin", &at],
            Some(&forge.repo),
            CALL,
        )
        .and_then(|out| {
            (out.split_whitespace().next().map(String::from))
                .ok_or_else(|| format!("no {at} at origin"))
        });
        Ok((head.to_string(), branch))
    }

    /// `LandReads::mergeable`, in gh's words.
    pub fn mergeable(&self) -> Result<String, String> {
        match self {
            Landing::GitHub { forge, pr } => {
                let said = gh(
                    forge,
                    pr,
                    &["pr", "view", "--json", "mergeable", "-q", ".mergeable"],
                );
                said.map(|s| s.trim().to_string())
            }
            Landing::Codeberg { forge, number } => forge.mergeable(*number),
        }
    }

    /// Its head's checks, as `dash ci wait` reads them.
    pub fn checks(&self) -> Result<Rows, String> {
        match self {
            Landing::GitHub { forge, pr } => crate::read_checks(&forge.repo, &forge.repo, pr),
            Landing::Codeberg { forge, number } => {
                let head = forge.head_title(*number)?.0;
                forge.checks(&head)
            }
        }
    }

    /// Each red of `names` explained. ponytail: none on Codeberg, so each of its reds is a verdict on the code; an infra red
    /// read there once one comes.
    pub fn reds(&self, names: &[&str]) -> Result<Vec<Red>, String> {
        match self {
            Landing::GitHub { forge, pr } => crate::reds_in(&forge.repo, pr, names),
            Landing::Codeberg { .. } => Ok(vec![]),
        }
    }

    /// dash-gate's tests on its merge with main: site_perso's alone, as no other repository holds dash-gate.
    pub fn gate(&self, head: &str) -> Result<MergedGate, String> {
        match self {
            Landing::GitHub { forge, pr } => forge.gate(pr, head),
            Landing::Codeberg { forge, .. } => Ok(MergedGate::Passed(format!(
                "dash-gate not run: it is site_perso's, and this PR is {}'s",
                forge.repo
            ))),
        }
    }

    /// A squash merge of exactly `pr.head`.
    pub fn merge(&self, pr: &Pr) -> Result<(), String> {
        match self {
            Landing::GitHub { forge, .. } => forge.merge(pr),
            Landing::Codeberg { forge, .. } => forge.merge(pr),
        }
    }

    /// Every file it changes (`gh pr view --json files` stops at 100).
    pub fn files(&self) -> Result<Vec<String>, String> {
        match self {
            Landing::GitHub { forge, pr } => {
                let files = gh(forge, pr, &["pr", "diff", "--name-only"])?;
                Ok(files.lines().map(String::from).collect())
            }
            Landing::Codeberg { forge, number } => forge.files(*number),
        }
    }

    /// The send paths it changes, which hold it for a reviewer's pass.
    pub fn sends(&self, pr: &Pr) -> Result<Vec<String>, String> {
        match self {
            Landing::GitHub { forge, .. } => forge.sends(pr),
            Landing::Codeberg { forge, number } => Ok(crate::sends(
                forge.files(*number)?.iter().map(String::as_str),
            )),
        }
    }

    pub fn verdict(&self, pr: &Pr) -> Result<Option<Review>, String> {
        match self {
            Landing::GitHub { forge, .. } => forge.verdict(pr),
            Landing::Codeberg { forge, .. } => forge.verdict(pr),
        }
    }

    pub fn comment(&self, body: &str) -> Result<(), String> {
        match self {
            Landing::GitHub { forge, pr } => {
                gh(forge, pr, &["pr", "comment", "--body", body]).map(drop)
            }
            Landing::Codeberg { forge, number } => forge.comment(*number, body),
        }
    }

    /// site_perso's main live after its merge, which restarts the look on the new binary; another repository deploys nothing here.
    pub fn deploy(&self) -> Result<String, String> {
        match self {
            Landing::GitHub { forge, .. } => forge.deploy(),
            Landing::Codeberg { forge, .. } => Ok(format!(
                "nothing: {} is not site_perso, which alone deploys here",
                forge.repo
            )),
        }
    }

    /// The deployed binary's monitors saved when its commit changed their data; Ok(None) when it did not, or holds none.
    pub fn save_monitors(&self) -> Result<Option<String>, String> {
        match self {
            Landing::GitHub { forge, .. } => forge.save_monitors(),
            Landing::Codeberg { .. } => Ok(None),
        }
    }

    /// Whether it is site_perso's, whose parity with the older app `dash land` checks.
    pub fn site_perso(&self) -> bool {
        matches!(self, Landing::GitHub { .. })
    }
}
