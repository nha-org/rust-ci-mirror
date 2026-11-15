//! A CI runner on a Hostman Cloud Server that stays up (task rust-hostman-ci-runner; service-tool step 5; @nha, 2026-11-14:
//! « We should use their cloud server to run CI. Cloud server only the tool should prevent anything else. And then kill the
//! whatsnr VPS. », « If CI is the bottleneck... use hostman as I asked. »).
//!
//! The desired state is one `Want`: a Cloud Server of that name, on, with a GitHub runner of that label online. `reconcile`
//! observes it through the `hostman` tool's JSON (a closure, so this crate reaches no other plugin), repairs what it can
//! (create, start, a public IPv4, a reinstall an hour) and otherwise says `Down` with its reason. `Runner` is the core's job model: a tick of the
//! queue (`dash_jobs`, the `Recurring` job `live` registers, task rust-hostman-ci-wire) runs it and its rows come back through the write path, so a status is data and an alarm is written
//! once per reason, never again while it stands and never silently. `route` is the router's tier order with this runner
//! before the Mac and the whatsnr VPS gone from it.
//! ponytail: one server, one runner; a pool is a `Vec<Want>` once CI needs more than `nextest -j 2` jobs at a time.
//! ponytail: one box reconciled by hand; how we manage infra next is weighed in private/docs/rfc-iac.html (@nha, 2026-11-15:
//! « I'd like an RFC there. super light. just compare and contrast. »).
use dash_core::{FieldWrite, Value};
use serde_json::{Value as Json, json};
use std::cell::RefCell;

pub mod live;
pub mod route;

/// The label the workflows ask for: alone, with no `self-hosted`, so a job takes the hosted path and no private/ reaches it.
/// Nothing in it names this repository: the box runs klar's CI too (@nha, 2026-11-15: « site-perso-ci => just ci. We will
/// run klar CI too. »). The old label, site-perso-hostman, stays on the whatsnr VPS runners until this one is online.
pub const LABEL: &str = "ci";
pub const MODEL: &str = "hostman-ci-runner";
/// The OS installed, as Hostman's list names it (@nha, 2026-11-15: « don't pick ubuntu. They are SLOW to patch any security
/// issue. debian. or alpine. »): the newest Debian Hostman lists, read at each create or reinstall (11 and 12 on
/// 2026-11-15, no 13); a list with no Debian makes and wipes nothing. Alpine's musl runs no glibc binary. Debian 12's glibc
/// is 2.36 and the mirror builds main's dash on ubuntu-24.04 (2.39, rust/.github/workflows/ci.yml): a job that runs that
/// binary on 12 fails if it asks a newer symbol.
pub const OS: &str = "debian";
/// How long a server may be on with no runner online before it is reinstalled with a fresh token: cloud-init's apt and the
/// runner's download take minutes (/code-review). A reboot healed nothing here: cloud-init runs once an instance, so a
/// failed one stays failed, and the runner's service already restarts itself.
pub const HEAL_MS: i64 = 15 * 60_000;
/// At most one reinstall in this long; past it with no runner, one alarm.
pub const REINSTALL_MS: i64 = 3_600_000;

/// The state kept true.
#[derive(Clone, Debug)]
pub struct Want {
    pub name: String,
    /// Names the server had before: one found under one is renamed and reinstalled, never a second made beside it.
    pub was: Vec<String>,
    /// The RAM a preset must hold, in MB: the create takes the cheapest such preset whose location has a free node.
    pub ram_mb: i64,
    /// The OS installed, its name in Hostman's list: its newest version there.
    pub os: &'static str,
    pub ssh_keys_ids: Vec<i64>,
    /// `owner/name` the runner registers to.
    pub repo: String,
}

impl Want {
    /// The one server wanted (@nha, 2026-11-14: « take one 8GB machine please. »): a preset with 8 GB of RAM, the cheapest in
    /// Hostman's live list whose location has a free node. Read 2026-11-15, five hold 8 GB: 5133 pl-1 at 15 USD a month,
    /// then 5145 us-2, 5157 nl-1, 5515 us-3 and 6273 de-1 at 20; pl-1 answered 409 no_free_node at the first create. `OS`, no
    /// ssh key, the runner needs none. It was site-perso-ci, created on Ubuntu 24.04 (server 9602127, 2026-11-15).
    /// Hostman's Terraform is not used: this tool calls the API, so its Cloud-Servers-only scope and the credit cap hold
    /// before each call, which a Terraform plan would go round.
    pub fn eight_gb(repo: &str) -> Self {
        Want {
            name: "ci".into(),
            was: vec!["site-perso-ci".into()],
            ram_mb: 8192,
            os: OS,
            ssh_keys_ids: vec![],
            repo: repo.into(),
        }
    }
}

