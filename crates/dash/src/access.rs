//! `dash access` and /tools on :7799 (task rust-access-map; @nha, 2026-11-08: « dash access -> this is the future of the tools
//! view, but dual-use. - list every access - for each can have a status / usage / other intel. »): every access he holds, once,
//! from bin/secret's names, the repos' tfvars, the CLIs' configs and GitHub's secrets, never a value. A folder the sandbox
//! denies is a row « refused », never an absence: the orchestrator said « no Scaleway keys » after one denied folder.
use crate::fresh::{self, Read};
use dash_surface as surface;
use serde_json::{Map, Value, json};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Works,
    Expired,
    Refused,
    /// No provider asked, or none answered.
    Unknown,
}

impl Status {
    fn word(self) -> &'static str {
        match self {
            Status::Works => "works",
            Status::Expired => "expired",
            Status::Refused => "refused",
            Status::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Access {
    pub source: &'static str,
    pub name: String,
    /// Where it lives: a file, a folder, repos.
    pub place: String,
    pub status: Status,
    pub usage: String,
    pub note: String,
}

impl Access {
    /// Its line in the text: « | » between fields, as a name holds « , » and « : », and none inside one, as a provider's
    /// error text may hold one (/code-review of rust-access-map).
    fn line(&self) -> String {
        let fields = [
            self.name.as_str(),
            self.status.word(),
            &self.usage,
            &self.note,
            &self.place,
        ];
        fields.map(|field| field.replace('|', "/")).join(" | ")
    }
}

/// A command's stdout, or why it failed without the command's own words (a repo named token-service is no expired token):
/// `asked` in the tool, a stand-in in a test. Sync: GitHub's lists run side by side.
pub type Run<'a> = &'a (dyn Fn(&str, &[&str]) -> Result<String, String> + Sync);

const NO_READING: &str = "no reading";
const NOT_ASKED: &str = "no provider asked";

/// A CLI's config and the call that asks its provider whether it still lets him in. `usage`: this month's, where it answers.
struct Cli {
    name: &'static str,
    config: &'static [&'static str],
    check: &'static [&'static str],
}

const CLIS: &[Cli] = &[
    Cli {
        name: "gh",
        config: &[".config/gh/hosts.yml"],
        check: &["gh", "api", "user", "--jq", ".login"],
    },
    Cli {
        name: "aws",
        config: &[".aws/credentials", ".aws/config"],
        check: &["aws", "sts", "get-caller-identity"],
    },
    Cli {
        name: "scw",
        config: &[".config/scw/config.yaml"],
        check: &["scw", "account", "project", "list"],
    },
    Cli {
        name: "fly",
        config: &[".fly/config.yml"],
        check: &["fly", "auth", "whoami"],
    },
    Cli {
        name: "docker",
        config: &[".docker/config.json"],
        check: &[],
    },
    Cli {
        name: "kubectl",
        config: &[".kube/config"],
        check: &[],
    },
    Cli {
        name: "terraform",
        config: &[".terraform.d/credentials.tfrc.json"],
        check: &[],
    },
    Cli {
        name: "npm",
        config: &[".npmrc"],
        check: &[],
    },
    Cli {
        name: "netrc",
        config: &[".netrc"],
        check: &[],
    },
];

/// What a provider's refusal says, read as a status. ponytail: words, not codes; a CLI's own exit codes if one misleads.
fn judged(why: &str) -> Status {
    let why = why.to_lowercase();
    let any = |words: &[&str]| words.iter().any(|w| why.contains(w));
    // AWS answers with codes, not words (/code-review of rust-access-map)
    if any(&[
        "not permitted",
        "permission denied",
        "403",
        "forbidden",
        "accessdenied",
    ]) {
        Status::Refused
    } else if any(&[
        "401",
        "expired",
        "unauthorized",
        "bad credentials",
        "invalid token",
        "invalidclienttokenid",
        "signaturedoesnotmatch",
    ]) {
        Status::Expired
    } else {
        // a missing CLI, a timeout, a crash: nobody refused anything (/code-review of rust-access-map)
        Status::Unknown
    }
}

/// The first `n` characters: a field may hold anything, so never a byte slice.
fn head(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// A failure's first line, short: what the row's note keeps.
fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .trim();
    head(line, 160)
}

