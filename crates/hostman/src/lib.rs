//! Hostman as a dash tool (task rust-tool-hostman; rfc-rust-unified section 20j, the service-tool skill's steps 1 to 4): its
//! Cloud Servers API and nothing else (@nha, 2026-11-13: « be super clear we cannot use bare metal or AI. We use exclusively
//! VPS/cloud servers »), the key from `.secrets/hostman.env`, the credit left and the month's spend, and a spend cap: a
//! create or resize whose month would pass the credit left is refused with its cost before any write. A write with no undo
//! (a delete, a reinstall) is his, in Hostman's panel. It keeps no log of its own: `Registry::call` hands each call to the
//! core's audit log, its passwords masked.
//! ponytail: no throttle; add one beside `call` once a loop calls it.
use dash_core::registry::{Contribution, Tool};
use serde_json::Value;
use std::cell::Cell;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The host its API answers on (its OpenAPI bundle, hostman.com/api-docs-data/bundle.json, names no other).
pub const API: &str = "https://hostman.com";
/// The key's file under the repository, and its one variable: read at each call, its value never printed.
pub const KEY_FILE: &str = ".secrets/hostman.env";
const KEY_VAR: &str = "HOSTMAN_API_KEY";
/// The arguments that ask the credit left and the month's spend.
pub const USAGE: &str = r#"{"usage":true}"#;
const FINANCES: &str = "/api/v1/account/finances";

/// One call to the API: its method, its path from the host, its JSON body; the answer's body, or why there is none.
pub type Http = Box<dyn Fn(&str, &str, Option<&str>) -> Result<String, String>>;

/// How curl's failure starts: no answer came, so a write may still have gone through.
const NO_ANSWER: &str = "hostman: no answer";
/// curl's exits that sent no request: no proxy or host resolved, no connection, no TLS handshake.
const NOT_SENT: [i32; 5] = [5, 6, 7, 35, 60];
/// How long the lock file's month stands in for a finances read that may not hold the last write yet (/code-review).
const LEDGER_S: u64 = 600;

pub struct Hostman {
    http: Http,
    /// The file a priced write locks from its finances read to its answer, so two `dash mcp` processes never both pass
    /// the cap on one balance (/code-review); None in a test, whose calls run one at a time.
    lock: Option<Box<dyn Fn() -> PathBuf>>,
    /// What the last call added to the month, in its currency: the audit log's cost.
    added: Cell<f64>,
}

/// The tool, registered as `hostman`; `repo` is read at a call, never at the registration, so a test's registry needs none.
pub fn contribution(repo: fn() -> PathBuf) -> Contribution {
    let http: Http = Box::new(move |method, path, body| curl(&repo(), method, path, body));
    let mut tool = Hostman::new(http);
    tool.lock = Some(Box::new(move || repo().join(".claude/run/hostman.lock")));
    Contribution {
        tools: vec![("hostman".into(), Box::new(tool) as Box<dyn Tool>)],
        ..Default::default()
    }
}

fn refused(method: &str, path: &str, why: &str) -> String {
    format!("hostman: {method} {path} refused: {why}")
}

/// Whether a path holds only the characters an API path and its query need. A glob, a fragment, an escape or a space each hid
/// the call curl sends from the scope and the cap (/code-review, twice): a list of what passes, never of what does not.
fn plain(path: &str) -> bool {
    let listed = |b: u8| b.is_ascii_alphanumeric() || b"/_-?=&.".contains(&b);
    path.bytes().all(listed) && !path.contains("..") && !path.contains("//")
}

/// A path's parts under /api/v1/, its query left out; None for any other path, or one that is not plain.
fn parts(path: &str) -> Option<Vec<&str>> {
    let bare = path.split('?').next().filter(|_| plain(path))?;
    Some(bare.strip_prefix("/api/v1/")?.split('/').collect())
}

fn is_id(part: &str) -> bool {
    !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())
}