/// What one reconcile did.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Up,
    Created(i64),
    Started(i64),
    /// On, its runner not online yet: it is booting or registering.
    Waiting(i64),
    /// Hostman at work on it: its status, such as installing.
    Busy(i64, String),
    /// A public IPv4 added: it had none, so it reached nothing.
    Addressed(i64),
    /// Reinstalled with a fresh registration token.
    Reinstalled(i64),
    /// No server made: the preset's location had no free node; the words name it and the preset the next tick tries.
    NoNode(String),
    /// Not repairable here: the reason, which is the alarm.
    Down(String),
}

impl Step {
    pub fn text(&self) -> String {
        match self {
            Step::Up => "up".into(),
            Step::Created(id) => format!("created server {id}"),
            Step::Started(id) => format!("started server {id}"),
            Step::Waiting(id) => format!("waiting: server {id} is on, {NO_RUNNER} yet"),
            Step::Busy(id, status) => format!("waiting: server {id} is {status}"),
            Step::Addressed(id) => {
                format!(
                    "addressed server {id}: it had no public IP, so its cloud-init reached nothing"
                )
            }
            Step::Reinstalled(id) => {
                format!("reinstalled server {id} with a fresh registration token")
            }
            Step::NoNode(said) => format!("no free node: {said}"),
            Step::Down(why) => format!("down: {why}"),
        }
    }
}

/// One call of the `hostman` tool: its arguments as JSON text, its answer.
pub type Call<'a> = &'a dyn Fn(&str) -> Result<String, String>;
/// The owned forms the job model keeps.
pub type Tool = Box<dyn Fn(&str) -> Result<String, String>>;
pub type Read<T> = Box<dyn Fn() -> Result<T, String>>;

fn ask(call: Call, method: &str, path: &str, body: Json) -> Result<Json, String> {
    let asked = json!({"method": method, "path": path, "body": body}).to_string();
    serde_json::from_str(&call(&asked)?).map_err(|_| format!("hostman: {path} answered no JSON"))
}

/// The words every status of a server on with no runner holds, by which `live` finds when that began after a restart.
pub const NO_RUNNER: &str = "no runner online";

/// The cloud-init a create or a reinstall sends: the runner installed, registered with `token` (valid an hour) as `name`
/// under `LABEL` alone, its tests on two threads (@nha, 2026-11-14: « cargo nextest -j 2 »). The audit log keeps a body, so
/// the token is one that is dead by the time anyone reads it. The release is GitHub's latest, read on the box: since
/// 2026-09-29 GitHub refuses to register a runner older than v2.329.0, and the 2.321.0 written here was one, so a pin goes
/// stale the same way. ICU is the release's own (libicu76 on Debian 13, libicu72 on 12): the runner's installdependencies.sh
/// asks libicu72 and older alone, its .NET does not start without one, and a name the release lacks in `packages` would fail
/// the whole list. `sudo`, which a Debian image may lack, runs config.sh as the runner, which refuses root. Paths are
/// absolute: no `~` that rests on sudo's HOME.
pub fn cloud_init(repo: &str, name: &str, token: &str) -> String {
    let get = "v=$(curl -fsSL https://api.github.com/repos/actions/runner/releases/latest | jq -r .tag_name) && curl -fsSL \
               -o r.tgz https://github.com/actions/runner/releases/download/$v/actions-runner-linux-x64-${v#v}.tar.gz";
    let config = format!(
        "sudo -u runner ./config.sh --unattended --no-default-labels --labels {LABEL} --name {name} --url \
         https://github.com/{repo} --token {token} --replace"
    );
    let at = "cd /home/runner/actions &&";
    format!(
        "#cloud-config\npackage_update: true\npackages: [build-essential, pkg-config, libssl-dev, libfontconfig1, sudo, git, curl, jq, unzip, zstd, python3, docker.io]\n\
         runcmd:\n  - apt-get install -y libicu76 || apt-get install -y libicu72\n  \
         - useradd -m -s /bin/bash runner && usermod -aG docker runner\n  \
         - mkdir -p /home/runner/actions && {at} {get} && tar -xzf r.tgz && rm r.tgz\n  \
         - /home/runner/actions/bin/installdependencies.sh\n  \
         - {at} echo NEXTEST_TEST_THREADS=2 >> .env && chown -R runner:runner . && {config}\n  \
         - {at} ./svc.sh install runner && ./svc.sh start\n"
    )
}