fn shown(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// bin/secret's names (`list --json`: names, kinds and states, never a value). `now`: UTC, `YYYY-MM-DDTHH:MM:SS`.
fn secrets(repo: &Path, run: Run, now: &str) -> Vec<Access> {
    let bin = crate::keys::live_tools(repo).secret;
    let parsed = run(&bin, &["list", "--json"]).and_then(|t| {
        serde_json::from_str::<Vec<Value>>(&t).map_err(|_| "its list is not JSON".to_string())
    });
    let list = match parsed {
        Ok(list) => list,
        Err(why) => {
            return vec![Access {
                source: "bin/secret",
                name: "bin/secret".into(),
                place: bin,
                status: judged(&why),
                usage: NO_READING.into(),
                note: first_line(&why),
            }];
        }
    };
    let field = |s: &Value, k: &str| {
        s.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    list.iter()
        .map(|s| {
            let (state, expires) = (field(s, "state"), field(s, "expires"));
            // bin/secret writes UTC, « 2026-11-08T00:00:00+00:00 »: compared to the second. A date alone holds through its day.
            let lapsed = match expires.len() {
                0 => false,
                10 => expires.as_str() < head(now, 10).as_str(),
                _ => head(&expires, 19).as_str() <= now,
            };
            let status = if lapsed || state == "expired" || state == "dead" {
                Status::Expired
            } else {
                Status::Unknown
            };
            let mut note = format!("{}, {state}", field(s, "kind"));
            if !expires.is_empty() {
                note += &format!(", expires {}", head(&expires, 10));
            }
            Access {
                source: "bin/secret",
                name: field(s, "name"),
                place: field(s, "source"),
                status,
                usage: NO_READING.into(),
                note: format!("{note}; {NOT_ASKED}"),
            }
        })
        .collect()
}

/// A worktree or a build holds copies of what its repo holds, never an access of its own.
const SKIPPED: &[&str] = &[
    ".git",
    ".worktrees",
    "worktrees",
    "node_modules",
    "target",
    "_build",
    "deps",
    "vendor",
    ".terraform",
    ".venv",
    ".cache",
    ".next",
];
// ponytail: a folder this deep is not walked and gets no row (source trees go deeper); add a row for it if a tfvars file ever sits there.
const DEPTH: usize = 10;

/// Each tfvars file under `dir`, and each folder the walk was refused.
fn walk(
    dir: &Path,
    depth: usize,
    files: &mut Vec<std::path::PathBuf>,
    refused: &mut Vec<std::path::PathBuf>,
) {
    // denied or any other error: what the folder holds is unknown, never absent (/code-review of rust-access-map)
    let Ok(entries) = std::fs::read_dir(dir) else {
        return refused.push(dir.to_path_buf());
    };
    for entry in entries {
        let Ok(entry) = entry else {
            refused.push(dir.to_path_buf());
            continue;
        };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        // a symlink is read as its target, a linked tfvars file or repo folder; any other entry costs no stat
        let kind = match entry.file_type() {
            Ok(k) if k.is_symlink() => std::fs::metadata(&path).map(|m| m.file_type()),
            kind => kind,
        };
        match kind {
            Ok(m) if m.is_dir() && depth > 0 && !SKIPPED.contains(&name.as_str()) => {
                walk(&path, depth - 1, files, refused)
            }
            Ok(m) if m.is_file() && name.ends_with(".tfvars") => files.push(path),
            Err(_) if name.ends_with(".tfvars") => refused.push(path),
            _ => {}
        }
    }
}

/// A variable that lets someone in: its name says so. An id, a region or a public key is no access.
fn is_credential(key: &str) -> bool {
    let key = key.to_lowercase();
    !key.contains("public")
        && ["key", "token", "secret", "password"]
            .iter()
            .any(|w| key.contains(w))
}

/// A top-level `key = value` line: its key and its value, a quoted one without its quotes and a bare one without a trailing
/// comment. A comment, a block's opening (`{`, `[`, a heredoc), a nested key or an empty value is none.
fn assignment(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    let bare = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    if key.is_empty() || !key.chars().all(bare) || line.starts_with(char::is_whitespace) {
        return None;
    }
    let value = value.trim();
    if value.starts_with(['{', '[']) || value.starts_with("<<") {
        return None;
    }
    let value = match value.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next().unwrap_or_default(),
        None => {
            let uncommented = value.split('#').next().unwrap_or_default();
            uncommented.split("//").next().unwrap_or_default().trim()
        }
    };
    (!value.is_empty()).then_some((key, value))
}

/// The repos' tfvars credentials, once each: one key with one value is one access however many files and repos copy it. The
/// value is hashed in memory to tell copies apart and never kept.
fn tfvars(repos: &Path, home: &Path) -> Vec<Access> {
    let (mut files, mut refused) = (vec![], vec![]);
    walk(repos, DEPTH, &mut files, &mut refused);
    files.sort();
    let mut seen: BTreeMap<(String, u64), (BTreeSet<String>, Vec<String>)> = BTreeMap::new();
    for file in &files {
        let repo = file.strip_prefix(repos).ok().and_then(|p| p.iter().next());
        let repo = repo
            .map(|r| r.to_string_lossy().to_string())
            .unwrap_or_default();
        // a file that is not UTF-8 is still read: only a failed read is unreadable (/code-review of rust-access-map)
        let Ok(bytes) = std::fs::read(file) else {
            refused.push(file.clone());
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        for (key, value) in text.lines().filter_map(assignment) {
            if !is_credential(key) {
                continue;
            }
            let mut hasher = DefaultHasher::new();
            value.hash(&mut hasher);
            let (in_repos, places) = seen.entry((key.to_string(), hasher.finish())).or_default();
            in_repos.insert(repo.clone());
            places.push(shown(file, home));
        }
    }
    // by key, then first file: a value's number never depends on the value itself (/code-review of rust-access-map)
    let mut groups: Vec<_> = seen.into_iter().collect();
    groups.sort_by(|((a, _), (_, at)), ((b, _), (_, bt))| (a, at).cmp(&(b, bt)));
    let mut named: BTreeMap<String, usize> = BTreeMap::new();
    let mut rows: Vec<Access> = groups
        .into_iter()
        .map(|((key, _), (in_repos, places))| {
            let name = format!(
                "{}: {key}",
                in_repos.into_iter().collect::<Vec<_>>().join(", ")
            );
            // one key holding two values in the same repos: two accesses, told apart by a number
            let n = named.entry(name.clone()).or_default();
            *n += 1;
            let name = if *n > 1 {
                format!("{name} ({n})")
            } else {
                name
            };
            Access {
                source: "tfvars",
                name,
                note: format!("in {} file(s); {NOT_ASKED}", places.len()),
                place: places.join(", "),
                status: Status::Unknown,
                usage: NO_READING.into(),
            }
        })
        .collect();
    rows.extend(refused.iter().map(|dir| Access {
        source: "tfvars",
        name: shown(dir, home),
        place: shown(dir, home),
        status: Status::Refused,
        usage: NO_READING.into(),
        note: "unreadable here: what it holds is unknown, not absent".into(),
    }));
    rows
}

/// The forge's Actions minutes this month, as GitHub's billing answers it.
fn github_minutes(owner: &str, month: &str, run: Run) -> String {
    let (year, month) = month.split_once('-').unwrap_or_default();
    let path = format!(
        "/organizations/{owner}/settings/billing/usage?year={year}&month={}",
        month.trim_start_matches('0')
    );
    let jq = r#"[.usageItems[]|select(.product=="actions" and .unitType=="Minutes")|.quantity]|add // 0|floor"#;
    match run("gh", &["api", &path, "--jq", jq]) {
        Ok(minutes) => format!("{} Actions minutes ({owner})", minutes.trim()),
        Err(e) => format!("{NO_READING}: {}", first_line(&e)),
    }
}

/// Each CLI config found, or refused, with its provider's answer.
fn clis(home: &Path, owner: &str, month: &str, run: Run) -> Vec<Access> {
    let mut rows = vec![];
    for cli in CLIS {
        // the first of its files there, or denied
        let found = cli.config.iter().find_map(|config| {
            let path = home.join(config);
            match std::fs::metadata(&path) {
                Ok(_) => Some((path, false)),
                Err(e) if e.kind() == ErrorKind::PermissionDenied => Some((path, true)),
                Err(_) => None,
            }
        });
        let Some((path, denied)) = found else {
            continue;
        };
        let (status, said) = match cli.check {
            [] if denied => (Status::Refused, NOT_ASKED.to_string()),
            [] => (Status::Unknown, NOT_ASKED.to_string()),
            [bin, args @ ..] => {
                let answer = run(bin, args);
                let said = match &answer {
                    Ok(text) => format!("{} says {}", cli.name, first_line(text)),
                    Err(e) => format!("{bin}: {}", first_line(e)),
                };
                (
                    answer.as_ref().err().map_or(Status::Works, |e| judged(e)),
                    said,
                )
            }
        };
        let note = if denied {
            format!(
                "the sandbox denies {}: what it holds is unknown, not absent; {said}",
                shown(&path, home)
            )
        } else {
            said
        };
        let usage = if cli.name == "gh" && status == Status::Works {
            github_minutes(owner, month, run)
        } else {
            NO_READING.into()
        };
        rows.push(Access {
            source: "cli",
            name: cli.name.into(),
            place: shown(&path, home),
            status,
            usage,
            note,
        });
    }
    rows
}

const REPO_LIMIT: usize = 1000;

/// GitHub's secrets of the forge owner: one row a name, its repos in its place. Never a value: GitHub gives none.
fn github(owner: &str, run: Run) -> Vec<Access> {
    let failed = |name: String, e: String| Access {
        source: "github",
        name,
        place: format!("github.com/{owner}"),
        status: judged(&e),
        usage: NO_READING.into(),
        note: first_line(&e),
    };
    let repos = match run(
        "gh",
        &[
            "repo",
            "list",
            owner,
            "--limit",
            &REPO_LIMIT.to_string(),
            "--json",
            "name",
            "--jq",
            ".[].name",
        ],
    ) {
        Ok(text) => text
            .lines()
            .map(str::to_string)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>(),
        Err(e) => return vec![failed(format!("{owner}: its repos"), e)],
    };
    let mut names: BTreeMap<String, (BTreeSet<String>, String)> = BTreeMap::new();
    let mut rows = vec![];
    if repos.len() == REPO_LIMIT {
        // the list stops there: a later repo's secrets are unknown, never absent (/code-review of rust-access-map)
        let cut = format!("listed {REPO_LIMIT} repos, gh's limit: later ones are not read");
        rows.push(failed(format!("{owner}: repos past {REPO_LIMIT}"), cut));
    }
    let scopes: Vec<_> = repos
        .iter()
        .map(|r| (r.clone(), format!("{owner}/{r}"), "--repo"))
        .chain([(
            format!("{owner} (organization)"),
            owner.to_string(),
            "--org",
        )])
        .collect();
    let list = |(_, target, flag): &(String, String, &str)| {
        let listed = run(
            "gh",
            &["secret", "list", flag, target, "--json", "name,updatedAt"],
        );
        listed.and_then(|t| serde_json::from_str::<Vec<Value>>(&t).map_err(|e| e.to_string()))
    };
    // A call each, eight at a time: one after another, 60 repos took past 40 s (/code-review of rust-access-map).
    let mut answers = vec![];
    for batch in scopes.chunks(8) {
        std::thread::scope(|s| {
            let calls: Vec<_> = batch
                .iter()
                .map(|scope| s.spawn(move || list(scope)))
                .collect();
            let joined = calls
                .into_iter()
                .map(|c| c.join().unwrap_or_else(|_| Err("its list panicked".into())));
            answers.extend(joined);
        });
    }
    for ((label, _, flag), parsed) in scopes.into_iter().zip(answers) {
        match parsed {
            Ok(list) => {
                for secret in list {
                    let name = secret["name"].as_str().unwrap_or_default().to_string();
                    let updated = secret["updatedAt"].as_str().unwrap_or_default();
                    let (repos, newest) = names.entry(name).or_default();
                    repos.insert(label.clone());
                    if updated > newest.as_str() {
                        *newest = updated.to_string();
                    }
                }
            }
            Err(e) if flag == "--org" && e.contains("404") => {}
            Err(e) => rows.push(failed(format!("{label}: its secrets"), e)),
        }
    }
    let listed = names.into_iter().map(|(name, (repos, updated))| Access {
        source: "github",
        name,
        place: repos.into_iter().collect::<Vec<_>>().join(", "),
        status: Status::Unknown,
        usage: NO_READING.into(),
        note: format!(
            "updated {}; GitHub never says whether it works",
            head(&updated, 10)
        ),
    });
    listed.chain(rows).collect()
}

/// The owner forge.env names, as `dash site deploy` reads it.
fn owner(repo: &Path) -> String {
    let text = std::fs::read_to_string(repo.join("forge.env")).unwrap_or_default();
    let named = text.lines().find_map(|l| l.strip_prefix("GITHUB_OWNER="));
    named
        .map(|o| o.trim().trim_matches('"').to_string())
        .unwrap_or_else(|| "nha-org".into())
}

/// Every access, from `home` (its `repo/` holds the repos, `repo` this one), each provider asked through `run`. `now`: UTC,
/// `YYYY-MM-DDTHH:MM:SS`.
pub fn read(home: &Path, repo: &Path, now: &str, run: Run) -> Vec<Access> {
    let owner = owner(repo);
    let mut rows = secrets(repo, run, now);
    rows.extend(tfvars(&home.join("repo"), home));
    rows.extend(clis(home, &owner, &head(now, 7), run));
    rows.extend(github(&owner, run));
    rows
}

/// The rows as the page's tree, a section a source and a line a row: the HTML and the text are both drawn from it.
fn tree(rows: &[Access], note: &str) -> String {
    let text = |t: String| json!({"type": "text", "props": {"text": t}});
    let mut elements = Map::new();
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": "Tools"}}),
    );
    elements.insert("note".into(), text(note.into()));
    let mut children = vec!["title".to_string(), "note".to_string()];
    let sources: BTreeSet<&str> = rows.iter().map(|r| r.source).collect();
    for (s, source) in sources.into_iter().enumerate() {
        let mut lines = vec![];
        for (i, row) in rows.iter().enumerate().filter(|(_, r)| r.source == source) {
            let id = format!("row-{i}");
            elements.insert(id.clone(), text(row.line()));
            lines.push(id);
        }
        let id = format!("source-{s}");
        elements.insert(
            id.clone(),
            json!({"type": "section", "props": {"title": source}, "children": lines}),
        );
        children.push(id);
    }
    elements.insert(
        "page".into(),
        json!({"type": "stack", "children": children}),
    );
    json!({"root": "page", "elements": elements}).to_string()
}

/// Now in UTC, as `read` takes it.
fn now() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let date = dash_core::date((now / 86_400) as i64);
    let (h, m, s) = (now / 3600 % 24, now / 60 % 60, now % 60);
    format!("{date}T{h:02}:{m:02}:{s:02}")
}

