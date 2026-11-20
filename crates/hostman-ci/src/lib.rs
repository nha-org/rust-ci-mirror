//! A CI runner on a Hostman Cloud Server that stays up (task rust-hostman-ci-runner; service-tool step 5; @nha, 2026-11-14:
//! « We should use their cloud server to run CI. Cloud server only the tool should prevent anything else. And then kill the
//! whatsnr VPS. », « If CI is the bottleneck... use hostman as I asked. »).
//!
//! The desired state is a pool of `Want`s (`live::pool`): each a Cloud Server of that name, on, with each of its GitHub
//! runners online under its label (task ci-runner-pool; @nha, 2026-11-18: « I suggest spinning the second server and seeing
//! how it holds. »), each reconciled on its own, its rows under its name. `reconcile`
//! observes it through the `hostman` tool's JSON (a closure, so this crate reaches no other plugin), repairs what it can
//! (create, start, a public IPv4, a reinstall an hour) and otherwise says `Down` with its reason. `Runner` is the core's job model: a tick of the
//! queue (`dash_jobs`, the `Recurring` job `live` registers, task rust-hostman-ci-wire) runs it and its rows come back through the write path, so a status is data and an alarm is written
//! once per reason, never again while it stands and never silently. `route` is the router's tier order with this runner
//! before the Mac and the whatsnr VPS gone from it.
//! ponytail: each box is created and healed on its own; none is ever deleted here (a delete has no undo: his, in Hostman's
//! panel), so a box dropped from the pool stays billed until he removes it.
//! ponytail: the boxes reconciled by hand; how we manage infra next is weighed in private/docs/rfc-iac.html (@nha, 2026-11-15:
//! « I'd like an RFC there. super light. just compare and contrast. »).
use dash_core::{FieldWrite, Value};
use serde_json::{Value as Json, json};
use std::cell::RefCell;

pub mod evidence;
pub mod live;
pub mod route;

/// The label the workflows ask for: alone, with no `self-hosted`, so a job takes the hosted path and no private/ reaches it.
/// Nothing in it names this repository: the box runs klar's CI too (@nha, 2026-11-15: « site-perso-ci => just ci. We will
/// run klar CI too. »). The old label, site-perso-hostman, stays on the whatsnr VPS runners until this one is online.
pub const LABEL: &str = "ci";
/// The label of the checks that build nothing (explainer-gate, approval-number, the site's page check: seconds each), alone,
/// on its own runner on the 8 GB box, so none waits behind a 30-minute build on `LABEL` (up to two hours on 2026-11-18).
pub const LIGHT: &str = "ci-light";
pub const MODEL: &str = "hostman-ci-runner";

/// One runner on a box: what it registers to (`owner/name`, a repository, or an organisation alone) and its one label.
#[derive(Clone, Debug, PartialEq)]
pub struct Reg {
    pub to: String,
    pub label: &'static str,
}

impl From<&str> for Reg {
    /// A runner under `LABEL`.
    fn from(to: &str) -> Self {
        Reg {
            to: to.into(),
            label: LABEL,
        }
    }
}

impl std::fmt::Display for Reg {
    /// Its registration alone under `LABEL`, as the statuses have always said it; its label beside it otherwise.
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.label {
            LABEL => write!(f, "{}", self.to),
            label => write!(f, "{} ({label})", self.to),
        }
    }
}

impl Reg {
    /// Its name on GitHub: the box's under `LABEL`, its label otherwise.
    /// ponytail: a label other than `LABEL` names one runner a registration; give `Reg` a name of its own if two boxes carry one.
    pub fn runner(&self, host: &str) -> String {
        match self.label {
            LABEL => host.into(),
            label => label.into(),
        }
    }

    /// Its labels on GitHub: under `LABEL` it takes `LIGHT` jobs too, so a light job runs on whichever box has a runner free
    /// (/code-review: a light runner on one box alone left the merge gate on the box that froze); `LIGHT`'s own runner takes
    /// light jobs alone, never a build.
    pub fn labels(&self) -> String {
        match self.label {
            LABEL => format!("{LABEL},{LIGHT}"),
            label => label.into(),
        }
    }
}
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
    /// The bandwidth a preset must hold at least, in Mbps.
    pub mbps: i64,
    /// The OS installed, its name in Hostman's list: its newest version there.
    pub os: &'static str,
    pub ssh_keys_ids: Vec<i64>,
    /// cargo's jobs at once for each runner, `CARGO_BUILD_JOBS` in its environment: one rustc short of a cold build that fills
    /// the box's RAM (the box froze twice mid-build on 2026-11-18).
    /// ponytail: per runner, so site_perso's and klar-im's builds at once on the 8 GB box are twice it; the swap takes that,
    /// and the run's readings say whether it holds.
    pub jobs: u8,
    /// Whether the box answers whatsnr.eu's redirect (Caddy, ports 80 and 443): one box alone, the one its DNS names.
    pub redirect: bool,
    /// The box's runners, each its own process, folder and service (task hostman-ci-klar-runner; @nha, 2026-11-15: « Hostman
    /// is done when: it runs our CI on debian, it runs Klar's CI on the same »).
    pub registrations: Vec<Reg>,
    /// Until when its create may pass the credit left, by his leave; none for a box the cap holds always.
    pub leave_until_ms: Option<i64>,
}

/// His leave (@nha, 2026-11-18: « feel free to lift gates if this is needed or spin up temporary hostman servers. »): ci-2
/// is created past the credit left, the 8 GB box down and its credit spent, until the credit's expiry, 2026-12-23 00:00
/// UTC. The hostman tool holds the same leave for the same name (its `LEAVE_NAME`, `LEAVE_UNTIL_S`).
pub const LEAVE_UNTIL_MS: i64 = 1_797_984_000_000;

impl Want {
    /// The one server wanted (@nha, 2026-11-14: « take one 8GB machine please. »): a preset with 8 GB of RAM, the cheapest in
    /// Hostman's live list whose location has a free node. Read 2026-11-15, five hold 8 GB: 5133 pl-1 at 15 USD a month,
    /// then 5145 us-2, 5157 nl-1, 5515 us-3 and 6273 de-1 at 20; pl-1 answered 409 no_free_node at the first create. `OS`, no
    /// ssh key, the runner needs none. It was site-perso-ci, created on Ubuntu 24.04 (server 9602127, 2026-11-15).
    /// Hostman's Terraform is not used: this tool calls the API, so its Cloud-Servers-only scope and the credit cap hold
    /// before each call, which a Terraform plan would go round.
    pub fn eight_gb(registrations: &[&str]) -> Self {
        Want {
            name: "ci".into(),
            was: vec!["site-perso-ci".into()],
            ram_mb: 8192,
            mbps: 0,
            os: OS,
            ssh_keys_ids: vec![],
            jobs: 3,
            redirect: true,
            registrations: registrations.iter().map(|&r| r.into()).collect(),
            leave_until_ms: None,
        }
    }