/// What a call may reach: Cloud Servers alone, its read-only lists, and the balance the cap reads. Bare metal (dedicated
/// servers), GPU, AI, Kubernetes, databases, storage and every other product are refused before any call.
pub fn scope(method: &str, path: &str) -> Result<(), String> {
    let allowed = match (method, parts(path).as_deref()) {
        ("GET", Some(["account", "finances"])) => true,
        ("GET", Some(["presets" | "configurator" | "os" | "software", "servers"])) => true,
        ("GET" | "POST", Some(["servers"])) => true,
        ("GET" | "POST" | "PATCH" | "DELETE", Some(["servers", id, ..])) => is_id(id),
        _ => false,
    };
    match allowed {
        true => Ok(()),
        false => Err(refused(
            method,
            path,
            "Cloud Servers (VPS) only: no bare metal, GPU, AI or other product (@nha, 2026-11-13: « We use exclusively \
             VPS/cloud servers »)",
        )),
    }
}

/// What a write adds to the month, read before it runs.
enum Spend {
    Free,
    /// A preset, and the bandwidth asked, which its own must hold.
    Creates(i64, Option<i64>),
    Resizes {
        server: String,
        preset: i64,
    },
}

/// The keys a create may send: none adds to the bill but the preset, which is priced, an `os_id` its free list holds, and
/// the two Hostman requires, `is_ddos_guard` (false alone) and a bandwidth its preset holds. A configuration, which is how
/// a GPU is asked, software or a network is refused, and an `image_id`, whose OS the free list never sees (/code-review).
const CREATE: [&str; 11] = [
    "name",
    "comment",
    "preset_id",
    "os_id",
    "avatar_id",
    "ssh_keys_ids",
    "cloud_init",
    "availability_zone",
    "project_id",
    "is_ddos_guard",
    "bandwidth",
];
/// The keys a server's PATCH may send: its configurator, software and bandwidth are refused, and a new OS or image, which
/// reinstalls it and wipes its disk (/code-review).
const PATCH: [&str; 5] = ["name", "comment", "preset_id", "avatar_id", "cloud_init"];

/// The POST verbs under a server that cost nothing more, each with the body keys it takes (Hostman's OpenAPI bundle).
const FREE: [(&str, &[&str]); 7] = [
    ("shutdown", &[]),
    ("hard-shutdown", &[]),
    ("start", &[]),
    ("reboot", &[]),
    ("reset-password", &[]),
    ("boot-mode", &["boot_mode"]),
    ("image-unmount", &[]),
];

/// The actions of POST /api/v1/servers/<id>/action that cost nothing more and lose no data: « remove » and « install » are
/// refused with the deletes, « clone » with the clones (/code-review).
const FREE_ACTIONS: [&str; 6] = [
    "hard_reboot",
    "hard_shutdown",
    "reboot",
    "reset_password",
    "shutdown",
    "start",
];

/// The OS families a create may install: Hostman's list holds no paid licence in them (17 OSes, all Linux or BSD, read
/// 2026-11-14); a Windows it may add later would bill one the cap never sees (/code-review).
const FREE_OS: [&str; 2] = ["linux", "BSD"];

/// The answer's keys masked before it leaves the tool, so the audit log, which keeps each answer, holds no server's root or
/// VNC password (/code-review): a key one of whose `_` parts is one of these words, so `is_private` or `private_ip` stay.
const SECRET: [&str; 5] = ["pass", "password", "token", "secret", "key"];

/// The first key of `body` the write may not send, or of the wrong type: an id an integer, `is_ddos_guard` false. A string
/// or a float preset or bandwidth would pass the cap unpriced (/code-review).
fn unpriced(body: &Value, allowed: &[&str]) -> Option<String> {
    let typed = |k: &str, v: &Value| match k {
        "preset_id" | "os_id" | "project_id" | "bandwidth" => v.is_i64(),
        "is_ddos_guard" => *v == false,
        _ => true,
    };
    let Some(keys) = body.as_object() else {
        return (!body.is_null()).then(|| "body, which is not an object".into());
    };
    let mut keys = keys.iter();
    keys.find(|(k, v)| !allowed.contains(&k.as_str()) || !typed(k, v))
        .map(|(k, _)| k.clone())
}

