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
    /// Whom it lets him into: its card on /tools.
    pub provider: String,
    pub name: String,
    /// Where it lives: a file, a folder, repos.
    pub place: String,
    pub status: Status,
    pub usage: String,
    pub note: String,
}

impl Access {
    /// Its line on its provider's card, a status chip first; in the text one line, the table's (surface's access element).
    fn element(&self) -> Value {
        json!({"type": "access", "props": {
            "name": self.name,
            "status": self.status.word(),
            "usage": self.usage,
            "note": self.note,
            "place": self.place,
            "source": self.source,
        }})
    }
}

/// A provider's name from the word an access is known by: bin/secret's name, a CLI, a key's first word. ponytail: a short
/// alias list, so DOCKERHUB_TOKEN's « dockerhub » is not the docker CLI's card; an alias here when a real name splits one.
fn provider(word: &str) -> String {
    let word = word.to_lowercase();
    let word = word.trim_end_matches("-password").replace('-', "");
    match word.as_str() {
        "gh" => "github".into(),
        "scw" => "scaleway".into(),
        _ => word,
    }
}

/// The provider a key's name opens on (BUNNYNET_API_KEY, healthchecks_api_key), none for a name any provider uses
/// (access_key, SECRET_KEY, s3_secret_key, API_TOKEN).
fn key_provider(key: &str) -> Option<String> {
    let first = key.split('_').next().unwrap_or(key).to_lowercase();
    let any = [
        "access", "secret", "s3", "tem", "api", "admin", "token", "password", "key",
    ];
    (!any.contains(&first.as_str())).then(|| provider(&first))
}

/// A tfvars key's provider: its name's, else its file's, scaleway.auto.tfvars's « scaleway »; terraform.tfvars names none.
/// ponytail: the first file's that is not terraform.tfvars, so prod.tfvars makes a « prod » card and a key copied into two
/// providers' files takes the first; a provider list if a real file ever misleads.
fn tfvars_provider(key: &str, places: &[String]) -> String {
    let stem = |p: &String| {
        let file = p.rsplit('/').next().unwrap_or(p);
        file.split('.').next().unwrap_or(file).to_string()
    };
    let file = || places.iter().map(stem).find(|s| s != "terraform");
    key_provider(key).unwrap_or_else(|| provider(&file().unwrap_or_else(|| "terraform".into())))
}

/// A command's stdout, or why it failed without the command's own words (a repo named token-service is no expired token):
/// `asked` in the tool, a stand-in in a test. Sync: GitHub's lists run side by side.
pub type Run<'a> = &'a (dyn Fn(&str, &[&str], &[(&str, &str)]) -> Result<String, String> + Sync);

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

/// Why a row has no provider's answer: never a bare unknown (task rust-access-usage).
fn unasked(what: &str) -> String {
    format!("{NOT_ASKED}: dash has no check for {what}")
}

/// A provider's HTTP answer, its code and body, through curl: the key in curl's environment alone (`--variable`), never on a
/// command line (rust-key-agent's rule). `auth`: the key's header, `{{KEY}}` where the key goes.
fn http(
    run: Run,
    auth: &str,
    key: &str,
    headers: &[&str],
    data: Option<&str>,
    url: &str,
) -> Result<(u16, String), String> {
    let mut args = vec![
        "-sS",
        "--max-time",
        "15",
        "--variable",
        "%KEY",
        "--expand-header",
        auth,
    ];
    for header in headers {
        args.extend(["-H", header]);
    }
    if let Some(data) = data {
        args.extend(["-H", "content-type: application/json", "--data", data]);
    }
    args.extend(["-w", "\n%{http_code}", url]);
    let text = run("curl", &args, &[("KEY", key)])?;
    let (body, code) = text
        .trim_end()
        .rsplit_once('\n')
        .unwrap_or(("", text.trim()));
    let code = code
        .parse()
        .map_err(|_| format!("curl gave no status: {}", first_line(&text)))?;
    Ok((code, body.to_string()))
}

/// A provider's own words for a failure: its JSON's message, else the body's first line.
fn why(code: u16, body: &str) -> String {
    let json: Value = serde_json::from_str(body).unwrap_or_default();
    let said = ["/error/message", "/message", "/errors/0/message"]
        .iter()
        .find_map(|at| json.pointer(at).and_then(Value::as_str));
    format!(
        "HTTP {code}: {}",
        said.map_or_else(|| first_line(body), |s| head(s, 160))
    )
}