    /// The pool's second box (@nha, 2026-11-18: « I'd say this one "2 x 3.3 GHz, 4 GB, 80 GB, 200 Mbps, €8.00/mo" », « in
    /// fact two of these is still less than [the 8 GB box] which we use »): 4 GB and 200 Mbps, the cheapest such preset in
    /// the live list. Read 2026-11-18 through the hostman tool, all 2 vCPU and 80 GB: 5147 us-2, 5159 nl-1, 5513 us-3 and
    /// 6271 de-1 at 8 USD a month; 5135 pl-1 at 6 has 100 Mbps and is left out. Its cargo on two jobs.
    pub fn four_gb(registrations: &[&str]) -> Self {
        Want {
            name: "ci-2".into(),
            was: vec![],
            ram_mb: 4096,
            mbps: 200,
            jobs: 2,
            redirect: false,
            leave_until_ms: Some(LEAVE_UNTIL_MS),
            ..Want::eight_gb(registrations)
        }
    }
}

/// What one reconcile did.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Up,
    Created(i64),
    /// Created past the credit left, by his leave (`Want::leave_until_ms`).
    CreatedByLeave(i64),
    Started(i64),
    /// On, a runner not online yet: it is booting or registering; the registrations GitHub lists none for, when it answered.
    Waiting(i64, String),
    /// Hostman at work on it: its status, such as installing.
    Busy(i64, String),
    /// A public IPv4 added: it had none, so it reached nothing.
    Addressed(i64),
    /// Reinstalled with a fresh registration token: why, and Hostman's last readings of the box before its disk went.
    Reinstalled(i64, String),
    /// Reinstalled as `Reinstalled`, for a runner that was online since the last reinstall and vanished, its box on: the box
    /// died under it (16:22 on 2026-11-18), which is an alarm.
    Healed(i64, String),
    /// Its runners online, its cloud-init older than the one wanted: reinstalled once none is busy and an hour has passed
    /// since the last.
    Held(i64),
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
            Step::CreatedByLeave(id) => {
                format!("created server {id}, past the credit cap by his leave until 2026-12-23")
            }
            Step::Started(id) => format!("started server {id}"),
            Step::Waiting(id, missing) => match missing.is_empty() {
                true => format!("waiting: server {id} is on, {NO_RUNNER} yet"),
                false => format!("waiting: server {id} is on, {NO_RUNNER} yet for {missing}"),
            },
            Step::Busy(id, status) => format!("waiting: server {id} is {status}"),
            Step::Addressed(id) => {
                format!(
                    "addressed server {id}: it had no public IP, so its cloud-init reached nothing"
                )
            }
            Step::Reinstalled(id, why) | Step::Healed(id, why) => {
                format!("reinstalled server {id} with a fresh registration token: {why}")
            }
            // « up » first: its runners take jobs meanwhile, so the router keeps it (`live::up`)
            Step::Held(id) => format!(
                "up: server {id} runs an older cloud-init; it is reinstalled once its runners are idle, an hour after the last"
            ),
            Step::NoNode(said) => format!("no free node: {said}"),
            Step::Down(why) => format!("down: {why}"),
        }
    }
}

/// One call of the `hostman` tool: its arguments as JSON text, its answer.
pub type Call<'a> = &'a dyn Fn(&str) -> Result<String, String>;
/// The owned forms the job model keeps.
pub type Tool = Box<dyn Fn(&str) -> Result<String, String>>;
/// A question about one runner (`Want::registrations`): whether it is online, a fresh token for it.
pub type Ask<T> = Box<dyn Fn(&Reg) -> Result<T, String>>;
/// The ssh public keys root takes, read at each tick (`live::keys`, from his ~/.ssh): never written in rust/.
pub type Keys = Box<dyn Fn() -> Result<Vec<String>, String>>;
/// What a reinstall keeps of server `id`, on at its public IPv4, before it wipes it: « evidence in » the file it wrote, or
/// why none (`live::evidence`, task ci-heal-evidence).
pub type Capture = Box<dyn Fn(i64, &str) -> String>;
/// A `Capture` lent to a tick: none when the runner was given none.
pub type Captured<'a> = Option<&'a dyn Fn(i64, &str) -> String>;
/// Whether a reinstall kept server `id`'s Caddy store (read off the box at its public IPv4 when on): kept in the data folder,
/// never sent (`live::store`, task ci-caddy-keeps-certs).
pub type Store = Box<dyn Fn(i64, Option<&str>) -> bool>;
/// A `Store` lent to a tick: none when the runner was given none.
pub type Stored<'a> = Option<&'a dyn Fn(i64, Option<&str>) -> bool>;
/// The kept store pushed back to server `id` at its IPv4 over ssh, on the install the third word names, Caddy started
/// after (`live::push`): refused while the box shows another install, its cloud-init runs or ssh does not answer.
pub type Push = Box<dyn Fn(i64, &str, &str) -> Result<(), String>>;
/// A `Push` lent to a tick.
pub type Pushed<'a> = Option<&'a dyn Fn(i64, &str, &str) -> Result<(), String>>;

/// Where a held install's cloud-init writes its install id (no secret): the push lands on that install alone.
pub const INSTALL_ID: &str = "/var/lib/ci-install-id";

/// A fresh install id: 64 random bits from the standard library's hasher seeds, in hex.
fn install_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let a = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    let b = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    format!("{a:016x}{b:016x}")
}

/// How long a held Caddy waits for its store before the reconciler says so (the box's own timer starts it then).
pub const HOLD_MS: i64 = 3_600_000;
/// runcmd's Caddy when the reconciler kept a store for the box (task ci-caddy-keeps-certs): stopped and disabled, so it
/// never asks Let's Encrypt before the store is pushed back over ssh, which starts it; an hour with no push, the box's own
/// timer starts it anyway (it then asks once). The store itself never travels in cloud-init: Hostman keeps the user-data,
/// and any process on the box, a CI job's PR code included, reads it from the metadata service.
pub const HOLD: &str = "  - systemctl disable --now caddy\n  - \"systemd-run --on-active=3600 --unit=caddy-unheld sh -c \
                        'systemctl is-enabled caddy || systemctl enable --now caddy'\"\n  - systemctl restart docker\n";

/// `init` with Caddy held for its store (`HOLD`) and its install `id` written to `INSTALL_ID` at the first boot, in the
/// body a reinstall sends, never in the fingerprint's cloud-init, so a hold moves no hash. A cloud-init with no Caddy holds
/// nothing.
pub fn with_hold(init: &str, id: &str) -> String {
    let entry = format!("write_files:\n  - path: {INSTALL_ID}\n    content: \"{id}\"\n");
    init.replacen("  - systemctl restart caddy docker\n", HOLD, 1)
        .replacen("write_files:\n", &entry, 1)
}

