//! A CI runner on a Hostman Cloud Server that stays up (task rust-hostman-ci-runner; service-tool step 5; @nha, 2026-11-14:
//! « We should use their cloud server to run CI. Cloud server only the tool should prevent anything else. And then kill the
//! whatsnr VPS. », « If CI is the bottleneck... use hostman as I asked. »).
//!
//! The desired state is one `Want`: a Cloud Server of that name, on, with a GitHub runner of that label online. `reconcile`
//! observes it through the `hostman` tool's JSON (a closure, so this crate reaches no other plugin), repairs what it can
//! (create, start, one reboot) and otherwise says `Down` with its reason. `Runner` is the core's job model: a tick of the
//! queue (`dash_jobs`, the `Recurring` job `live` registers, task rust-hostman-ci-wire) runs it and its rows come back through the write path, so a status is data and an alarm is written
//! once per reason, never again while it stands and never silently. `route` is the router's tier order with this runner
//! before the Mac and the whatsnr VPS gone from it.
//! ponytail: one server, one runner; a pool is a `Vec<Want>` once CI needs more than `nextest -j 2` jobs at a time.
use dash_core::{FieldWrite, Value};
use serde_json::{Value as Json, json};
use std::cell::RefCell;

pub mod live;
pub mod route;

/// The label the workflows ask for: alone, with no `self-hosted`, so a job takes the hosted path and no private/ reaches it.
pub const LABEL: &str = "site-perso-hostman";
/// The runner release the server installs.
const RUNNER: &str = "2.321.0";
pub const MODEL: &str = "hostman-ci-runner";
/// Ticks (a tick a minute) a server may be on with no runner online before one reboot, and before the alarm: cloud-init's
/// apt and the runner's download take minutes (/code-review).
const REBOOT_AT: u32 = 10;
const DOWN_AT: u32 = 20;

/// The state kept true.
#[derive(Clone, Debug)]
pub struct Want {
    pub name: String,
    /// The RAM a preset must hold, in MB: the create takes the cheapest such preset whose location has a free node.
    pub ram_mb: i64,
    /// The Ubuntu release installed, its id read from Hostman's list at the create.
    pub ubuntu: String,
    pub ssh_keys_ids: Vec<i64>,
    /// `owner/name` the runner registers to.
    pub repo: String,
}