/// Whether Hostman lists a public network on `server` with no IP in it: such a server reaches no apt, no GitHub. A server
/// with no network list is no reading, never « no IP ».
fn unaddressed(server: &Json) -> bool {
    let mut nets = server["networks"].as_array().into_iter().flatten();
    nets.any(|n| n["type"] == "public" && n["ips"].as_array().is_some_and(Vec::is_empty))
}

/// Hostman's id of the newest `name` in its live list, versions compared part by part (13 over 12, 3.24 over 3.9): an id
/// written here is a guess the day the list changes. Ubuntu is refused whatever the list holds (@nha, 2026-11-15: « don't
/// pick ubuntu. »).
fn os_id(call: Call, name: &str) -> Result<i64, String> {
    if name == "ubuntu" {
        return Err(
            "Ubuntu refused (@nha, 2026-11-15: « don't pick ubuntu. »): want Debian".into(),
        );
    }
    let listed = ask(call, "GET", "/api/v1/os/servers", Json::Null)?;
    let version = |o: &Json| -> Vec<u64> {
        let v = o["version"].as_str().unwrap_or_default();
        v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
    };
    let all = listed["servers_os"].as_array().into_iter().flatten();
    all.filter(|o| o["name"] == name && o["id"].is_i64())
        .max_by_key(|o| version(o))
        .and_then(|o| o["id"].as_i64())
        .ok_or(format!(
            "hostman: its list holds no {name}, so no server is created or reinstalled"
        ))
}

/// Whether `server` is the one wanted: its name, and its OS's name as Hostman lists it (any version: a newer one listed
/// later is no reason to wipe a box that works).
fn as_wanted(server: &Json, want: &Want) -> (bool, bool) {
    let named = server["name"] == want.name.as_str();
    (named, server["os"]["name"] == want.os)
}

/// How long a preset and its location that answered « no free node » are not asked again.
pub const NO_NODE_MS: i64 = 3_600_000;

/// A create Hostman refused for want of a free node: the preset, its location, and when.
#[derive(Clone, Debug, PartialEq)]
pub struct NoNode {
    pub preset: i64,
    pub location: String,
    pub at_ms: i64,
}