/// A check's answer as a status and the note it leaves. Fly says 200 with `errors` to a dead token.
fn checked(
    answer: &Result<(u16, String), String>,
    works: impl Fn(&Value) -> String,
) -> (Status, String) {
    match answer {
        Ok((200..=299, body)) if !body.contains("\"errors\"") => (
            Status::Works,
            works(&serde_json::from_str(body).unwrap_or_default()),
        ),
        // GraphQL's dead token: 200, its code UNAUTHORIZED (/code-review of rust-access-usage: the code, never words in a body)
        Ok((200..=299, body)) if body.contains("\"UNAUTHORIZED\"") => {
            (Status::Expired, why(200, body))
        }
        Ok((401, body)) => (Status::Expired, why(401, body)),
        Ok((403, body)) => (Status::Refused, why(403, body)),
        Ok((code, body)) => (Status::Unknown, why(*code, body)),
        // curl failed (a timeout, no route): its stdout may hold a body, never a judgement
        Err(e) => (Status::Unknown, first_line(e)),
    }
}

/// A usage call's reading, or why there is none.
fn reading(
    answer: Result<(u16, String), String>,
    read: impl Fn(&Value) -> Option<String>,
) -> String {
    match answer {
        Ok((200, body)) => {
            let json = serde_json::from_str(&body).unwrap_or_default();
            read(&json).unwrap_or_else(|| format!("{NO_READING}: its answer holds no usage"))
        }
        Ok((code, body)) => format!("{NO_READING}: {}", why(code, &body)),
        Err(e) => format!("{NO_READING}: {}", first_line(&e)),
    }
}

/// A row's status, this month's usage and its note, as its provider answers them.
type Checked = (Status, String, String);

/// `now`: UTC, `YYYY-MM-DDTHH:MM:SS`; this month is its first 7 characters.
fn anthropic(key: &str, _owner: &str, now: &str, run: Run) -> Checked {
    let ask = |url: &str| {
        http(
            run,
            "x-api-key: {{KEY}}",
            key,
            &["anthropic-version: 2023-06-01"],
            None,
            url,
        )
    };
    // ponytail: one page of daily buckets, 31 covers a month; follow next_page if a bucket is ever finer
    let url = format!(
        "https://api.anthropic.com/v1/organizations/cost_report?starting_at={}-01T00:00:00Z&limit=31",
        head(now, 7)
    );
    // the cost report first: an Admin key reads it and cannot list models (/code-review of rust-access-usage)
    let report = ask(&url);
    if let Ok((200, _)) = report {
        let (status, note) = checked(&report, |_| "Anthropic answers its cost report".into());
        return (status, cost(report), note);
    }
    let (status, note) = checked(&ask("https://api.anthropic.com/v1/models?limit=1"), |_| {
        "Anthropic lists its models".into()
    });
    let usage = if status == Status::Works {
        cost(report)
    } else {
        NO_READING.into()
    };
    (status, usage, note)
}

/// Anthropic's cost report as this month's dollars, or why there are none.
fn cost(report: Result<(u16, String), String>) -> String {
    reading(report, |json| {
        let amounts = json["data"]
            .as_array()?
            .iter()
            .flat_map(|day| day["results"].as_array().into_iter().flatten());
        // Anthropic's cost report counts cents, as a decimal string
        let cents: f64 = amounts
            .filter_map(|r| r["amount"].as_str()?.parse::<f64>().ok())
            .sum();
        Some(format!("${:.2} this month", cents / 100.0))
    })
}

/// Fly answers a token's billing state, never a month's usage: the usage says so.
fn fly(key: &str, _owner: &str, _now: &str, run: Run) -> Checked {
    let query = r#"{"query":"{ organizations { nodes { slug billingStatus creditBalanceFormatted apps { totalCount } } } }"}"#;
    // `fly tokens create` gives « FlyV1 fm2_… », sent as it is; an older token goes after Bearer
    let auth = if key.starts_with("FlyV1 ") {
        "Authorization: {{KEY}}"
    } else {
        "Authorization: Bearer {{KEY}}"
    };
    let answer = http(
        run,
        auth,
        key,
        &[],
        Some(query),
        "https://api.fly.io/graphql",
    );
    let (status, note) = checked(&answer, |_| "Fly lists its organizations".into());
    let usage = if status == Status::Works {
        reading(answer, |json| {
            let orgs = json.pointer("/data/organizations/nodes")?.as_array()?;
            let said: Vec<String> = orgs
                .iter()
                .map(|o| {
                    format!(
                        "{} billing {}, credit {}, {} app(s)",
                        o["slug"].as_str().unwrap_or("?"),
                        o["billingStatus"].as_str().unwrap_or("?"),
                        o["creditBalanceFormatted"].as_str().unwrap_or("?"),
                        o.pointer("/apps/totalCount").unwrap_or(&Value::Null)
                    )
                })
                .collect();
            Some(format!(
                "{NO_READING}: Fly's API gives no month's usage; {}",
                said.join("; ")
            ))
        })
    } else {
        NO_READING.into()
    };
    (status, usage, note)
}