impl Want {
    /// The one server wanted (@nha, 2026-11-14: « take one 8GB machine please. »): a preset with 8 GB of RAM, the cheapest in
    /// Hostman's live list whose location has a free node. Read 2026-11-15, five hold 8 GB: 5133 pl-1 at 15 USD a month,
    /// then 5145 us-2, 5157 nl-1, 5515 us-3 and 6273 de-1 at 20; pl-1 answered 409 no_free_node at the first create. Ubuntu 24.04, no ssh key, the runner needs none: main's dash, which the gate's jobs
    /// run, links the glibc of the mirror's ubuntu-24.04 (rust/.github/workflows/ci.yml), so 22.04 may not load it
    /// (/code-review of PR 1639).
    /// Hostman's Terraform is not used: this tool calls the API, so its Cloud-Servers-only scope and the credit cap hold
    /// before each call, which a Terraform plan would go round.
    pub fn eight_gb(repo: &str) -> Self {
        Want {
            name: "site-perso-ci".into(),
            ram_mb: 8192,
            ubuntu: "24.04".into(),
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
    Rebooted(i64),
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
            Step::Waiting(id) => {
                format!("waiting: server {id} is on, its runner is not online yet")
            }
            Step::Rebooted(id) => format!("rebooted server {id}: its runner was offline"),
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

/// The cloud-init a create sends: the runner installed, registered with `token` (valid an hour) under `LABEL` alone, its
/// tests on two threads (@nha, 2026-11-14: « cargo nextest -j 2 »). The audit log keeps a body, so the token is one that is
/// dead by the time anyone reads it.
pub fn cloud_init(repo: &str, token: &str) -> String {
    let url = format!(
        "https://github.com/actions/runner/releases/download/v{RUNNER}/actions-runner-linux-x64-{RUNNER}.tar.gz"
    );
    let config = format!(
        "./config.sh --unattended --no-default-labels --labels {LABEL} --url https://github.com/{repo} --token {token} --replace"
    );
    format!(
        "#cloud-config\npackage_update: true\npackages: [build-essential, pkg-config, libssl-dev, libfontconfig1, git, curl, jq, unzip, zstd, python3, docker.io]\n\
         runcmd:\n  - useradd -m -s /bin/bash runner && usermod -aG docker runner\n  \
         - sudo -u runner bash -c 'mkdir -p ~/actions && cd ~/actions && curl -fsSL -o r.tgz {url} && tar -xzf r.tgz'\n  \
         - /home/runner/actions/bin/installdependencies.sh\n  \
         - sudo -u runner bash -c 'cd ~/actions && echo NEXTEST_TEST_THREADS=2 >> .env && {config}'\n  \
         - cd /home/runner/actions && ./svc.sh install runner && ./svc.sh start\n"
    )
}

/// Hostman's id of Ubuntu `version`, from its live list: an id written here is a guess the day the list changes.
fn ubuntu(call: Call, version: &str) -> Result<i64, String> {
    let listed = ask(call, "GET", "/api/v1/os/servers", Json::Null)?;
    let mut all = listed["servers_os"].as_array().into_iter().flatten();
    all.find(|o| o["name"] == "ubuntu" && o["version"] == version)
        .and_then(|o| o["id"].as_i64())
        .ok_or(format!(
            "hostman: its list holds no Ubuntu {version}, so no server is created"
        ))
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

/// One observation and the repair it allows. `online`: whether a runner with `LABEL` is online on GitHub; `token`: a fresh
/// registration token, asked only when a server is created; `stuck`: the ticks this server has been on with no runner online;
/// `refused`: the creates refused for want of a free node, kept an hour from `now_ms`. One create a tick at most.
pub fn reconcile(
    call: Call,
    online: &dyn Fn() -> Result<bool, String>,
    token: &dyn Fn() -> Result<String, String>,
    want: &Want,
    stuck: &mut u32,
    (refused, now_ms): (&mut Vec<NoNode>, i64),
) -> Step {
    let mut step = || -> Result<Step, String> {
        let listed = ask(call, "GET", "/api/v1/servers", Json::Null)?;
        // an answer with no list is no reading, never « no server »: a create on it would make a second one (/code-review)
        let servers = listed["servers"]
            .as_array()
            .ok_or("hostman: its server list holds no servers, so none is created")?;
        let server = servers
            .iter()
            .find(|s| s["name"].as_str() == Some(&want.name));
        let Some(server) = server else {
            *stuck = 0;
            let os_id = ubuntu(call, &want.ubuntu)?;
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
                "cloud_init": cloud_init(&want.repo, &token()?),
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
        let post = |verb: &str| {
            ask(
                call,
                "POST",
                &format!("/api/v1/servers/{id}/{verb}"),
                Json::Null,
            )
        };
        let status = server["status"].as_str().unwrap_or("");
        match status {
            "off" => {
                *stuck = 0;
                post("start")?;
                Ok(Step::Started(id))
            }
            "no_paid" | "blocked" => {
                Err(format!("server {id} is {status}: his, in Hostman's panel"))
            }
            // creating, installing, turning on, rebooting: Hostman is at work, which is no reason to count or reboot
            s if s != "on" => Ok(Step::Waiting(id)),
            _ => {
                // a GitHub read that fails counts as no runner, and never reboots a server whose runner is unseen
                let seen = online();
                if seen == Ok(true) {
                    *stuck = 0;
                    return Ok(Step::Up);
                }
                *stuck += 1;
                match (*stuck, seen) {
                    (n, _) if n < REBOOT_AT => Ok(Step::Waiting(id)),
                    (n, Ok(_)) if n == REBOOT_AT => {
                        post("reboot")?;
                        Ok(Step::Rebooted(id))
                    }
                    (n, _) if n < DOWN_AT => Ok(Step::Waiting(id)),
                    // no count in it: a reason that changes each tick would sound a new alarm each tick
                    (_, Ok(_)) => Err(format!("server {id} has no runner online after a reboot")),
                    (_, Err(e)) => Err(format!(
                        "server {id}: GitHub's runner list cannot be read: {e}"
                    )),
                }
            }
        }
    };
    step().unwrap_or_else(Step::Down)
}

/// The job model the core's queue runs each tick: its rows are `ci_runner/<name>`'s `status` and, once per reason, `alarm`
/// (empty again when it is up). Never silent: a Down is a status and an alarm, and a Down of another reason a new alarm.
pub struct Runner {
    want: Want,
    call: Tool,
    online: Read<bool>,
    token: Read<String>,
    /// The ticks on with no runner online, and the alarm last written.
    state: RefCell<(u32, String)>,
    /// The creates refused for want of a free node, and the clock that ages them.
    refused: RefCell<Vec<NoNode>>,
    now: Box<dyn Fn() -> i64>,
}

impl Runner {
    pub fn new(want: Want, call: Tool, online: Read<bool>, token: Read<String>) -> Self {
        Self {
            want,
            call,
            online,
            token,
            state: RefCell::new((0, String::new())),
            refused: RefCell::default(),
            now: Box::new(|| 0),
        }
    }

    /// The clock a refusal's hour is counted on: the host's.
    pub fn clocked(mut self, now: Box<dyn Fn() -> i64>) -> Self {
        self.now = now;
        self
    }

    pub fn tick(&self) -> Vec<FieldWrite> {
        let (stuck, alarmed) = &mut *self.state.borrow_mut();
        let refused = (&mut *self.refused.borrow_mut(), (self.now)());
        let step = reconcile(
            &*self.call,
            &*self.online,
            &*self.token,
            &self.want,
            stuck,
            refused,
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
            _ => String::new(),
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