/// A call's spend, the call in `scope`: each write's path matched whole, its body held to that write's keys. A write the
/// tool cannot price (a configured server, a clone, which copies disks, IPs and backups, a disk, an IP) is refused, and so
/// is one with no undo.
/// ponytail: presets only; price the configurator's cpu, ram and disk once a size no preset has is needed.
fn spend(method: &str, path: &str, body: &Value) -> Result<Spend, String> {
    let no_price = |what: &str| {
        Err(refused(
            method,
            path,
            &format!("{what}: the tool prices presets alone"),
        ))
    };
    let preset = body["preset_id"].as_i64();
    let (keys, spend): (&[&str], _) = match (method, parts(path).unwrap_or_default().as_slice()) {
        ("GET", _) => return Ok(Spend::Free),
        ("DELETE", _) => return Err(undone(method, path)),
        ("POST", ["servers"]) => match preset {
            Some(p) => (&CREATE, Spend::Creates(p, body["bandwidth"].as_i64())),
            None => return no_price("no preset_id"),
        },
        ("PATCH", ["servers", id]) => (
            &PATCH,
            preset.map_or(Spend::Free, |preset| Spend::Resizes {
                server: id.to_string(),
                preset,
            }),
        ),
        ("POST", ["servers", _, "action"]) => match body["action"].as_str() {
            Some("remove" | "install") => return Err(undone(method, path)),
            Some("clone") => return no_price("a clone, which copies its disks, IPs and backups"),
            Some(a) if FREE_ACTIONS.contains(&a) => (&["action"], Spend::Free),
            _ => return no_price("an action it does not know"),
        },
        ("POST", ["servers", _, verb]) => match FREE.iter().find(|(v, _)| v == verb) {
            Some((_, keys)) => (*keys, Spend::Free),
            None => return no_price("it adds to the bill"),
        },
        ("PATCH", ["servers", _, "local-networks", "nat-mode"]) => (&["nat_mode"], Spend::Free),
        _ => return no_price("it adds to the bill"),
    };
    match unpriced(body, keys) {
        Some(k) => no_price(&format!("its {k}")),
        None => Ok(spend),
    }
}

/// A write that loses a server's data, refused whatever it costs.
fn undone(method: &str, path: &str) -> String {
    refused(
        method,
        path,
        "it has no undo: his, in Hostman's panel (CLAUDE.md: an irreversible, controversial choice blocks on him)",
    )
}

/// The answer with each secret's value masked; an answer that is not JSON, as it came.
fn masked(answer: String) -> String {
    fn mask(v: &mut Value) {
        match v {
            Value::Object(o) => {
                o.iter_mut()
                    .for_each(|(k, v)| match k.split('_').any(|p| SECRET.contains(&p)) {
                        true if !v.is_null() => *v = Value::from("(masked)"),
                        _ => mask(v),
                    })
            }
            Value::Array(a) => a.iter_mut().for_each(mask),
            _ => {}
        }
    }
    match serde_json::from_str::<Value>(&answer) {
        Ok(mut v) => {
            mask(&mut v);
            v.to_string()
        }
        Err(_) => answer,
    }
}

fn parsed(body: &str) -> Result<Value, String> {
    serde_json::from_str(body).map_err(|_| "hostman: its answer is not JSON".to_string())
}

/// The credit left and the month's spend, from the finances' answer.
pub fn usage_text(finances: &str) -> Result<String, String> {
    let f = &parsed(finances)?["finances"];
    let n = |k: &str| {
        f[k].as_f64()
            .ok_or(format!("hostman: its finances hold no {k}"))
    };
    let cur = f["currency"].as_str().unwrap_or("USD");
    let (left, month, hour, paid) = (
        n("balance")?,
        n("monthly_cost")?,
        n("hourly_cost")?,
        n("total_paid")?,
    );
    Ok(format!(
        "hostman: {left:.2} {cur} of credit left; the month's spend {month:.2} {cur} ({hour:.2} an hour, the servers running \
         now); {:.2} {cur} spent of {paid:.2} paid",
        paid - left
    ))
}