/// A registration's runner as GitHub lists it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seen {
    Gone,
    Idle,
    /// Online and running a job: no reinstall wipes it.
    Busy,
}

impl From<bool> for Seen {
    fn from(online: bool) -> Self {
        if online { Seen::Idle } else { Seen::Gone }
    }
}

fn ask(call: Call, method: &str, path: &str, body: Json) -> Result<Json, String> {
    let asked = json!({"method": method, "path": path, "body": body}).to_string();
    let answer = call(&asked)?;
    // a write's 204 answers nothing (start, each action): done, never « no JSON », which once hid a write that went
    // through and left it uncounted (server 9602127, 2026-11-15 12:02); an empty read is still no reading
    if method != "GET" && answer.trim().is_empty() {
        return Ok(Json::Null);
    }
    serde_json::from_str(&answer).map_err(|_| format!("hostman: {path} answered no JSON"))
}

/// The statuses Hostman's OpenAPI lists for a server it is at work on (read 2026-11-15): waited on, never counted. Any
/// other but on, off, error and the refused ones is an alarm: an unlisted status waited on in silence is how « error »
/// stood from 12:02 on 2026-11-15 with no alarm.
const AT_WORK: [&str; 12] = [
    "installing",
    "software_install",
    "reinstalling",
    "turning_on",
    "turning_off",
    "hard_turning_off",
    "rebooting",
    "hard_rebooting",
    "removing",
    "cloning",
    "transfer",
    "configuring",
];

/// The words every status of a server on with no runner holds, by which `live` finds when that began after a restart.
pub const NO_RUNNER: &str = "no runner online";
/// The words a heal's status holds, by which `live` finds the last heal after a restart.
pub const VANISHED: &str = "vanished after it was online";

/// The box's resolver, written in place of the image's (a link to a stub is replaced, never written through), and kept
/// by dhclient, which writes its lease's nameservers again at each renewal, when the image runs one.
pub const RESOLVER: &str = "rm -f /etc/resolv.conf && printf 'nameserver 1.1.1.1\\nnameserver 8.8.8.8\\noptions timeout:1 \
                            attempts:2 single-request-reopen\\n' > /etc/resolv.conf";
const DHCLIENT: &str = "grep -qs supersede /etc/dhcp/dhclient.conf || echo 'supersede domain-name-servers 1.1.1.1, \
                        8.8.8.8;' >> /etc/dhcp/dhclient.conf || true";

/// apt's daily timers: the lists refreshed and unattended-upgrades run each day; its Debian default takes the security
/// archive (`label=Debian-Security`).
pub const UPGRADES: &str =
    "APT::Periodic::Update-Package-Lists \"1\";\nAPT::Periodic::Unattended-Upgrade \"1\";\n";
/// Every apt on the box waits for the lock up to 10 minutes, never fails on it: apt's daily timers may hold it at first
/// boot, while cloud-init and installdependencies.sh install, or later while a job runs (/code-review). Written first, not
/// deferred, so the packages' own install waits too.
pub const LOCK_WAIT: &str = "DPkg::Lock::Timeout \"600\";\n";
/// dockerd restarted by a security update keeps its containers: a job's survive it (/code-review).
pub const DOCKER: &str = "{\"live-restore\": true}\n";
/// whatsnr.eu's one job since the Scaleway VPS goes: the same path on whatsnr.com. Its http form is named, so a plain http
/// call gets the 301 to whatsnr.com at once, never first one to its own https.
pub const CADDYFILE: &str =
    "http://whatsnr.eu, whatsnr.eu {\n\tredir https://whatsnr.com{uri} permanent\n}\n";

/// The box's kernel, then its root, each in a boot of its own, before any work (task ci-box-image-resize). ci-2's kernel,
/// Debian 12's 6.1.0-55 (6.1.192; Hostman lists Debian 11 and 12 alone, 2026-11-19), oopsed « BUG: kernel NULL pointer
/// dereference » at « ext4_mb_load_buddy_gfp+0xbc » in writeback 3.5 minutes after cloud-init grew its mounted root from
/// 1277947 to 20938747 blocks (04:49:09 to 04:52:52): the worker died, and every write after it stuck. So cloud-init's own
/// growth is off, and two bootcmds, read each boot, each run once behind its marker, every step chained with && so a
/// failure leaves no marker and no reboot and the next boot tries again; each ends on a reboot, the sleep holding
/// cloud-init until systemd stops it. Boot one: the release's backports kernel.
pub const KERNEL: &str = "[ -e /var/lib/ci-kernel ] || { c=$(. /etc/os-release && echo $VERSION_CODENAME) && echo \"deb \
                          http://deb.debian.org/debian $c-backports main\" > /etc/apt/sources.list.d/backports.list && \
                          apt-get update -q && apt-get install -y -q -t $c-backports linux-image-cloud-amd64 && touch \
                          /var/lib/ci-kernel && sync && systemctl reboot && sleep 600; }";
/// Boot two: the partition and filesystem grown on the backports kernel alone, never Debian 12's 6.1 (its version moves
/// with backports, so the test is « not 6.1 »), then a reboot, so no write lands on a root grown while mounted, on any
/// kernel. growpart's NOCHANGE (a partition grown before a failed resize2fs) is no failure.
pub const GROWN: &str = "[ -e /var/lib/ci-grown ] || { (uname -r | grep -qv '^6\\.1\\.' || (echo \"ci-grown: running $(uname \
                         -r), not the backports kernel: the root is not grown\" >&2 && false)) && d=$(findmnt -no SOURCE /) \
                         && p=/dev/$(lsblk -no PKNAME $d) && n=${d##*[!0-9]} && (growpart $p $n || growpart $p $n | grep -q \
                         NOCHANGE) && resize2fs $d && touch /var/lib/ci-grown && sync && systemctl reboot && sleep 600; }";
/// runcmd's first step: no runner on a root never grown, said in the kernel's log and cloud-init's, so the box stays without
/// a runner and the reconciler heals it, capturing it first.
pub const GROWN_FIRST: &str = "[ -e /var/lib/ci-grown ] || { echo \"ci-grown: no /var/lib/ci-grown, the root was not grown \
                               on the backports kernel: no runner installed\" | tee /dev/kmsg >&2; exit 1; }";

/// 4 GB of swap, made once and on at each boot after: a cold build that outgrows the RAM slows down, where with none the box
/// froze and its runner vanished with no log (16:22 and 17:13 on 2026-11-18, both mid-build).
pub const SWAP: &str = "fallocate -l 4G /swapfile && chmod 600 /swapfile && mkswap /swapfile && swapon /swapfile && echo \
                        '/swapfile none swap sw 0 0' >> /etc/fstab";