/// CircleCI's credits this month on the forge owner's projects: its insights by project, from the 1st to now.
fn circleci(key: &str, owner: &str, now: &str, run: Run) -> Checked {
    let api = "https://circleci.com/api/v2";
    let ask = |path: &str| {
        http(
            run,
            "Circle-Token: {{KEY}}",
            key,
            &[],
            None,
            &format!("{api}/{path}"),
        )
    };
    let (status, note) = checked(&ask("me"), |me| {
        format!("CircleCI says {}", me["login"].as_str().unwrap_or("?"))
    });
    if status != Status::Works {
        return (status, NO_READING.into(), note);
    }
    let summary = ask(&format!(
        "insights/gh/{owner}/summary?reporting-window=last-90-days"
    ));
    let usage = reading(summary, |json| {
        let projects = json["all_projects"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let (from, to) = (format!("{}-01T00:00:00Z", head(now, 7)), format!("{now}Z"));
        let mut credits = 0;
        for project in projects.iter().filter_map(Value::as_str) {
            // ponytail: one page of workflows a project; follow next_page_token if a project ever holds more
            let path = format!(
                "insights/gh/{owner}/{project}/workflows?all-branches=true&start-date={from}&end-date={to}"
            );
            match ask(&path) {
                Ok((200, body)) => {
                    let json: Value = serde_json::from_str(&body).unwrap_or_default();
                    let items = json["items"]
                        .as_array()
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    credits += items
                        .iter()
                        .filter_map(|w| w.pointer("/metrics/total_credits_used")?.as_i64())
                        .sum::<i64>();
                }
                // a project unread is no zero: the month's count says so
                Ok((code, body)) => {
                    return Some(format!("{NO_READING}: {project}: {}", why(code, &body)));
                }
                Err(e) => return Some(format!("{NO_READING}: {project}: {}", first_line(&e))),
            }
        }
        Some(format!("{credits} credits this month (gh/{owner})"))
    });
    (status, usage, note)
}

/// A Scaleway key, as klar's tfvars hold it: the secret and its organization.
fn scaleway(secret: &str, organization: &str, run: Run) -> Checked {
    let ask = |path: &str| {
        let url = format!("https://api.scaleway.com/{path}");
        http(run, "X-Auth-Token: {{KEY}}", secret, &[], None, &url)
    };
    // ponytail: a key with no right on fr-par-1's servers reads refused; ask another API if a key ever holds none there
    let servers =
        format!("instance/v1/zones/fr-par-1/servers?per_page=1&organization={organization}");
    let (status, note) = checked(&ask(&servers), |_| {
        "Scaleway lists fr-par-1's servers".into()
    });
    if status != Status::Works {
        return (status, NO_READING.into(), note);
    }
    // the current billing period by default; ponytail: one page of 100 lines
    let billing =
        format!("billing/v2beta1/consumptions?organization_id={organization}&page_size=100");
    let usage = reading(ask(&billing), |json| {
        let lines = json["consumptions"].as_array()?;
        let money = |v: &Value| {
            v["units"].as_f64().unwrap_or(0.0) + v["nanos"].as_f64().unwrap_or(0.0) / 1e9
        };
        let total: f64 = lines.iter().map(|l| money(&l["value"])).sum();
        let currency = lines
            .first()
            .and_then(|l| l.pointer("/value/currency_code")?.as_str())
            .unwrap_or("EUR");
        let count = json["total_count"].as_u64().unwrap_or(0);
        let cut = if count > lines.len() as u64 {
            format!(", the first {} of {count} lines", lines.len())
        } else {
            String::new()
        };
        Some(format!("{total:.2} {currency} this month{cut}"))
    });
    (status, usage, note)
}

/// A provider's check: the key, the forge owner, now, the runner.
type Check = fn(&str, &str, &str, Run) -> Checked;

/// The providers dash asks, by bin/secret's name: each of its keys is asked.
fn check_for(name: &str) -> Option<Check> {
    match name {
        "anthropic" => Some(anthropic),
        "fly" => Some(fly),
        "circleci" => Some(circleci),
        _ => None,
    }
}

/// bin/secret's names (`list --json`: names, kinds and states, never a value). `now`: UTC, `YYYY-MM-DDTHH:MM:SS`.
fn secrets(repo: &Path, owner: &str, run: Run, now: &str) -> Vec<Access> {
    let bin = crate::keys::live_tools(repo).secret;
    let parsed = run(&bin, &["list", "--json"], &[]).and_then(|t| {
        serde_json::from_str::<Vec<Value>>(&t).map_err(|_| "its list is not JSON".to_string())
    });
    let list = match parsed {
        Ok(list) => list,
        Err(why) => {
            return vec![Access {
                source: "bin/secret",
                provider: "bin/secret".into(),
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
        .flat_map(|s| {
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
            let name = field(s, "name");
            let provider = provider(&name);
            let row = |name: String, (status, usage, said): Checked| Access {
                source: "bin/secret",
                provider: provider.clone(),
                name,
                place: field(s, "source"),
                status,
                usage,
                note: format!("{note}; {said}"),
            };
            let Some(check) = check_for(&name) else {
                return vec![row(
                    name.clone(),
                    (status, NO_READING.into(), unasked(&name)),
                )];
            };
            // each of its keys asked: anthropic holds one a login (rust-access-usage)
            let vars: Vec<&str> = s["keys"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            if vars.is_empty() {
                // a secret with no key is still a row, never an absence
                let why = format!("{NOT_ASKED}: bin/secret lists no key in it");
                return vec![row(name.clone(), (status, NO_READING.into(), why))];
            }
            vars.iter()
                .map(|var| {
                    let named = if vars.len() > 1 {
                        format!("{name} ({var})")
                    } else {
                        name.clone()
                    };
                    // its stdout is the value, held for the call alone
                    let (asked, usage, said) = match run(&bin, &["get", &name, var], &[]) {
                        Ok(value) => check(value.trim(), owner, now, run),
                        Err(e) => (
                            judged(&e),
                            NO_READING.into(),
                            format!("bin/secret get: {}", first_line(&e)),
                        ),
                    };
                    // bin/secret's expiry holds when no provider answers (/code-review of rust-access-usage)
                    let status = match (status, asked) {
                        (Status::Expired, Status::Unknown) => Status::Expired,
                        _ => asked,
                    };
                    row(named, (status, usage, said))
                })
                .collect()
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
/// value is hashed in memory to tell copies apart and never kept; a Scaleway secret is held for its provider's call alone.
fn tfvars(repos: &Path, home: &Path, run: Run) -> Vec<Access> {
    let (mut files, mut refused) = (vec![], vec![]);
    walk(repos, DEPTH, &mut files, &mut refused);
    files.sort();
    let mut seen: BTreeMap<(String, u64), (BTreeSet<String>, Vec<String>)> = BTreeMap::new();
    // a Scaleway secret_key's value and organization, by its group
    let mut scw: BTreeMap<(String, u64), (String, String)> = BTreeMap::new();
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
        let lines: Vec<(&str, &str)> = text.lines().filter_map(assignment).collect();
        let value_of = |name: &str| lines.iter().find(|(k, _)| *k == name).map(|(_, v)| *v);
        // Scaleway's: its access key opens on SCW, beside the secret and the organization (klar's infra/*.tfvars)
        let organization = value_of("access_key")
            .filter(|a| a.starts_with("SCW"))
            .and(value_of("organization_id"));
        for &(key, value) in &lines {
            if !is_credential(key) {
                continue;
            }
            let mut hasher = DefaultHasher::new();
            value.hash(&mut hasher);
            let group = (key.to_string(), hasher.finish());
            if let (Some(org), "secret_key") = (organization, key) {
                scw.entry(group.clone())
                    .or_insert((value.to_string(), org.to_string()));
            }
            let (in_repos, places) = seen.entry(group).or_default();
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
        .map(|(group, (in_repos, places))| {
            let key = &group.0;
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
            let (status, usage, said) = match scw.get(&group) {
                Some((secret, org)) => scaleway(secret, org, run),
                None => (Status::Unknown, NO_READING.into(), unasked(key)),
            };
            Access {
                source: "tfvars",
                provider: tfvars_provider(key, &places),
                name,
                note: format!("in {} file(s); {said}", places.len()),
                place: places.join(", "),
                status,
                usage,
            }
        })
        .collect();
    rows.extend(refused.iter().map(|dir| Access {
        source: "tfvars",
        provider: "unreadable".into(),
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
    match run("gh", &["api", &path, "--jq", jq], &[]) {
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
            [] if denied => (Status::Refused, unasked(cli.name)),
            [] => (Status::Unknown, unasked(cli.name)),
            [bin, args @ ..] => {
                let answer = run(bin, args, &[]);
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
            provider: provider(cli.name),
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
        provider: "github".into(),
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
        &[],
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
            &[],
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
        provider: key_provider(&name).unwrap_or_else(|| "github".into()),
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
    let mut rows = secrets(repo, &owner, run, now);
    rows.extend(tfvars(&home.join("repo"), home, run));
    rows.extend(clis(home, &owner, &head(now, 7), run));
    rows.extend(github(&owner, run));
    rows
}

/// The rows as the page's tree, a card a provider and a line an access: the HTML and the text are both drawn from it, the
/// text one titled block of table lines a provider (task rust-tools-human).
fn tree(rows: &[Access], note: &str) -> String {
    let text = |t: String| json!({"type": "text", "props": {"text": t}});
    let mut elements = Map::new();
    elements.insert(
        "title".into(),
        json!({"type": "heading", "props": {"text": "Tools"}}),
    );
    elements.insert("note".into(), text(note.into()));
    let mut by: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, row) in rows.iter().enumerate() {
        by.entry(&row.provider).or_default().push(i);
    }
    let mut cards = vec![];
    for (p, (provider, at)) in by.into_iter().enumerate() {
        let mut lines = vec![];
        for i in at {
            let id = format!("access-{i}");
            elements.insert(id.clone(), rows[i].element());
            lines.push(id);
        }
        let id = format!("provider-{p}");
        elements.insert(
            id.clone(),
            json!({"type": "section", "props": {"title": provider}, "children": lines}),
        );
        cards.push(id);
    }
    elements.insert(
        "providers".into(),
        json!({"type": "cards", "children": cards}),
    );
    elements.insert(
        "page".into(),
        json!({"type": "stack", "children": ["title", "note", "providers"]}),
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

/// A command's stdout, `env` added to its environment, its failure without the « <bin> <args>: » it opens on.
fn asked(bin: &str, args: &[&str], env: &[(&str, &str)]) -> Result<String, String> {
    let what = format!("{bin} {}:", args.join(" "));
    match dash_process::answer(bin, args, None, env, Duration::from_secs(20)) {
        Ok((status, text, _)) if status.success() => Ok(text),
        Ok((status, text, reason)) => Err(format!("{status}: {} {}", reason.trim(), text.trim())),
        Err(e) => {
            let e = String::from(e);
            Err(e
                .strip_prefix(&what)
                .map_or(e.clone(), |why| why.trim().to_string()))
        }
    }
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

/// /tools' page, whose text is what `dash access` prints, and whether a read has ended: before the first, the page says it is
/// reading, and the server answers an agent 202.
pub fn page(home: &Path) -> Result<(surface::Drawing, bool), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let home = home.to_path_buf();
    let read = fresh::last(&READ, now, "access", move || live(&home))?;
    Ok(match read {
        Some((rows, at)) => (drawing(&rows, &fresh::ago(now, at)), true),
        None => (
            drawing(&[], "Reading every access: the next reload shows it."),
            false,
        ),
    })
}

fn drawing(rows: &[Access], note: &str) -> surface::Drawing {
    surface::Drawing::Tree {
        title: "Tools".into(),
        tree: tree(rows, note),
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

    fn providers(bin: &str, args: &[&str], _env: &[(&str, &str)]) -> Result<String, String> {
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
        let depot = row("bin/secret", "depot");
        assert!(depot.note.ends_with(&unasked("depot")), "{depot:?}");
        // a refusal is read from its words
        assert_eq!(judged("HTTP 401: Bad credentials"), Status::Expired);
        assert_eq!(judged("HTTP 403: Forbidden"), Status::Refused);
        assert_eq!(judged("gh api user: no answer in 20s"), Status::Unknown);
        // a missing CLI is unknown, and its command's words never reach the judgement
        let missing = asked("/no/such/token-403-cli", &["x"], &[]);
        assert!(
            !missing.as_ref().unwrap_err().contains("token-403"),
            "{missing:?}"
        );
        assert_eq!(judged(missing.as_ref().unwrap_err()), Status::Unknown);
        assert_eq!(head("2026-11-é8T00", 10), "2026-11-é8");
    }

    /// Done-when 1 and 2 of rust-page-forms, and rust-tools-human's: the server's `drawn` makes /tools' one drawing HTML or
    /// text; the HTML a card a provider, each access its status chip first, then usage, note and place; the text the table.
    #[test]
    fn tools_is_a_card_a_provider_in_html_and_the_table_in_text() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("access-views");
        let rows = rows(&dir.0);
        let form = |html| crate::drawn(html, Some(Ok(drawing(&rows, "Read 0 min ago."))));
        let ((_, html_kind, html), (_, text_kind, text)) = (form(true), form(false));
        assert_eq!((html_kind, text_kind), (crate::HTML, crate::TEXT));
        // a provider each: the shared Scaleway key of graal's terraform.tfvars and klar's scaleway.auto.tfvars, the gh CLI
        let of = |name: &str| {
            rows.iter()
                .find(|r| r.name == name)
                .map(|r| r.provider.as_str())
        };
        assert_eq!(of("graal, klar: secret_key"), Some("scaleway"));
        assert_eq!(of("gh"), Some("github"));
        assert_eq!(of("scw"), Some("scaleway"));
        assert_eq!(of("anthropic"), Some("anthropic"));
        // a key any provider uses takes its file's, in any case; a GitHub secret named so, GitHub's (/code-review)
        let scw = ["~/repo/k/scaleway.auto.tfvars".to_string()];
        assert_eq!(tfvars_provider("SECRET_KEY", &scw), "scaleway");
        assert_eq!(tfvars_provider("bunnynet_api_key", &scw), "bunnynet");
        assert_eq!(key_provider("API_TOKEN"), None);
        assert_eq!(key_provider("FLY_API_TOKEN").as_deref(), Some("fly"));
        let providers: BTreeSet<&str> = rows.iter().map(|r| r.provider.as_str()).collect();
        assert!(providers.len() < rows.len(), "{providers:?}");
        // the text: the heading, the note, a title a provider, a line an access
        let said = text.lines().filter(|l| !l.is_empty()).count();
        assert_eq!(said, 2 + providers.len() + rows.len(), "{text}");
        assert!(text.contains("\nscaleway\n"), "{text}");
        // the HTML: the providers a grid of cards, a card a provider holding its accesses (its tags are surface's to test)
        assert_eq!(
            html.matches(r#"class="issue-part""#).count(),
            providers.len(),
            "{html}"
        );
        assert!(html.contains(r#"class="cards" id="providers""#), "{html}");
        let escaped = |cell: &str| {
            cell.replace('&', "&#38;")
                .replace('<', "&#60;")
                .replace('>', "&#62;")
                .replace('"', "&#34;")
        };
        let providers: Vec<&str> = providers.into_iter().collect();
        for (i, row) in rows.iter().enumerate() {
            let cells = [
                row.name.as_str(),
                row.status.word(),
                &row.usage,
                &row.note,
                &row.place,
                row.source,
            ];
            let line = cells.join("\t");
            // six fields a line, split back whole
            assert!(text.lines().any(|l| l == line), "{row:?}");
            assert_eq!(line.split('\t').count(), 6, "{line}");
            // on its provider's card, its chip first, then the name, usage, note, source and place
            let p = providers
                .iter()
                .position(|p| *p == row.provider)
                .expect("provider");
            let card = |p: usize| html.find(&format!(r#"id="provider-{p}""#));
            let (from, to) = (card(p).expect("card"), card(p + 1).unwrap_or(html.len()));
            let status = row.status.word();
            let parts = [
                format!(r#"id="access-{i}" class="item access" data-status="{status}""#),
                format!(r#"class="badge chip">{status}"#),
                format!(r#"class="name">{}"#, escaped(&row.name)),
                format!(r#"class="usage">{}"#, escaped(&row.usage)),
                format!(r#"class="note">{}"#, escaped(&row.note)),
                format!(r#"class="source">{}"#, escaped(row.source)),
                format!(" {}", escaped(&row.place)),
            ];
            let mut at = from;
            for part in parts {
                let found = html[at..].find(&part);
                at += found.unwrap_or_else(|| panic!("{part} after {at}: {row:?}")) + 1;
            }
            // all of it inside its own access: before the next access, or its card's end
            let next = html[from..]
                .find(&format!(r#"id="access-{i}""#))
                .map(|s| from + s);
            let end = next
                .and_then(|s| html[s + 1..].find(r#"id="access-"#).map(|e| s + 1 + e))
                .unwrap_or(to)
                .min(to);
            assert!(at <= end, "{row:?} past its access");
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
            let run = move |_: &str, _: &[&str], _: &[(&str, &str)]| Ok(list.to_string());
            secrets(Path::new("/r"), "acme", &run, "2026-11-08T12:00:00")[0].status
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
        let rows = tfvars(&repos, &dir.0, &providers);
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
        row.note = "gh: error\tjq\nretry".into();
        let text = surface::text(&tree(&[row], "")).expect("text");
        let line = text
            .lines()
            .find(|l| l.starts_with("a: api_token"))
            .expect("line");
        assert_eq!(line.split('\t').count(), 6, "{line}");
    }

    /// Stand-in providers (task rust-access-usage): a key whose value holds « good » is let in, « refused » is refused, any
    /// other is dead. The body, then curl's code on its own line, as `-w` writes it.
    fn http_stand_in(args: &[&str], key: &str) -> String {
        let url = *args.last().expect("url");
        // a FlyV1 token goes as it is, any other after Bearer
        let fly_v1 = args.contains(&"Authorization: {{KEY}}");
        if url.contains("api.fly.io") && key.starts_with("FlyV1 ") != fly_v1 {
            return "bad header\n400".into();
        }
        let admin = key.contains("admin");
        let (code, body) = if key.contains("refused") {
            (403, r#"{"message":"insufficient permissions"}"#)
        } else if !key.contains("good") && url.contains("api.fly.io") {
            (200, r#"{"errors":[{"message":"You must be authenticated","extensions":{"code":"UNAUTHORIZED"}}],"data":null}"#)
        } else if !key.contains("good") {
            (401, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#)
        } else if url.contains("/v1/models") && admin {
            (401, r#"{"error":{"message":"invalid x-api-key"}}"#)
        } else if url.contains("/v1/models") {
            (200, r#"{"data":[]}"#)
        } else if url.contains("cost_report") && !admin {
            (401, r#"{"error":{"message":"The Admin API requires an Admin API key"}}"#)
        } else if url.contains("cost_report?starting_at=2026-11-01T00:00:00Z") {
            (200, r#"{"data":[{"results":[{"amount":"1234.5","currency":"USD"}]},{"results":[{"amount":"100"}]}]}"#)
        } else if url.contains("api.fly.io/graphql") {
            (200, r#"{"data":{"organizations":{"nodes":[{"slug":"personal","billingStatus":"CURRENT","creditBalanceFormatted":"$0.00","apps":{"totalCount":1}}]}}}"#)
        } else if url.ends_with("/v2/me") {
            (200, r#"{"login":"nha"}"#)
        } else if url.contains("insights/gh/acme/summary") {
            (200, r#"{"all_projects":["site_perso","cli"]}"#)
        } else if url.contains("insights/gh/acme/site_perso/workflows?all-branches=true&start-date=2026-11-01T00:00:00Z&end-date=2026-11-08T12:00:00Z") {
            (200, r#"{"items":[{"metrics":{"total_credits_used":400}},{"metrics":{"total_credits_used":29}}]}"#)
        } else if url.contains("insights/gh/acme/cli/workflows") {
            (200, r#"{"items":[{"metrics":{"total_credits_used":1}}]}"#)
        } else if url.contains("instance/v1/zones/fr-par-1/servers") && url.contains("organization=org-1") {
            (200, r#"{"servers":[]}"#)
        } else if url.contains("billing/v2beta1/consumptions?organization_id=org-1") {
            (200, r#"{"consumptions":[{"value":{"currency_code":"EUR","units":3,"nanos":500000000}},{"value":{"currency_code":"EUR","units":1,"nanos":0}}]}"#)
        } else {
            (404, "not a stand-in")
        };
        format!("{body}\n{code}")
    }

    #[test]
    fn each_provider_says_works_expired_or_refused_and_this_month_s_usage() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("access-usage");
        let tf = |access: &str, secret: &str| {
            format!(
                "access_key = \"{access}\"\nsecret_key = \"{secret}\"\norganization_id = \"org-1\"\n"
            )
        };
        write(
            &dir.0.join("repo/klar/infra/scaleway.auto.tfvars"),
            &tf("SCWGOOD", "scw-good-STAND-IN"),
        );
        write(
            &dir.0.join("repo/klar/old/scaleway.auto.tfvars"),
            &tf("SCWOLD", "scw-refused-STAND-IN"),
        );
        // an access key not Scaleway's is never sent there
        write(
            &dir.0.join("repo/other/x.tfvars"),
            &tf("AKIA1", "aws-good-STAND-IN"),
        );
        write(
            &dir.0.join("repo/site_perso/forge.env"),
            "GITHUB_OWNER=acme\n",
        );
        let values = [
            ("anthropic ANTHROPIC_API_KEY", "sk-good-STAND-IN"),
            ("anthropic ANTHROPIC_API_KEY_OLD", "sk-dead-STAND-IN"),
            (
                "anthropic ANTHROPIC_ADMIN_KEY",
                "sk-ant-admin-good-STAND-IN",
            ),
            ("fly FLY_API_TOKEN", "FlyV1 fm2_dead-STAND-IN"),
            ("circleci CIRCLE_API_TOKEN", "cci-good-STAND-IN"),
        ];
        let argv = Mutex::new(vec![]);
        let run = |bin: &str, args: &[&str], env: &[(&str, &str)]| {
            argv.lock().expect("argv").push(args.join(" "));
            match (bin, args) {
                (b, ["list", "--json"]) if b.ends_with("bin/secret") => Ok(json!([
                    {"name": "anthropic", "kind": "API key", "source": "a.env", "state": "ok", "keys": ["ANTHROPIC_API_KEY", "ANTHROPIC_API_KEY_OLD", "ANTHROPIC_ADMIN_KEY"]},
                    {"name": "fly", "kind": "unknown", "source": "f.env", "state": "ok", "keys": ["FLY_API_TOKEN"]},
                    {"name": "circleci", "kind": "token", "source": "c.env", "state": "ok", "keys": ["CIRCLE_API_TOKEN"]},
                    {"name": "warp", "kind": "API key", "source": "w.env", "state": "ok", "keys": ["WARP_API_KEY"]}
                ])
                .to_string()),
                (b, ["get", name, var]) if b.ends_with("bin/secret") => {
                    let asked = format!("{name} {var}");
                    let value = values.iter().find(|(k, _)| *k == asked).map(|(_, v)| *v);
                    value.map(|v| format!("{v}\n")).ok_or("no such key".to_string())
                }
                ("curl", _) => {
                    let key = env.iter().find(|(k, _)| *k == "KEY").expect("the key in curl's environment").1;
                    Ok(http_stand_in(args, key))
                }
                _ => Err("not a stand-in".into()),
            }
        };
        let rows = read(
            &dir.0,
            &dir.0.join("repo/site_perso"),
            "2026-11-08T12:00:00",
            &run,
        );
        let row = |name: &str| {
            rows.iter()
                .find(|r| r.name == name)
                .unwrap_or_else(|| panic!("{name}: {rows:#?}"))
                .clone()
        };
        let said = |r: &Access| (r.status, r.usage.clone());
        // Anthropic: an Admin key reads its cost report in dollars though it lists no models; a live key says why it reads
        // none; a dead one expired, unread
        let admin = row("anthropic (ANTHROPIC_ADMIN_KEY)");
        let dollars = (Status::Works, "$13.35 this month".into());
        assert_eq!(said(&admin), dollars, "{admin:?}");
        let live = row("anthropic (ANTHROPIC_API_KEY)");
        let why = "no reading: HTTP 401: The Admin API requires an Admin API key";
        assert_eq!(said(&live), (Status::Works, why.into()), "{live:?}");
        let dead = row("anthropic (ANTHROPIC_API_KEY_OLD)");
        assert_eq!(
            said(&dead),
            (Status::Expired, NO_READING.into()),
            "{dead:?}"
        );
        assert!(
            dead.note.ends_with("HTTP 401: invalid x-api-key"),
            "{dead:?}"
        );
        // Fly: a 200 holding UNAUTHORIZED is a dead token
        assert_eq!(row("fly").status, Status::Expired, "{:?}", row("fly"));
        // CircleCI: the month's credits over the owner's projects
        let circleci = row("circleci");
        assert_eq!(
            said(&circleci),
            (Status::Works, "430 credits this month (gh/acme)".into()),
            "{circleci:?}"
        );
        assert!(circleci.note.ends_with("CircleCI says nha"), "{circleci:?}");
        // Scaleway: klar's tfvars key asked with its organization; a refused key says so, its words kept
        let scw = rows
            .iter()
            .filter(|r| r.name.starts_with("klar: secret_key"))
            .collect::<Vec<_>>();
        let by = |place: &str| scw.iter().find(|r| r.place.contains(place)).expect(place);
        assert_eq!(
            said(by("infra")),
            (Status::Works, "4.50 EUR this month".into()),
            "{scw:?}"
        );
        assert_eq!(by("old").status, Status::Refused, "{scw:?}");
        assert!(
            by("old")
                .note
                .ends_with("HTTP 403: insufficient permissions"),
            "{scw:?}"
        );
        let other = row("other: secret_key");
        assert_eq!(other.status, Status::Unknown, "{other:?}");
        // never a value on a command line, nor in a row
        let argv = argv.lock().expect("argv").join("\n");
        let printed = format!("{rows:?}");
        for (_, value) in values {
            assert!(!argv.contains(value) && !printed.contains(value), "{value}");
        }
        assert!(
            !argv.contains("STAND-IN") && !printed.contains("STAND-IN"),
            "{argv}"
        );
    }

    /// Part 2 of rust-access-usage: no row reads a bare unknown, its note says why no provider answered.
    #[test]
    fn a_row_no_provider_answers_says_why() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("access-why");
        let mut rows = rows(&dir.0);
        let listed =
            json!([{"name": "fly", "kind": "token", "source": "f.env", "state": "ok", "keys": []}]);
        let run = move |_: &str, _: &[&str], _: &[(&str, &str)]| Ok(listed.to_string());
        rows.extend(secrets(
            Path::new("/r"),
            "acme",
            &run,
            "2026-11-08T12:00:00",
        ));
        let unknown: Vec<&Access> = rows
            .iter()
            .filter(|r| r.status == Status::Unknown)
            .collect();
        assert!(unknown.len() > 5, "{rows:#?}");
        for row in unknown {
            let why = row.note.rsplit("; ").next().unwrap_or_default();
            assert!(
                why.len() > 12 && why != NOT_ASKED && !why.eq_ignore_ascii_case("unknown"),
                "{row:?}"
            );
        }
        let fly = rows.iter().find(|r| r.place == "f.env").expect("fly");
        assert!(
            fly.note.ends_with("bin/secret lists no key in it"),
            "{fly:?}"
        );
        // curl timing out after a body holding « 403 » is no answer: bin/secret's own expiry stands, else unknown
        let timed_out = |expires: &str| {
            let listed = json!([{"name": "fly", "kind": "token", "source": "f.env", "state": "ok", "expires": expires, "keys": ["FLY_API_TOKEN"]}]);
            let run = move |_: &str, args: &[&str], _: &[(&str, &str)]| match args {
                ["list", "--json"] => Ok(listed.to_string()),
                ["get", ..] => Ok("fly-good-STAND-IN".into()),
                _ => Err("exit status: 28: Operation timed out {\"code\":403}\n403".into()),
            };
            let row = secrets(Path::new("/r"), "acme", &run, "2026-11-08T12:00:00").remove(0);
            assert!(row.note.contains("Operation timed out"), "{row:?}");
            row.status
        };
        assert_eq!(timed_out("2026-11-01"), Status::Expired);
        assert_eq!(timed_out(""), Status::Unknown);
    }
}