/// A Cloud Servers preset from the presets' answer; one with a GPU is refused.
fn preset(presets: &Value, id: i64) -> Result<&Value, String> {
    let found = presets["server_presets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["id"].as_i64() == Some(id))
        .ok_or(format!("hostman: no Cloud Servers preset {id}"))?;
    match found["gpu"].as_f64().is_some_and(|g| g > 0.0) {
        true => Err(format!(
            "hostman: preset {id} holds a GPU: Cloud Servers without GPU only"
        )),
        false => Ok(found),
    }
}

/// A preset's price a month.
fn price(presets: &Value, id: i64) -> Result<f64, String> {
    preset(presets, id)?["price"]
        .as_f64()
        .ok_or(format!("hostman: preset {id} names no price"))
}

impl Hostman {
    pub fn new(http: Http) -> Self {
        Self {
            http,
            lock: None,
            added: Cell::new(0.0),
        }
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        parsed(&(self.http)("GET", path, None)?)
    }

    /// A server's preset, from its answer: one made by the configurator has none here, so it is refused.
    fn preset_of(&self, server: &str) -> Result<i64, String> {
        let got = self.get(&format!("/api/v1/servers/{server}"))?;
        got["server"]["preset_id"].as_i64().ok_or(format!(
            "hostman: server {server} has no preset, so its price is unknown"
        ))
    }

    /// An `os_id` a create may send: one in Hostman's list, of a free family.
    fn free_os(&self, id: i64) -> Result<(), String> {
        let list = self.get("/api/v1/os/servers")?;
        let os = list["servers_os"].as_array().into_iter().flatten();
        let family = os
            .into_iter()
            .find(|o| o["id"].as_i64() == Some(id))
            .map(|o| &o["family"]);
        match family.and_then(Value::as_str) {
            Some(f) if FREE_OS.contains(&f) => Ok(()),
            _ => Err(format!(
                "hostman: os {id} refused: Linux or BSD from its list alone, since another may bill a licence the tool \
                 does not price"
            )),
        }
    }

    /// What the write adds to the month, or its refusal with that cost when the month would pass the credit left. The
    /// presets are read once, whatever the write prices (/code-review).
    /// `floor`: the month the last write under the lock left, which finances may not show yet.
    fn capped(&self, spend: Spend, floor: f64) -> Result<(f64, f64), String> {
        let presets = || self.get("/api/v1/presets/servers");
        let (what, added) = match spend {
            Spend::Free => return Ok((0.0, 0.0)),
            Spend::Creates(p, bandwidth) => {
                let presets = presets()?;
                let own = preset(&presets, p)?["bandwidth"].as_i64();
                if bandwidth.is_some_and(|b| own.is_none_or(|own| b > own)) {
                    return Err(format!(
                        "hostman: a server on preset {p} with more bandwidth than its own refused: the tool prices presets \
                         alone"
                    ));
                }
                (format!("a server on preset {p}"), price(&presets, p)?)
            }
            Spend::Resizes { server, preset } => {
                let (was, presets) = (self.preset_of(&server)?, presets()?);
                let added = price(&presets, preset)? - price(&presets, was)?;
                (format!("server {server} resized to preset {preset}"), added)
            }
        };
        let f = &self.get(FINANCES)?["finances"];
        let (left, running) = (f["balance"].as_f64(), f["monthly_cost"].as_f64());
        let (Some(left), Some(running)) = (left, running) else {
            return Err("hostman: its finances hold no balance, so no write is priced".into());
        };
        let running = running.max(floor);
        let cur = f["currency"].as_str().unwrap_or("USD");
        let month = running + added;
        if month > left {
            return Err(format!(
                "hostman: refused: {what} costs {added:.2} {cur} a month, {month:.2} {cur} a month with the servers \
                 running, past the spend cap of {left:.2} {cur} of credit left"
            ));
        }
        Ok((added, month))
    }

    /// One call: `{"usage": true}`, else `{"method", "path", "body"}` in scope and under the cap, passed through.
    fn asked(&self, arguments: &str) -> Result<String, String> {
        let asked: Value = serde_json::from_str(arguments)
            .map_err(|_| "hostman: its arguments are not JSON".to_string())?;
        if asked["usage"] == true {
            return usage_text(&(self.http)("GET", FINANCES, None)?);
        }
        let method = asked["method"]
            .as_str()
            .unwrap_or("GET")
            .to_ascii_uppercase();
        let path = asked["path"].as_str().ok_or("hostman: no path")?;
        scope(&method, path)?;
        let body = &asked["body"];
        let spend = spend(&method, path, body)?;
        // held to the answer: a second process reads the finances once this write is in them (/code-review)
        let mut held = match (&spend, &self.lock) {
            (Spend::Free, _) | (_, None) => None,
            (_, Some(at)) => Some(locked(&at())?),
        };
        if let (Spend::Creates(..), Some(os)) = (&spend, body["os_id"].as_i64()) {
            self.free_os(os)?;
        }
        let floor = held.as_mut().map_or(0.0, ledger);
        let (added, month) = self.capped(spend, floor)?;
        let sent = (!body.is_null()).then(|| body.to_string());
        let answer = (self.http)(&method, path, sent.as_deref());
        // a write with no answer may have gone through: its cost is kept, and the caller told to look (/code-review)
        let unknown = method != "GET" && answer.as_ref().is_err_and(|e| e.starts_with(NO_ANSWER));
        if answer.is_ok() || unknown {
            // a downgrade adds nothing to the bill: the audit log's cost is never negative (/code-review)
            self.added.set(added.max(0.0));
            if let Some(file) = held.as_mut() {
                let _ = written(file, month);
            }
        }
        answer.map(masked).map_err(|e| match unknown {
            true => format!(
                "{e}: it may have gone through, so read GET /api/v1/servers before sending it again"
            ),
            false => e,
        })
    }
}

impl Tool for Hostman {
    fn description(&self) -> &str {
        "Hostman's Cloud Servers (VPS) API and nothing else. {\"usage\": true}: the credit left and the month's spend. Else \
         {\"method\", \"path\", \"body\"} under /api/v1/servers, its presets, configurator, os and software lists, or \
         /api/v1/account/finances. Bare metal, GPU, AI and every other product are refused; a create or resize past the \
         credit left is refused with its cost; a delete, reinstall or clone is refused. Passwords in an answer are masked: \
         give a create ssh_keys_ids. Never put a secret in a body: the audit log keeps the arguments."
    }
    fn input_schema(&self) -> &str {
        r#"{"type":"object","properties":{"usage":{"type":"boolean"},"method":{"type":"string","enum":["GET","POST","PATCH","DELETE"]},"path":{"type":"string"},"body":{"type":"object"}}}"#
    }
    fn call(&self, arguments: &str) -> Result<String, String> {
        self.added.set(0.0);
        self.asked(arguments)
    }
    /// What a write that went through, or may have, committed a month, in its currency: a rate, never what was billed.
    fn cost(&self, _arguments: &str, _answer: &Result<String, String>) -> f64 {
        self.added.get()
    }
    fn usage(&self) -> Option<&str> {
        Some(USAGE)
    }
}

/// The key's value from `<repo>/.secrets/hostman.env`.
fn key(repo: &Path) -> Result<String, String> {
    let file = repo.join(KEY_FILE);
    let text =
        std::fs::read_to_string(&file).map_err(|e| format!("hostman: {}: {e}", file.display()))?;
    text.lines()
        .find_map(|l| {
            let l = l.trim();
            let l = l.strip_prefix("export ").unwrap_or(l);
            l.strip_prefix(KEY_VAR)?.trim_start().strip_prefix('=')
        })
        .map(|v| {
            let v = v.trim();
            let quoted = ['"', '\'']
                .iter()
                .find_map(|q| v.strip_prefix(*q)?.strip_suffix(*q));
            // an unquoted value ends at its comment, as a shell reads it
            quoted
                .unwrap_or_else(|| v.split(" #").next().unwrap_or(v).trim())
                .to_string()
        })
        .filter(|v| !v.is_empty())
        .ok_or(format!("hostman: {} holds no {KEY_VAR}", file.display()))
}

/// The month the lock file holds, while under `LEDGER_S` old; 0 when none.
fn ledger(file: &mut File) -> f64 {
    let mut text = String::new();
    let _ = file.read_to_string(&mut text);
    let mut parts = text.split_whitespace().map(str::parse::<f64>);
    match (parts.next(), parts.next()) {
        (Some(Ok(at)), Some(Ok(month)))
            if dash_process::now_s().saturating_sub(at as u64) < LEDGER_S =>
        {
            month
        }
        _ => 0.0,
    }
}

/// The month a write left, with now, into the lock file.
fn written(file: &mut File, month: f64) -> std::io::Result<()> {
    file.set_len(0)?;
    file.rewind()?;
    write!(file, "{} {month}", dash_process::now_s())
}

/// `at` locked, blocking, until the result is dropped; its folder made first.
fn locked(at: &Path) -> Result<File, String> {
    let held = |e: std::io::Error| format!("hostman: the cap's lock {}: {e}", at.display());
    if let Some(dir) = at.parent() {
        std::fs::create_dir_all(dir).map_err(held)?;
    }
    let file = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(at)
        .map_err(held)?;
    file.lock().map_err(held)?;
    Ok(file)
}

/// A call over curl, the key in its environment, never its command line; the body on its input.
fn curl(repo: &Path, method: &str, path: &str, body: Option<&str>) -> Result<String, String> {
    let key = key(repo)?;
    let url = format!("{API}{path}");
    let mut args = vec![
        "-sS",
        "-g",
        "--max-time",
        "20",
        "--proto",
        "=https",
        "-X",
        method,
        "--variable",
        "%KEY",
        "--expand-header",
        "authorization: Bearer {{KEY}}",
        "-w",
        "\n%{http_code}",
    ];
    if body.is_some() {
        args.extend([
            "-H",
            "content-type: application/json",
            "--data-binary",
            "@-",
        ]);
    }
    args.push(&url);
    let limit = Duration::from_secs(25);
    let (status, text, why) =
        dash_process::answer_fed("curl", &args, None, &[("KEY", &key)], body, limit)
            // one that ran past its limit or its output may have sent the write (/code-review)
            .map_err(|no| match no.started {
                true => format!("{NO_ANSWER}: {}", no.why),
                false => no.why,
            })?;
    answered(status.code(), &text, &why)
}

/// curl's exit, its output and its stderr's end, read: a write that may have gone through says NO_ANSWER, and one sure
/// not to have, or refused by a 4xx, does not.
fn answered(exit: Option<i32>, text: &str, why: &str) -> Result<String, String> {
    let (answer, code) = text.rsplit_once('\n').unwrap_or(("", text.trim()));
    match code.trim() {
        _ if NOT_SENT.contains(&exit.unwrap_or(-1)) => {
            Err(format!("hostman: not sent: {}", why.trim()))
        }
        _ if exit != Some(0) => Err(format!("{NO_ANSWER}: {}", why.trim())),
        code if code.starts_with('2') => Ok(answer.to_string()),
        // a gateway's 5xx may follow a write the server made (/code-review)
        code if code.starts_with('5') => Err(format!(
            "{NO_ANSWER}: it answered {code}: {}",
            answer.chars().take(300).collect::<String>()
        )),
        code => Err(format!(
            "hostman answered {code}: {}",
            answer.chars().take(300).collect::<String>()
        )),
    }
}

#[cfg(test)]
mod tests;