/// The cloud-init a create or a reinstall sends: one runner per registration, each with its token (valid an hour), in its
/// own folder and service (svc.sh names it after the registration and the runner, so two never clash), registered under
/// its one label as `Reg::runner` names it, its tests on two threads (@nha, 2026-11-14: « cargo nextest -j 2 »), cargo's jobs at `jobs`. Each runner is its own runcmd lines, so one
/// that fails to register leaves the other running. The audit log keeps a body, so each token is one that is dead by the
/// time anyone reads it. The release is GitHub's latest, read on the box: since 2026-09-29 GitHub refuses to register a
/// runner older than v2.329.0, and the 2.321.0 written here was one, so a pin goes stale the same way. ICU is the
/// release's own (libicu76 on Debian 13, libicu72 on 12): the runner's installdependencies.sh asks libicu72 and older
/// alone, its .NET does not start without one, and a name the release lacks in `packages` would fail the whole list.
/// `sudo`, which a Debian image may lack, runs config.sh as the runner, which refuses root. Paths are absolute: no `~`
/// that rests on sudo's HOME. `bootcmd` writes the resolver before apt: on 9602127 (Debian 12, de-1) every new connection
/// waited 5.3 to 5.6 s, glibc's 5 s resolver timeout and the call itself (gate run 43710566004: the action's download info,
/// the curl, each lazy git fetch, one quantum each; 44 s for eight fetches), so one lookup in each lost its first answer.
/// Two public resolvers, a 1 s timeout and A and AAAA asked from separate sockets cover a dead first nameserver and a NAT
/// that drops one of two parallel queries; bootcmd runs each boot, and dhclient is told to keep them at a renewal.
///
/// The box is seen into and kept current (task ci-box-open-and-current; @nha, 2026-11-15: « can I see into the server? How
/// do we install/update stuff there? »): root takes `keys` (`disable_root: false`, so cloud-init adds no « login as »
/// refusal before them); unattended-upgrades applies Debian's security updates each day, by apt's daily timers; Caddy
/// answers whatsnr.eu, http and https, with a 301 to the same path on whatsnr.com, in place of the Scaleway VPS. The two
/// files apt's packages would also own are written `defer`red, after the packages, so they are ours; dockerd keeps its
/// containers when an update restarts it, and apt waits for its lock rather than failing.
/// Caddy and its Caddyfile on `want.redirect`'s box alone.
/// Caddy's certificates outlive a reinstall: kept off the box before the wipe, Caddy held (`with_hold`) and the store pushed
/// back over ssh (task ci-caddy-keeps-certs), since asking Let's Encrypt at each one spent whatsnr.eu's 5 a week
/// (2026-11-19).
pub fn cloud_init(want: &Want, tokens: &[(&Reg, String)], keys: &[String]) -> String {
    let (name, jobs) = (&want.name, want.jobs);
    let (caddy, caddyfile, restart) = match want.redirect {
        true => (", caddy", CADDYFILE, "caddy docker"),
        false => ("", "", "docker"),
    };
    let get = "v=$(curl -fsSL https://api.github.com/repos/actions/runner/releases/latest | jq -r .tag_name) && curl -fsSL \
               -o /home/runner/r.tgz https://github.com/actions/runner/releases/download/$v/actions-runner-linux-x64-${v#v}.tar.gz";
    let dir = |r: &Reg| format!("/home/runner/{}-{}", r.to.replace('/', "-"), r.label);
    // JSON's quoting is YAML's: a key's comment or a file's quotes never break the document
    let q = |s: &str| Json::from(s).to_string();
    let keys: Vec<String> = keys.iter().map(|k| q(k)).collect();
    let file = |path: &str, text: &str| {
        format!(
            "  - path: {path}\n    defer: true\n    content: {}\n",
            q(text)
        )
    };
    let mut init = format!(
        "#cloud-config\ndisable_root: false\nssh_authorized_keys: [{}]\ngrowpart: {{mode: \"off\"}}\nresize_rootfs: false\n\
         bootcmd:\n  - {RESOLVER}\n  - {DHCLIENT}\n  - {}\n  - {}\npackage_update: true\npackages: [build-essential, pkg-config, libssl-dev, libfontconfig1, sudo, git, curl, jq, unzip, zstd, python3, docker.io, qemu-guest-agent, unattended-upgrades{caddy}]\n\
         write_files:\n  - path: /etc/apt/apt.conf.d/90lock-wait\n    content: {}\n{}{}{}runcmd:\n  - {}\n  - systemctl restart {restart}\n  - systemctl start qemu-guest-agent\n  - {SWAP}\n  - apt-get install -y libicu76 || apt-get install -y libicu72\n  \
         - useradd -m -s /bin/bash runner && usermod -aG docker runner\n  - {get}\n",
        keys.join(", "),
        q(KERNEL),
        q(GROWN),
        q(LOCK_WAIT),
        file("/etc/apt/apt.conf.d/20auto-upgrades", UPGRADES),
        file("/etc/docker/daemon.json", DOCKER),
        match caddyfile.is_empty() {
            true => String::new(),
            false => file("/etc/caddy/Caddyfile", caddyfile),
        },
        q(GROWN_FIRST),
    );
    for (i, (r, token)) in tokens.iter().enumerate() {
        let (d, labels, runner, to) = (dir(r), r.labels(), r.runner(name), &r.to);
        let config = format!(
            "sudo -u runner ./config.sh --unattended --no-default-labels --labels {labels} --name {runner} --url \
             https://github.com/{to} --token {token} --replace"
        );
        init += &format!("  - mkdir -p {d} && tar -xzf /home/runner/r.tgz -C {d}\n");
        // once, from the first folder, before any config.sh: the libraries are the box's, shared by every runner
        if i == 0 {
            init += &format!("  - {d}/bin/installdependencies.sh\n");
        }
        init += &format!(
            "  - cd {d} && echo NEXTEST_TEST_THREADS=2 >> .env && echo CARGO_BUILD_JOBS={jobs} >> .env && chown -R runner:runner . && {config}\n  \
             - cd {d} && ./svc.sh install runner && ./svc.sh start\n"
        );
    }
    init + "  - rm -f /home/runner/r.tgz\n"
}

/// A fresh token for each of `want`'s registrations, and the cloud-init that registers them all. A registration GitHub
/// gives no token is left out, never the others with it (/code-review): its runner stays missing, which the status names
/// and the next reinstall asks again; with no token at all, nothing is created or reinstalled.
fn init_for(
    want: &Want,
    keys: &[String],
    token: &dyn Fn(&Reg) -> Result<String, String>,
) -> Result<String, String> {
    let (mut tokens, mut refused) = (vec![], vec![]);
    for r in &want.registrations {
        match token(r) {
            Ok(t) => tokens.push((r, t)),
            Err(e) => refused.push(format!("{e} ({r})")),
        }
    }
    match tokens.is_empty() {
        true => Err(refused.join("; ")),
        false => Ok(cloud_init(want, &tokens, keys)),
    }
}