/// `dash_process::run`, its failure without the « <bin> <args>: » it opens on.
fn asked(bin: &str, args: &[&str]) -> Result<String, String> {
    let what = format!("{bin} {}:", args.join(" "));
    dash_process::run(bin, args, None, Duration::from_secs(20)).map_err(|e| {
        e.strip_prefix(&what)
            .map_or(e.clone(), |why| why.trim().to_string())
    })
}

fn live(home: &Path) -> Vec<Access> {
    read(home, &crate::tasks::repo(), &now(), &asked)
}

/// `dash access`: the rows as /tools' text.
pub fn list(mut out: std::fs::File) {
    use std::io::Write;
    let text = std::env::var("HOME")
        .map_err(|e| format!("HOME: {e}"))
        .and_then(|home| surface::text(&tree(&live(Path::new(&home)), "Read now.")));
    match text {
        Ok(text) => write!(out, "{text}").expect("stdout"),
        Err(e) => {
            eprintln!("access: {e}");
            std::process::exit(1);
        }
    }
}

/// Some 10 s of provider calls, read off the routing thread as /agents reads.
static READ: Mutex<Read<Vec<Access>>> = Mutex::new(Read::new());

/// /tools: HTML when the client asks for it (a browser), else the text `dash access` prints. `None` before the first read
/// ends, for the text: an agent must never read « no rows » as « no access » (/code-review of rust-access-map).
pub fn page(home: &Path, html: bool) -> Result<Option<String>, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let home = home.to_path_buf();
    let read = fresh::last(&READ, now, "access", move || live(&home))?;
    let (rows, note) = match read {
        Some((rows, at)) => (rows, fresh::ago(now, at)),
        None if !html => return Ok(None),
        None => (
            vec![],
            "Reading every access: the next reload shows it.".to_string(),
        ),
    };
    let tree = tree(&rows, &note);
    if html {
        surface::page("Tools", &tree).map(Some)
    } else {
        surface::text(&tree).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;
    use std::os::unix::fs::PermissionsExt;

    const VALUE: &str = "SCW-STAND-IN-SECRET-0000";

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(path, text).expect("write");
    }

    /// A home with: one key copied into a worktree, a second folder of the same repo and another repo, a key holding two
    /// values in one repo, comments and a nested key, a readable CLI config, one the sandbox denies, and bin/secret and
    /// GitHub answering as stand-ins.
    fn home(dir: &Path) {
        let tf = |access: &str| {
            format!("access_key = \"{access}\"\nsecret_key = \"{VALUE}\"\nproject_id = \"p-1\"\n")
        };
        write(
            &dir.join("repo/klar/infra/scaleway.auto.tfvars"),
            &tf("SCWAAA"),
        );
        write(
            &dir.join("repo/klar/infra/prod/scaleway.auto.tfvars"),
            &tf("SCWBBB"),
        );
        write(
            &dir.join("repo/klar/.worktrees/a/infra/scaleway.auto.tfvars"),
            &tf("SCWAAA"),
        );
        write(
            &dir.join("repo/graal/infra/terraform.tfvars"),
            &format!(
                "secret_key = \"{VALUE}\" # klar's\n// api_token = \"old-STAND-IN\"\nnested = {{\n  token = \"x\"\n}}\n"
            ),
        );
        write(&dir.join(".aws/credentials"), "[default]\n");
        write(
            &dir.join("repo/site_perso/forge.env"),
            "GITHUB_OWNER=acme\n",
        );
        write(
            &dir.join(".config/gh/hosts.yml"),
            "github.com:\n  user: nha\n",
        );
        write(
            &dir.join(".config/scw/config.yaml"),
            &format!("secret_key: {VALUE}\n"),
        );
        write(
            &dir.join("repo/locked/x.tfvars"),
            &format!("token = \"{VALUE}\"\n"),
        );
        let lock = |p: &Path| {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o000)).expect("chmod")
        };
        lock(&dir.join(".config/scw"));
        lock(&dir.join("repo/locked"));
    }

    fn unlock(dir: &Path) {
        for p in [".config/scw", "repo/locked"] {
            let _ = std::fs::set_permissions(dir.join(p), std::fs::Permissions::from_mode(0o755));
        }
    }

    fn providers(bin: &str, args: &[&str]) -> Result<String, String> {
        let said = args.join(" ");
        match (bin, said.as_str()) {
            (b, "list --json") if b.ends_with("bin/secret") => Ok(json!([
                {"name": "anthropic", "kind": "API key", "source": "~/repo/site_perso/.secrets/anthropic.env", "state": "ok", "expires": null},
                {"name": "buildkite", "kind": "API access token", "source": "~/.secrets/buildkite.env", "state": "ok", "expires": "2026-11-08T00:00:00+00:00"},
                {"name": "depot", "kind": "token", "source": "~/.secrets/depot.env", "state": "ok", "expires": "2026-11-08T23:00:00+00:00"}
            ])
            .to_string()),
            ("gh", "api user --jq .login") => Ok("nha\n".into()),
            ("gh", s) if s.starts_with("api /organizations/acme/settings/billing/usage?year=2026&month=11") => Ok("2073\n".into()),
            ("gh", s) if s.starts_with("repo list acme") => Ok("site_perso\ncli\n".into()),
            ("gh", s) if s.starts_with("secret list --repo acme/") => {
                Ok(r#"[{"name":"ANTHROPIC_API_KEY","updatedAt":"2026-10-25T21:00:00Z"}]"#.into())
            }
            ("gh", s) if s.starts_with("secret list --org acme") => Ok(r#"[{"name":"FLY_ORG","updatedAt":"2026-10-01T00:00:00Z"}]"#.into()),
            // as `asked` gives them: the reason, never the command's own words
            ("scw", _) => Err("exit status: 1: stat config.yaml: operation not permitted".into()),
            ("fly", _) => Err("exit status: 1: Error: token expired".into()),
            ("aws", _) => Err("exit status: 253: Unable to locate credentials".into()),
            _ => Err("not a stand-in".into()),
        }
    }

    fn rows(dir: &Path) -> Vec<Access> {
        home(dir);
        // fly's config is there so its probe runs
        write(&dir.join(".fly/config.yml"), "access_token: x\n");
        let rows = read(
            dir,
            &dir.join("repo/site_perso"),
            "2026-11-08T12:00:00",
            &providers,
        );
        unlock(dir);
        rows
    }

    #[test]
    fn each_access_is_listed_once_and_never_its_value() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("access-once");
        let rows = rows(&dir.0);
        let names: Vec<(&str, &str)> = rows.iter().map(|r| (r.source, r.name.as_str())).collect();
        let count =
            |source: &str, name: &str| names.iter().filter(|n| **n == (source, name)).count();
        // bin/secret's names
        assert_eq!(count("bin/secret", "anthropic"), 1, "{names:?}");
        // one value in two repos and three files, a worktree's copy skipped and a trailing comment no other value: one row
        let shared = "graal, klar: secret_key";
        assert_eq!(count("tfvars", shared), 1, "{names:?}");
        let key = rows.iter().find(|r| r.name == shared).expect("shared");
        assert!(key.note.starts_with("in 3 file(s)"), "{key:?}");
        // one key, two values, one repo: two rows told apart
        assert_eq!(count("tfvars", "klar: access_key"), 1, "{names:?}");
        assert_eq!(count("tfvars", "klar: access_key (2)"), 1, "{names:?}");
        // numbered by first file, whatever the values: prod/ sorts before scaleway.auto.tfvars
        let second = rows.iter().find(|r| r.name == "klar: access_key (2)");
        assert_eq!(
            second.map(|r| r.place.as_str()),
            Some("~/repo/klar/infra/scaleway.auto.tfvars")
        );
        // an id, a comment and a nested key are no access
        assert_eq!(count("tfvars", "klar: project_id"), 0, "{names:?}");
        assert!(
            !names
                .iter()
                .any(|(_, n)| n.contains("api_token") || n.ends_with(": token")),
            "{names:?}"
        );
        assert!(!is_credential("ssh_public_keys") && is_credential("tem_smtp_password"));
        // a denied folder is a refused row, never an absence
        let locked = rows
            .iter()
            .find(|r| r.source == "tfvars" && r.name.ends_with("repo/locked"))
            .expect("locked");
        assert_eq!(locked.status, Status::Refused);
        // the CLIs' configs: gh read, scw denied, aws by its credentials alone, docker absent
        assert_eq!(count("cli", "gh"), 1);
        assert_eq!(count("cli", "scw"), 1);
        assert_eq!(count("cli", "aws"), 1);
        assert_eq!(count("cli", "docker"), 0);
        // GitHub: one name in two repos is one row naming both
        assert_eq!(count("github", "ANTHROPIC_API_KEY"), 1, "{names:?}");
        let key = rows
            .iter()
            .find(|r| r.name == "ANTHROPIC_API_KEY")
            .expect("key");
        assert_eq!(key.place, "cli, site_perso");
        assert_eq!(count("github", "FLY_ORG"), 1);
        let printed = surface::text(&tree(&rows, "")).expect("text") + &tree(&rows, "");
        assert!(
            !printed.contains(VALUE)
                && !printed.contains("SCWAAA")
                && !printed.contains("SCWBBB")
                && !printed.contains("old-STAND-IN"),
            "{printed}"
        );
    }

    #[test]
    fn a_provider_s_answer_is_a_row_s_status_and_its_usage() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("access-status");
        let rows = rows(&dir.0);
        let row = |source: &str, name: &str| {
            rows.iter()
                .find(|r| r.source == source && r.name == name)
                .expect(name)
                .clone()
        };
        let gh = row("cli", "gh");
        assert_eq!(gh.status, Status::Works);
        assert_eq!(gh.usage, "2073 Actions minutes (acme)");
        assert_eq!(gh.note, "gh says nha");
        let scw = row("cli", "scw");
        assert_eq!(scw.status, Status::Refused);
        assert!(
            scw.note
                .starts_with("the sandbox denies ~/.config/scw/config.yaml"),
            "{scw:?}"
        );
        let fly = row("cli", "fly");
        assert_eq!(fly.status, Status::Expired);
        assert_eq!(fly.usage, NO_READING);
        // no credentials is nobody refusing
        assert_eq!(row("cli", "aws").status, Status::Unknown);
        // an expiry this morning is past, one tonight is not; a key nobody asks says so
        assert_eq!(row("bin/secret", "buildkite").status, Status::Expired);
        assert_eq!(row("bin/secret", "depot").status, Status::Unknown);
        let anthropic = row("bin/secret", "anthropic");
        assert_eq!(anthropic.status, Status::Unknown);
        assert!(anthropic.note.ends_with(NOT_ASKED), "{anthropic:?}");
        // a refusal is read from its words
        assert_eq!(judged("HTTP 401: Bad credentials"), Status::Expired);
        assert_eq!(judged("HTTP 403: Forbidden"), Status::Refused);
        assert_eq!(judged("gh api user: no answer in 20s"), Status::Unknown);
        // a missing CLI is unknown, and its command's words never reach the judgement
        let missing = asked("/no/such/token-403-cli", &["x"]);
        assert!(
            !missing.as_ref().unwrap_err().contains("token-403"),
            "{missing:?}"
        );
        assert_eq!(judged(missing.as_ref().unwrap_err()), Status::Unknown);
        assert_eq!(head("2026-11-é8T00", 10), "2026-11-é8");
    }

    #[test]
    fn the_page_and_the_text_hold_the_same_rows() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("access-views");
        let rows = rows(&dir.0);
        let tree = tree(&rows, "Read 0 min ago.");
        let (text, html) = (
            surface::text(&tree).expect("text"),
            surface::page("Tools", &tree).expect("page"),
        );
        // the heading, the note, a title a source, a line a row
        let sources: BTreeSet<&str> = rows.iter().map(|r| r.source).collect();
        let said = text.lines().filter(|l| !l.is_empty()).count();
        assert_eq!(said, 2 + sources.len() + rows.len(), "{text}");
        assert!(text.contains("\n## tfvars\n"), "{text}");
        for row in &rows {
            let line = row.line();
            // five fields a line, split back whole
            assert!(text.lines().any(|l| l == line), "{row:?}");
            assert_eq!(line.split(" | ").count(), 5, "{line}");
            let escaped = line
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            assert!(html.contains(&escaped), "{row:?}");
        }
    }

    #[test]
    fn a_block_an_empty_value_and_a_date_alone_are_read_as_written() {
        let _budget = TestBudget::new();
        for line in [
            "access_keys = {",
            "tokens = [",
            "secret = <<EOF",
            "secret_key = \"\"",
            "api_token =",
        ] {
            assert_eq!(assignment(line), None, "{line}");
        }
        assert_eq!(
            assignment("api_token = abc # old"),
            Some(("api_token", "abc"))
        );
        let listed = |expires: &str| {
            let list = json!([{"name": "k", "kind": "token", "source": "s", "state": "ok", "expires": expires}]);
            let run = move |_: &str, _: &[&str]| Ok(list.to_string());
            secrets(Path::new("/r"), &run, "2026-11-08T12:00:00")[0].status
        };
        assert_eq!(
            listed("2026-11-08"),
            Status::Unknown,
            "holds through its day"
        );
        assert_eq!(listed("2026-11-07"), Status::Expired);
        assert_eq!(listed("2026-11-08T11:59:59+00:00"), Status::Expired);
    }

    /// The review lane's /code-review of PR 1217: a link is read as its target, a file that is not UTF-8 is read, a
    /// terraform cache is no access, AWS's codes are judged, and a field's « | » never splits its line.
    #[test]
    fn a_link_a_latin_1_file_and_a_cache_are_read_as_what_they_are() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("access-links");
        let repos = dir.0.join("repo");
        write(&dir.0.join("shared/x.tfvars"), "api_token = \"linked\"\n");
        write(
            &repos.join("a/plain.tfvars"),
            "db_password = \"caf\u{e9}\"\n",
        );
        std::fs::write(
            repos.join("a/latin.tfvars"),
            b"smtp_password = \"caf\xe9\"\n",
        )
        .expect("latin-1");
        write(
            &repos.join("a/.terraform/modules/m/examples/e.tfvars"),
            "secret_key = \"dummy\"\n",
        );
        std::os::unix::fs::symlink(dir.0.join("shared/x.tfvars"), repos.join("a/link.tfvars"))
            .expect("link");
        let rows = tfvars(&repos, &dir.0);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            ["a: api_token", "a: db_password", "a: smtp_password"],
            "{rows:?}"
        );
        assert!(rows.iter().all(|r| r.status != Status::Refused), "{rows:?}");
        for code in ["AccessDenied: no", "InvalidClientTokenId: bad"] {
            assert_ne!(judged(code), Status::Unknown, "{code}");
        }
        let mut row = rows[0].clone();
        row.note = "gh: error | jq".into();
        let text = surface::text(&tree(&[row], "")).expect("text");
        let line = text
            .lines()
            .find(|l| l.starts_with("a: api_token"))
            .expect("line");
        assert_eq!(line.split(" | ").count(), 5, "{line}");
    }
}