/// The presets a create may take, from the live list: Cloud Servers presets holding `ram_mb` with no GPU, cheapest first
/// (ties by id), none whose preset or location answered « no free node » in `refused`.
pub fn candidates<'a>(presets: &'a Json, ram_mb: i64, refused: &[NoNode]) -> Vec<&'a Json> {
    let gone = |p: &Json| {
        (refused.iter())
            .any(|r| p["id"].as_i64() == Some(r.preset) || p["location"] == r.location.as_str())
    };
    let mut fit: Vec<&Json> = (presets["server_presets"].as_array().into_iter().flatten())
        .filter(|p| p["ram"].as_i64() == Some(ram_mb) && p["gpu"].as_f64().is_none_or(|g| g == 0.0))
        .filter(|p| p["price"].as_f64().is_some() && p["id"].as_i64().is_some() && !gone(p))
        .collect();
    fit.sort_by(|a, b| {
        let key = |p: &Json| {
            (
                p["price"].as_f64().unwrap_or(f64::MAX),
                p["id"].as_i64().unwrap_or(0),
            )
        };
        key(a)
            .partial_cmp(&key(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    fit
}

/// A preset as its words say it: « 5133 in pl-1 ».
fn named(p: &Json) -> String {
    format!("{} in {}", p["id"], p["location"].as_str().unwrap_or("?"))
}

/// A create's month at `price` against the credit left, read before the create: refused with both numbers when it would pass.
fn within_credit(call: Call, preset_id: i64, price: f64) -> Result<(), String> {
    let f = &ask(call, "GET", "/api/v1/account/finances", Json::Null)?["finances"];
    let (Some(left), Some(running)) = (f["balance"].as_f64(), f["monthly_cost"].as_f64()) else {
        return Err("hostman: its finances hold no balance, so no create is priced".into());
    };
    match running + price > left {
        true => Err(format!(
            "no server created: preset {preset_id} costs {price:.2} a month, {:.2} with the servers running, past the \
             {left:.2} of credit left",
            running + price
        )),
        false => Ok(()),
    }
}

/// What a reconcile remembers from one tick to the next; `live` seeds the two times from its own log after a restart, so
/// a deploy every ten minutes never holds the reinstall off.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    /// Since when the server has been on with no runner online.
    pub since: Option<i64>,
    /// When it was last reinstalled.
    pub installed: Option<i64>,
    /// The creates refused for want of a free node, kept an hour.
    pub refused: Vec<NoNode>,
}

/// One observation and the repair it allows. `online`: whether the runner `want.name` is online on GitHub with `LABEL`;
/// `token`: a fresh registration token, asked only at a create or a reinstall. One create or reinstall a tick at most; a
/// box of the wrong name or OS is reinstalled at once, and the right one only when GitHub said it has no runner: a GitHub,
/// or a resolver, that does not answer never wipes it.
pub fn reconcile(
    call: Call,
    online: &dyn Fn() -> Result<bool, String>,
    token: &dyn Fn() -> Result<String, String>,
    want: &Want,
    state: &mut State,
    now_ms: i64,
) -> Step {
    let State {
        since,
        installed,
        refused,
    } = state;
    let mut step = || -> Result<Step, String> {
        let listed = ask(call, "GET", "/api/v1/servers", Json::Null)?;
        // an answer with no list is no reading, never « no server »: a create on it would make a second one (/code-review)
        let servers = listed["servers"]
            .as_array()
            .ok_or("hostman: its server list holds no servers, so none is created")?;
        // one under a former name is this one, renamed below: never a second beside it
        let ours = |s: &&Json| {
            let name = s["name"].as_str().unwrap_or_default();
            name == want.name || want.was.iter().any(|w| w == name)
        };
        let Some(server) = servers.iter().find(ours) else {
            *since = None;
            let os_id = os_id(call, want.os)?;
            refused.retain(|r| now_ms - r.at_ms < NO_NODE_MS);
            let presets = ask(call, "GET", "/api/v1/presets/servers", Json::Null)?;
            let Some(&p) = candidates(&presets, want.ram_mb, refused).first() else {
                return Err(match refused.is_empty() {
                    true => format!("hostman: no Cloud Servers preset holds {} MB of RAM", want.ram_mb),
                    // no time in it: one alarm while it lasts (a reason that changes each tick sounds each tick)
                    false => "no free node in any preset with that RAM: each is asked again an hour after it refused".into(),
                });
            };
            let (id, price) = (
                p["id"].as_i64().unwrap_or(0),
                p["price"].as_f64().unwrap_or(0.0),
            );
            within_credit(call, id, price)?;
            let body = json!({
                "name": want.name, "preset_id": id, "os_id": os_id, "bandwidth": p["bandwidth"],
                "is_ddos_guard": false, "ssh_keys_ids": want.ssh_keys_ids,
                "cloud_init": cloud_init(&want.repo, &want.name, &token()?),
            });
            let made = match ask(call, "POST", "/api/v1/servers", body) {
                // a 409 made nothing: the next tick lists none of that name and takes the next preset elsewhere
                Err(e) if e.contains("409") && e.contains("no_free_node") => {
                    let location = p["location"].as_str().unwrap_or_default().to_string();
                    refused.push(NoNode {
                        preset: id,
                        location,
                        at_ms: now_ms,
                    });
                    let next = candidates(&presets, want.ram_mb, refused)
                        .first()
                        .map(|n| named(n));
                    let next = next.map_or("none left within the hour".into(), |n| {
                        format!("the next tick tries {n}")
                    });
                    return Ok(Step::NoNode(format!("preset {} refused; {next}", named(p))));
                }
                made => made?,
            };
            let id = made["server"]["id"].as_i64();
            return id.map(Step::Created).ok_or("hostman: a create answered no server id, so none is counted: read GET /api/v1/servers".into());
        };
        let id = server["id"]
            .as_i64()
            .ok_or("hostman: a server with no id")?;
        let post = |path: &str, body: Json| {
            ask(call, "POST", &format!("/api/v1/servers/{id}{path}"), body)
        };
        let status = server["status"].as_str().unwrap_or("");
        if status != "on" {
            *since = None;
        }
        match status {
            "off" => {
                post("/start", Json::Null)?;
                Ok(Step::Started(id))
            }
            "no_paid" | "blocked" => {
                Err(format!("server {id} is {status}: his, in Hostman's panel"))
            }
            // creating, installing, turning on, rebooting: Hostman is at work, which is no reason to count or reinstall
            s if s != "on" => Ok(Step::Busy(id, s.to_string())),
            _ => {
                let (right_name, right_os) = as_wanted(server, want);
                let seen = online();
                if right_name && right_os && seen == Ok(true) {
                    *since = None;
                    return Ok(Step::Up);
                }
                // the OS's id before any spend: a list without it adds no IP and wipes nothing
                let os = match right_os {
                    true => None,
                    false => Some(os_id(call, want.os)?),
                };
                if unaddressed(server) {
                    post("/ips", json!({"type": "ipv4"}))?;
                    // its cloud-init ran with no network and runs once an instance: reinstalled on the next tick
                    *since = Some(now_ms - HEAL_MS);
                    return Ok(Step::Addressed(id));
                }
                // the right box waits for its runner; a wrong name or OS is reinstalled at once
                if right_name && right_os {
                    let on = *since.get_or_insert(now_ms);
                    if now_ms - on < HEAL_MS {
                        return Ok(Step::Waiting(id));
                    }
                    // no time or count in a reason: one that changes each tick sounds a new alarm each tick
                    if let Err(e) = seen {
                        return Err(format!(
                            "server {id}: GitHub's runner list cannot be read: {e}"
                        ));
                    }
                }
                if installed.is_some_and(|at| now_ms - at < REINSTALL_MS) {
                    return Err(format!(
                        "server {id}: {NO_RUNNER} after its reinstall; the next is an hour after the last"
                    ));
                }
                // one PATCH: its name, a fresh token, and a new OS, which reinstalls it; the same OS takes « install »
                let mut body = json!({"cloud_init": cloud_init(&want.repo, &want.name, &token()?)});
                if !right_name {
                    body["name"] = json!(want.name);
                }
                if let Some(os) = os {
                    body["os_id"] = json!(os);
                }
                let patched = ask(call, "PATCH", &format!("/api/v1/servers/{id}"), body);
                let sent = match (patched, os) {
                    (Ok(_), None) => post("/action", json!({"action": "install"})),
                    (patched, _) => patched,
                };
                // counted when it went through or may have; one never sent, or refused, is asked again the next tick
                if sent
                    .as_ref()
                    .map_or_else(|e| e.starts_with("hostman: no answer"), |_| true)
                {
                    *installed = Some(now_ms);
                }
                sent?;
                *since = None;
                Ok(Step::Reinstalled(id))
            }
        }
    };
    step().unwrap_or_else(Step::Down)
}

/// The job model the core's queue runs each tick: its rows are `ci_runner/<name>`'s `status` and, once per reason, `alarm`
/// (empty again when it is up, and only then: a reinstall or a wait between two Downs of one reason is no new alarm). Never
/// silent: a Down is a status and an alarm, and a Down of another reason a new alarm.
pub struct Runner {
    want: Want,
    call: Tool,
    online: Read<bool>,
    token: Read<String>,
    state: RefCell<State>,
    /// The alarm last written.
    alarmed: RefCell<String>,
    now: Box<dyn Fn() -> i64>,
}

impl Runner {
    pub fn new(want: Want, call: Tool, online: Read<bool>, token: Read<String>) -> Self {
        Self {
            want,
            call,
            online,
            token,
            state: RefCell::default(),
            alarmed: RefCell::default(),
            now: Box::new(|| 0),
        }
    }

    /// The clock a refusal's hour is counted on: the host's.
    pub fn clocked(mut self, now: Box<dyn Fn() -> i64>) -> Self {
        self.now = now;
        self
    }

    /// The state a restart starts from.
    pub fn seeded(self, state: State) -> Self {
        *self.state.borrow_mut() = state;
        self
    }

    pub fn tick(&self) -> Vec<FieldWrite> {
        let alarmed = &mut *self.alarmed.borrow_mut();
        let step = reconcile(
            &*self.call,
            &*self.online,
            &*self.token,
            &self.want,
            &mut self.state.borrow_mut(),
            (self.now)(),
        );
        let row = |field: &str, text: String| FieldWrite {
            table: "ci_runner".into(),
            row: self.want.name.clone(),
            field: field.into(),
            value: Value::Text(text),
        };
        let mut rows = vec![row("status", step.text())];
        let alarm = match &step {
            Step::Down(why) => why.clone(),
            Step::Up => String::new(),
            _ => alarmed.clone(),
        };
        if alarm != *alarmed {
            rows.push(row("alarm", alarm.clone()));
            *alarmed = alarm;
        }
        rows
    }
}

#[cfg(test)]
mod tests;