/// The server's Hostman comment for the cloud-init `want` and `keys` make: its sha256, the tokens left out, which change at
/// each call. A box whose comment differs runs another cloud-init and is reinstalled (task ci-box-open-and-current).
pub fn fingerprint(want: &Want, keys: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let tokens: Vec<(&Reg, String)> = (want.registrations.iter())
        .map(|r| (r, "TOKEN".to_string()))
        .collect();
    let sum = Sha256::digest(cloud_init(want, &tokens, keys));
    let hex: String = sum.iter().map(|b| format!("{b:02x}")).collect();
    format!("cloud-init sha256 {hex}")
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

/// The presets a create may take, from the live list: Cloud Servers presets holding `want`'s RAM and at least its
/// bandwidth, with no GPU, cheapest first (ties by id), none whose preset or location answered « no free node » in `refused`.
pub fn candidates<'a>(presets: &'a Json, want: &Want, refused: &[NoNode]) -> Vec<&'a Json> {
    let gone = |p: &Json| {
        (refused.iter())
            .any(|r| p["id"].as_i64() == Some(r.preset) || p["location"] == r.location.as_str())
    };
    let sized = |p: &Json| {
        p["ram"].as_i64() == Some(want.ram_mb) && p["bandwidth"].as_i64().unwrap_or(0) >= want.mbps
    };
    let mut fit: Vec<&Json> = (presets["server_presets"].as_array().into_iter().flatten())
        .filter(|p| sized(p) && p["gpu"].as_f64().is_none_or(|g| g == 0.0))
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
/// The preset's month against the credit left, before the create: the public IPv4 the box gets next is the hostman tool's
/// to price, on the create itself (one price, one place), and a refusal of its cap is held as this one is.
/// Ok(None) inside the credit, Ok(Some(refusal)) past it, which his leave alone lets through; Err when the finances are not
/// read, which nothing lets through.
fn within_credit(call: Call, preset_id: i64, price: f64) -> Result<Option<String>, String> {
    let f = &ask(call, "GET", "/api/v1/account/finances", Json::Null)?["finances"];
    let (Some(left), Some(running)) = (f["balance"].as_f64(), f["monthly_cost"].as_f64()) else {
        return Err("hostman: its finances hold no balance, so no create is priced".into());
    };
    let month = running + price;
    // the credit left in whole units: it drains each hour, and a reason holding its cents was a new alarm each 20 minutes
    Ok((month > left).then(|| {
        format!(
            "no server created: preset {preset_id} costs {price:.2} a month, {month:.2} with the servers running, past \
             the {:.0} of credit left",
            left.floor()
        )
    }))
}

/// The words of the hostman tool's cap refusing a write past the credit left.
const SPEND_CAP: &str = "past the spend cap";

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
    /// The server's public IPv4 as last listed: where `ssh root@` reaches it.
    pub ip: Option<String>,
    /// The `fingerprint` the last reinstall sent as its comment: listed back unchanged, its runners up, is Hostman keeping
    /// no comment, an alarm, never a reinstall each hour (/code-review).
    pub sent: Option<String>,
    /// The registrations whose runner GitHub listed online since the last reinstall or create: one of them missing again is
    /// a box that died under it, healed after `HEAL_MS` whatever `REINSTALL_MS` says. Never resumed after a restart: an « up »
    /// says nothing of a runner added since (/code-review).
    pub seen: Vec<String>,
    /// When it was last healed: past one heal, the next waits `REINSTALL_MS` (/code-review: a box that dies under each cold
    /// build would be reinstalled each 35 minutes, each a cold build again).
    pub healed: Option<i64>,
    /// A create the credit refused, or an IPv4 add refused or failed, and when: said again for `HEAL_MS` with no call, the
    /// credit moving slowly.
    pub poor: Option<(i64, String)>,
    /// When a reinstall Hostman refused as busy last hard-rebooted the box: one an hour at most.
    pub rebooted: Option<i64>,
    /// When the reinstall that held the box's Caddy for its kept store went: its push is asked each tick until done, for
    /// `HOLD_MS` at most (task ci-caddy-keeps-certs).
    pub held: Option<(i64, String)>,
}

/// What Hostman last read of `server` before a reinstall wipes it: its newest CPU load and RAM from its statistics, with
/// when, and its system disk's use from the listing. The one statistics path Hostman's OpenAPI documents (its bundle,
/// hostman.com/api-docs-data/bundle.json: `date_from`, `date_to`; Timeweb's sibling API also lists a CPU form,
/// `/statistics/{time_from}/{period}/{keys}`, which Hostman's does not). Read live on 2026-11-18 (server 9608429, 14:00 to
/// 17:53 UTC): UTC times, newest first, RAM and disk load every five minutes, CPU empty in every answer, and no point at all
/// from a runner's death to 35 to 50 minutes after it (14:56 to 15:51, 15:56 to 16:46, 16:58 to 17:49): the box stops
/// reporting while it is stuck. So the window is six hours, and the reading's time is part of what it says. Readings that do
/// not answer are said unread, and never hold the reinstall off.
fn readings(call: Call, server: &Json, id: i64, now_ms: i64) -> String {
    let at = |ms: i64| dash_core::iso(ms / 1000).trim_end_matches('Z').to_string();
    let path = format!(
        "/api/v1/servers/{id}/statistics?date_from={}&date_to={}",
        at(now_ms - 6 * 3_600_000),
        at(now_ms - 60_000)
    );
    let mb = |v: &Json| format!("{:.0}", v.as_f64().unwrap_or(0.0));
    let mut said = vec![];
    match ask(call, "GET", &path, Json::Null) {
        Err(e) => said.push(format!("cpu and ram unread: {e}")),
        Ok(s) => {
            // by its time, whatever the order: one format, so the text's order is the time's
            let last = |k: &str| {
                let points = s[k].as_array().into_iter().flatten();
                points
                    .max_by_key(|p| p["logged_at"].as_str().unwrap_or_default().to_string())
                    .cloned()
            };
            if let Some(c) = last("cpu") {
                said.push(format!(
                    "cpu {:.0}%",
                    c["load"].as_f64().unwrap_or(0.0) * 100.0
                ));
            }
            if let Some(r) = last("ram") {
                let when = r["logged_at"].as_str().unwrap_or("an unnamed time");
                said.push(format!(
                    "ram {} of {} MB at {when}",
                    mb(&r["used"]),
                    mb(&r["total"])
                ));
            }
        }
    }
    let disks = server["disks"].as_array().into_iter().flatten();
    if let Some(d) = disks
        .clone()
        .find(|d| d["is_system"] == true)
        .or(disks.clone().next())
    {
        said.push(format!(
            "disk {} of {} MB used",
            mb(&d["used"]),
            mb(&d["size"])
        ));
    }
    match said.is_empty() {
        true => "no reading listed".into(),
        false => format!("last readings: {}", said.join(", ")),
    }
}

/// The first public IPv4 Hostman lists on `server`.
fn public_ipv4(server: &Json) -> Option<String> {
    let nets = server["networks"].as_array().into_iter().flatten();
    let ips = nets
        .filter(|n| n["type"] == "public")
        .flat_map(|n| n["ips"].as_array().into_iter().flatten());
    ips.filter(|i| i["type"] == "ipv4")
        .find_map(|i| i["ip"].as_str().map(String::from))
}

/// One observation and the repair it allows. `online`: whether a registration's runner `want.name` is online on GitHub
/// with `LABEL`; `token`: a fresh registration token for one, asked only at a create or a reinstall. Whichever runner is
/// missing reinstalls the box with a fresh token for each: cloud-init is the one way the tool reaches it (his ssh key is
/// for him to look), so a reinstall is how a runner is registered again, under the one hourly limit and the one alarm whichever it is. One create or reinstall a tick at most; a
/// box of the wrong name or OS is reinstalled at once, and the right one only when GitHub said it has no runner: a GitHub,
/// or a resolver, that does not answer never wipes it. A box whose comment is not `fingerprint`, its runners all online, is
/// reinstalled once none is busy (task ci-box-open-and-current): a cloud-init changed here reaches the box, and no job is
/// cut. No reinstall, whatever its reason, wipes a busy runner. `keys` unread is Down: a box remade without them is shut.
pub fn reconcile(
    call: Call,
    online: &dyn Fn(&Reg) -> Result<Seen, String>,
    token: &dyn Fn(&Reg) -> Result<String, String>,
    keys: &dyn Fn() -> Result<Vec<String>, String>,
    (evidence, store, push): (Captured, Stored, Pushed),
    want: &Want,
    (state, now_ms): (&mut State, i64),
) -> Step {
    let State {
        since,
        installed,
        refused,
        ip,
        sent,
        seen,
        healed,
        poor,
        rebooted,
        held,
    } = state;
    let mut step = || -> Result<Step, String> {
        // read only where a cloud-init is sent or compared: a start or an IP needs no key (/code-review)
        let keyed = || {
            let keys = keys().map_err(|e| format!("no ssh key for root: {e}"))?;
            let print = fingerprint(want, &keys);
            Ok::<_, String>((keys, print))
        };
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
        let found = servers.iter().find(ours);
        *ip = found.and_then(public_ipv4);
        // a held Caddy's kept store pushed back over ssh, which starts it, once the box answers and its cloud-init is done
        // (task ci-caddy-keeps-certs): asked each tick, said once past an hour and left to the box's own timer
        if let (Some((at, install)), Some(push), Some(server)) = (held.clone(), push, found) {
            let id = server["id"].as_i64().unwrap_or(0);
            let pushed = match (server["status"] == "on", public_ipv4(server)) {
                (true, Some(ip)) => push(id, &ip, &install),
                _ => Err("the box is not on at a public IPv4".into()),
            };
            match pushed {
                Ok(()) => *held = None,
                Err(e) if now_ms - at >= HOLD_MS => {
                    *held = None;
                    return Err(format!(
                        "server {id}: Caddy's store not pushed back in an hour: {e}"
                    ));
                }
                Err(_) => {}
            }
        }
        let Some(server) = found else {
            *since = None;
            if let Some((_, why)) = poor.as_ref().filter(|(at, _)| now_ms - at < HEAL_MS) {
                return Err(why.clone());
            }
            let os_id = os_id(call, want.os)?;
            refused.retain(|r| now_ms - r.at_ms < NO_NODE_MS);
            let presets = ask(call, "GET", "/api/v1/presets/servers", Json::Null)?;
            let Some(&p) = candidates(&presets, want, refused).first() else {
                return Err(match refused.is_empty() {
                    true => format!(
                        "hostman: no Cloud Servers preset holds {} MB of RAM and {} Mbps",
                        want.ram_mb, want.mbps
                    ),
                    // no time in it: one alarm while it lasts (a reason that changes each tick sounds each tick)
                    false => "no free node in any preset with that RAM: each is asked again an hour after it refused".into(),
                });
            };
            let (id, price) = (
                p["id"].as_i64().unwrap_or(0),
                p["price"].as_f64().unwrap_or(0.0),
            );
            *poor = None;
            // past the credit left, his leave lets this box through until its day (the hostman tool holds the same)
            let leave = want.leave_until_ms.is_some_and(|until| now_ms < until);
            // the leave lets the credit's refusal alone through: an unread credit is an error, as before, for every box
            let by_leave = match within_credit(call, id, price) {
                Ok(Some(_)) if leave => true,
                Ok(Some(why)) | Err(why) => {
                    *poor = Some((now_ms, why.clone()));
                    return Err(why);
                }
                Ok(None) => false,
            };
            let (keys, print) = keyed()?;
            let body = json!({
                "name": want.name, "preset_id": id, "os_id": os_id, "bandwidth": p["bandwidth"],
                "is_ddos_guard": false, "ssh_keys_ids": want.ssh_keys_ids,
                "cloud_init": init_for(want, &keys, token)?, "comment": print,
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
                    let next = candidates(&presets, want, refused)
                        .first()
                        .map(|n| named(n));
                    let next = next.map_or("none left within the hour".into(), |n| {
                        format!("the next tick tries {n}")
                    });
                    return Ok(Step::NoNode(format!("preset {} refused; {next}", named(p))));
                }
                // the tool's cap, the IPv4 priced with the preset: held as a refusal here is
                Err(e) if e.contains(SPEND_CAP) => {
                    *poor = Some((now_ms, e.clone()));
                    return Err(e);
                }
                made => made?,
            };
            seen.clear();
            let id = made["server"]["id"].as_i64();
            let made = if by_leave {
                Step::CreatedByLeave
            } else {
                Step::Created
            };
            return id.map(made).ok_or("hostman: a create answered no server id, so none is counted: read GET /api/v1/servers".into());
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
            "no_paid" | "blocked" | "permanent_blocked" => {
                Err(format!("server {id} is {status}: his, in Hostman's panel"))
            }
            // installing, turning on, rebooting: Hostman is at work, which is no reason to count or reinstall
            s if AT_WORK.contains(&s) => Ok(Step::Busy(id, s.to_string())),
            s if s != "on" && s != "error" => Err(format!(
                "server {id} is {s}, which Hostman does not list as at work: his, in Hostman's panel"
            )),
            // « error » (unlisted in Hostman's OpenAPI) is a failed install: the box holds no data, so it is reinstalled
            // at once, under the one hourly limit
            _ => {
                let broken = status == "error";
                let (right_name, right_os) = as_wanted(server, want);
                // the registrations GitHub lists no runner for, and those it could not read: one unread never hides
                // another's missing runner (/code-review); a box in error is reinstalled whatever GitHub says
                let (mut missing, mut unread, mut busy) = (vec![], vec![], false);
                for r in want.registrations.iter().filter(|_| !broken) {
                    let (got, said) = (online(r), r.to_string());
                    if matches!(got, Ok(Seen::Idle | Seen::Busy)) && !seen.contains(&said) {
                        seen.push(said.clone());
                    }
                    match got {
                        Ok(Seen::Idle) => {}
                        Ok(Seen::Busy) => busy = true,
                        Ok(Seen::Gone) => missing.push(said),
                        Err(e) => unread.push(format!("{e} ({said})")),
                    }
                }
                // a runner seen since the last reinstall, missing now: its box died under it
                let vanished: Vec<&str> = (missing.iter())
                    .filter(|m| seen.contains(m))
                    .map(String::as_str)
                    .collect();
                let (keys, print) = keyed()?;
                // a list with no comment is no reading, and no reading wipes nothing
                let current = server["comment"].as_str().is_none_or(|c| c == print);
                let all_up =
                    !broken && right_name && right_os && missing.is_empty() && unread.is_empty();
                if all_up && current {
                    *since = None;
                    return Ok(Step::Up);
                }
                if all_up && sent.as_deref() == Some(print.as_str()) {
                    return Err(format!(
                        "server {id}: its comment is not the cloud-init hash its reinstall sent, so Hostman keeps no \
                         comment and no other reinstall follows"
                    ));
                }
                // its runners all up on an older cloud-init: held within the hour of the last
                if all_up && installed.is_some_and(|at| now_ms - at < REINSTALL_MS) {
                    *since = None;
                    return Ok(Step::Held(id));
                }
                // no reinstall wipes a busy runner: a missing one waits, counted from its first tick; with none missing
                // the box is held, its runners up, and a list unread past its 15 minutes is the alarm it was
                // ponytail: a job GitHub hands out between this read and the PATCH is cut; a drain needs ssh to the box
                if busy {
                    if !missing.is_empty() {
                        since.get_or_insert(now_ms);
                        return Ok(Step::Waiting(id, missing.join(", ")));
                    }
                    let Some(e) = unread.first() else {
                        *since = None;
                        return Ok(Step::Held(id));
                    };
                    if now_ms - *since.get_or_insert(now_ms) >= HEAL_MS {
                        return Err(format!(
                            "server {id}: GitHub's runner list cannot be read: {e}"
                        ));
                    }
                    return Ok(Step::Held(id));
                }
                // the OS's id before any spend: a list without it adds no IP and wipes nothing
                let os = match right_os {
                    true => None,
                    false => Some(os_id(call, want.os)?),
                };
                if unaddressed(server) {
                    // one attempt: an add refused (the tool's cap) or failed is said again with no call for `HEAL_MS`, as a
                    // create the credit refused, then tried once more (review of PR 1783)
                    if let Some((_, why)) = poor.as_ref().filter(|(at, _)| now_ms - at < HEAL_MS) {
                        return Err(why.clone());
                    }
                    if let Err(e) = post("/ips", json!({"type": "ipv4"})) {
                        *poor = Some((now_ms, e.clone()));
                        return Err(e);
                    }
                    *poor = None;
                    // its cloud-init ran with no network and runs once an instance: reinstalled on the next tick
                    *since = Some(now_ms - HEAL_MS);
                    return Ok(Step::Addressed(id));
                }
                // the right box waits for its runner; a wrong name or OS, or a box in error, is reinstalled at once
                if right_name && right_os && !broken && !all_up {
                    let on = *since.get_or_insert(now_ms);
                    if now_ms - on < HEAL_MS {
                        return Ok(Step::Waiting(id, missing.join(", ")));
                    }
                    // a reinstall only for a runner GitHub said is missing; no time or count in a reason: one that
                    // changes each tick sounds a new alarm each tick
                    if let (true, Some(e)) = (missing.is_empty(), unread.first()) {
                        return Err(format!(
                            "server {id}: GitHub's runner list cannot be read: {e}"
                        ));
                    }
                }
                // one reason whether it is in error or on with no runner: a box that moves between the two within the
                // hour is one alarm (/code-review). A runner that came up and vanished is past it: the install worked, and
                // the box died after (16:22 on 2026-11-18 waited its hour); one never seen keeps the hour for cloud-init
                // past one heal the next waits the hour: a box that dies under each cold build is not wiped each 35 minutes
                let died = right_name && right_os && !broken && !vanished.is_empty();
                let heal = died && healed.is_none_or(|at| now_ms - at >= REINSTALL_MS);
                if !heal && installed.is_some_and(|at| now_ms - at < REINSTALL_MS) {
                    return Err(format!(
                        "server {id}: {NO_RUNNER}, or in error, after its reinstall; the next is an hour after the last"
                    ));
                }
                // one PATCH: its name, a fresh token, and its os_id, which reinstalls it, the same OS as much as a new
                // one (Hostman logs « reinstall », 09:39 on 2026-11-15). Never the deprecated « install » action: on an
                // installed box it logged « install », wiped nothing, and left 9602127 in « error » (12:02).
                let os = match os {
                    Some(os) => os,
                    None => os_id(call, want.os)?,
                };
                let why = match () {
                    _ if broken => "it was in error".to_string(),
                    _ if !right_name => {
                        format!("it was named {}", server["name"].as_str().unwrap_or("?"))
                    }
                    _ if !right_os => {
                        let ran = server["os"]["name"].as_str().unwrap_or("an unnamed OS");
                        format!("it ran {ran}, not {}", want.os)
                    }
                    _ if died => format!("its runner for {} {VANISHED}", vanished.join(", ")),
                    _ if !missing.is_empty() => {
                        format!("no runner online for {}", missing.join(", "))
                    }
                    _ => "its cloud-init changed".into(),
                };
                let mut body = json!({"cloud_init": init_for(want, &keys, token)?, "os_id": os, "comment": &print});
                if !right_name {
                    body["name"] = json!(want.name);
                }
                // read before the disk goes, whatever answers: the reason a box died is lost with it otherwise. After the
                // tokens, so a refused token reads nothing, and next to the PATCH, the window a job lands in kept short
                let why = format!("{why}; {}", readings(call, server, id, now_ms));
                let (why, patched, hold) = reinstall(
                    call,
                    (evidence, store),
                    server,
                    (id, body, why),
                    (rebooted, now_ms),
                );
                // counted when it went through or may have; one never sent, or refused, is asked again the next tick.
                // Counted, its runners are no longer seen: a heal that may have gone through is never sent again
                if patched
                    .as_ref()
                    .map_or_else(|e| e.starts_with("hostman: no answer"), |_| true)
                {
                    *installed = Some(now_ms);
                    *held = hold.map(|install| (now_ms, install));
                    seen.clear();
                    if died {
                        *healed = Some(now_ms);
                    }
                }
                patched?;
                *sent = Some(print);
                *since = None;
                Ok(match died {
                    true => Step::Healed(id, why),
                    false => Step::Reinstalled(id, why),
                })
            }
        }
    };
    step().unwrap_or_else(Step::Down)
}

/// The one way to reinstall a box, whatever the reason (a heal, a runner never seen, a cloud-init changed, a wrong name or
/// OS, a box in error): its evidence kept first (`evidence`, within the tick's 90 s), then the PATCH of its os_id that
/// wipes it, sent whatever the capture got. A reinstall wipes the only proof of why the box hung: ci-2 lost it twice on
/// 2026-11-19 (a heal at 03:43:09, then a « no runner online » after a restart forgot it had been up; task
/// ci-heal-evidence). Only a box on with a public IPv4 is captured: one off, being made or with no address has nothing
/// to read, and its reinstall says so, never spending the 90 s on it. No other code sends this PATCH
/// (`no_code_reinstalls_but_through_the_one_function_that_captures_first`), nor a reboot: a reinstall Hostman refuses as
/// busy hard-reboots the box here, after its capture, one an hour at most (`rebooted`), and the next tick asks the
/// reinstall again. The old ci box answered vds_is_busy for 12 hours after a soft reboot of its hung guest, which had no
/// agent to obey it; Hostman's hard reboot, done at the infrastructure level with no agent, freed it (07:16:55 on
/// 2026-11-19; task ci-box-agent-reboot). Out: the reason with what the capture kept, and the PATCH's answer.
fn reinstall(
    call: Call,
    (evidence, store): (Captured, Stored),
    server: &Json,
    (id, mut body, why): (i64, Json, String),
    (rebooted, now_ms): (&mut Option<i64>, i64),
) -> (String, Result<Json, String>, Option<String>) {
    let on = (server["status"] == "on")
        .then(|| public_ipv4(server))
        .flatten();
    let kept = evidence.map(
        |evidence| match (server["status"].as_str(), public_ipv4(server)) {
            (Some("on"), Some(ip)) => evidence(id, &ip),
            (Some("on"), None) => "not captured: no public IPv4 listed".into(),
            (status, _) => format!(
                "not captured: the box is « {} », not on",
                status.unwrap_or("?")
            ),
        },
    );
    // Caddy's certificates kept off the box before the wipe, its new Caddy held until they are pushed back over ssh (task
    // ci-caddy-keeps-certs): no reinstall asks Let's Encrypt again for a name it holds, and no secret rides in cloud-init
    let held = match (
        store.is_some_and(|s| s(id, on.as_deref())),
        body["cloud_init"].as_str(),
    ) {
        (true, Some(init)) if init.contains("  - systemctl restart caddy docker\n") => {
            // a fresh id for this install, so its push never lands on the system about to be wiped
            let install = install_id();
            body["cloud_init"] = json!(with_hold(init, &install));
            Some(install)
        }
        _ => None,
    };
    let and = |text: String| match &kept {
        Some(kept) => format!("{text}; {kept}"),
        None => text,
    };
    // a refusal names the capture too: refused tick after tick (vds_is_busy), its status reads the file of the hour
    let patched = ask(call, "PATCH", &format!("/api/v1/servers/{id}"), body);
    let patched = match patched {
        Err(e)
            if e.contains("vds_is_busy")
                && rebooted.is_none_or(|at| now_ms - at >= REINSTALL_MS) =>
        {
            *rebooted = Some(now_ms);
            let hard = json!({"action": "hard_reboot"});
            let said = match ask(call, "POST", &format!("/api/v1/servers/{id}/action"), hard) {
                Ok(_) => "hard-rebooted, the reinstall asked again next".to_string(),
                Err(r) => format!("its hard reboot failed: {r}"),
            };
            Err(format!("{e}; {said}"))
        }
        other => other,
    };
    (and(why), patched.map_err(and), held)
}

/// The job model the core's queue runs each tick: its rows are `ci_runner/<name>`'s `status` and, once per reason, `alarm`
/// (empty again when it is up, and only then: a reinstall or a wait between two Downs of one reason is no new alarm). Never
/// silent: a Down is a status and an alarm, and a Down of another reason a new alarm.
pub struct Runner {
    want: Want,
    call: Tool,
    online: Ask<Seen>,
    token: Ask<String>,
    keys: Keys,
    evidence: Option<Capture>,
    store: Option<Store>,
    push: Option<Push>,
    state: RefCell<State>,
    /// The alarm last written.
    alarmed: RefCell<String>,
    now: Box<dyn Fn() -> i64>,
}

impl Runner {
    pub fn new(want: Want, call: Tool, online: Ask<Seen>, token: Ask<String>) -> Self {
        Self {
            want,
            call,
            online,
            token,
            keys: Box::new(|| Ok(vec![])),
            evidence: None,
            store: None,
            push: None,
            state: RefCell::default(),
            alarmed: RefCell::default(),
            now: Box::new(|| 0),
        }
    }

    /// Whether a reinstall kept the box's Caddy store off it (`store`): nothing kept, and Caddy never held, unless given.
    pub fn stored(mut self, store: Store) -> Self {
        self.store = Some(store);
        self
    }

    /// How a held Caddy's store goes back to its box (`push`).
    pub fn pushed(mut self, push: Push) -> Self {
        self.push = Some(push);
        self
    }

    /// What a reinstall keeps of the box before it wipes it (`evidence`): nothing, and nothing said, unless given.
    pub fn evidenced(mut self, evidence: Capture) -> Self {
        self.evidence = Some(evidence);
        self
    }

    /// The clock a refusal's hour is counted on: the host's.
    pub fn clocked(mut self, now: Box<dyn Fn() -> i64>) -> Self {
        self.now = now;
        self
    }

    /// The ssh public keys root takes: none unless given.
    pub fn keyed(mut self, keys: Keys) -> Self {
        self.keys = keys;
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
            &*self.keys,
            (
                self.evidence.as_deref(),
                self.store.as_deref(),
                self.push.as_deref(),
            ),
            &self.want,
            (&mut self.state.borrow_mut(), (self.now)()),
        );
        let row = |field: &str, text: String| FieldWrite {
            table: "ci_runner".into(),
            row: self.want.name.clone(),
            field: field.into(),
            value: Value::Text(text),
        };
        let mut rows = vec![row("status", step.text())];
        // each tick, so the page's window always holds it
        if let Some(ip) = &self.state.borrow().ip {
            rows.push(FieldWrite {
                table: "ci_box".into(),
                ..row("ip", ip.clone())
            });
        }
        let alarm = match &step {
            Step::Down(why) => why.clone(),
            // the box died under its runner: why and its last readings, once, cleared once it is up again
            Step::Healed(..) => step.text(),
            Step::Up | Step::Held(_) => String::new(),
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
