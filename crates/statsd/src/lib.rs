//! The tool's own numbers, as statsd lines over UDP to whatever listens on loopback: the fewest dependencies there are, none,
//! and no key here (the listener holds its own). Tooling beside the rings: only the binary takes it, and the core never hears
//! of it (@nha, 2026-11-06: « integrate datadog in rust too. different app. so we don't grow accidentally. »).
//!
//! The switch is one file in the data folder, read before each send, so `dash telemetry on|off` also acts on what is running.
//! No file is off: a test's or CI's fresh folder sends nothing, and `make rust/serve` turns the live folder on once.
use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SERVICE: &str = "dash-rust";
const SWITCH: &str = "telemetry";
/// The statsd port's convention.
const ENDPOINT: &str = "127.0.0.1:8125";
const OFF: &str = "off";

/// `on` or `off`, kept in `dir` across restarts.
pub fn set(dir: &Path, on: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let value = if on { ENDPOINT } else { OFF };
    std::fs::write(dir.join(SWITCH), value)
}

/// Where the numbers go, or None while off. Only an address counts as on: a host name would look itself up at each send, on the
/// serial request loop, and any other text, `off` too, is off.
pub fn endpoint(dir: &Path) -> Option<SocketAddr> {
    let text = std::fs::read_to_string(dir.join(SWITCH)).ok()?;
    text.trim().parse().ok()
}

pub struct Statsd {
    dir: PathBuf,
    tags: String,
    socket: Option<UdpSocket>,
}

impl Statsd {
    /// `command` tells one process's numbers from another's on the same service.
    pub fn new(dir: &Path, command: &str) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").ok();
        // A full send buffer drops a number and never holds the caller.
        let socket = socket.filter(|s| s.set_nonblocking(true).is_ok());
        Self {
            dir: dir.to_path_buf(),
            tags: format!("service:{SERVICE},command:{command}"),
            socket,
        }
    }

    /// Fire and forget: nothing listening costs the tool nothing and tells it nothing.
    // ponytail: one small file read a number, on the serial request loop too, where the next request's route_* holds the wait;
    // a cached switch if it ever shows there.
    fn send(&self, line: &str) {
        if let (Some(to), Some(socket)) = (endpoint(&self.dir), &self.socket) {
            let _ = socket.send_to(line.as_bytes(), to);
        }
    }

    pub fn gauge(&self, name: &str, value: f64) {
        self.send(&format!("dash_rust.{name}:{value:.1}|g|#{}", self.tags));
    }

    /// For a number whose highest matters: a gauge keeps the last sent per tag set, so two processes of one command would hide
    /// each other's. The agent's `.max` series keeps the highest, of however many processes.
    pub fn highest(&self, name: &str, value: f64) {
        self.send(&format!("dash_rust.{name}:{value:.1}|h|#{}", self.tags));
    }

    /// The process's time so far, to its first useful answer.
    pub fn started(&self, since: Instant) {
        let ms = since.elapsed().as_secs_f64() * 1000.0;
        self.send(&format!("dash_rust.start_ms:{ms:.1}|ms|#{}", self.tags));
    }

    pub fn step_took(&self, step: &str, took: Duration) {
        let ms = took.as_secs_f64() * 1000.0;
        let tags = &self.tags;
        self.send(&format!("dash_rust.step_ms:{ms:.1}|ms|#{tags},step:{step}"));
    }

    pub fn step<T>(&self, step: &str, work: impl FnOnce() -> T) -> T {
        let at = Instant::now();
        let value = work();
        self.step_took(step, at.elapsed());
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;

    fn listener() -> UdpSocket {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("bind");
        socket
            .set_read_timeout(Some(Duration::from_millis(200)))
            .expect("timeout");
        socket
    }

    fn line(socket: &UdpSocket) -> Option<String> {
        let mut buf = [0u8; 512];
        let n = socket.recv(&mut buf).ok()?;
        Some(String::from_utf8_lossy(&buf[..n]).into_owned())
    }

    #[test]
    fn each_kind_is_a_line_under_the_service() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("statsd-lines");
        let socket = listener();
        let to = socket.local_addr().expect("address").to_string();
        std::fs::write(dir.0.join(SWITCH), &to).expect("the switch");
        let report = Statsd::new(&dir.0, "serve");
        report.gauge("memory.current_mb", 12.34);
        assert_eq!(
            line(&socket).expect("a gauge"),
            "dash_rust.memory.current_mb:12.3|g|#service:dash-rust,command:serve"
        );
        report.highest("memory.peak_mb", 150.0);
        assert_eq!(
            line(&socket).expect("a highest"),
            "dash_rust.memory.peak_mb:150.0|h|#service:dash-rust,command:serve"
        );
        report.step_took("open_store", Duration::from_millis(5));
        assert_eq!(
            line(&socket).expect("a step"),
            "dash_rust.step_ms:5.0|ms|#service:dash-rust,command:serve,step:open_store"
        );
        assert_eq!(
            report.step("pick", || 7),
            7,
            "a step answers its work's value"
        );
        assert!(line(&socket).expect("its time").contains("step:pick"));
        report.started(Instant::now());
        assert!(
            line(&socket)
                .expect("the start")
                .starts_with("dash_rust.start_ms:")
        );
    }

    #[test]
    fn no_file_or_off_sends_nothing_and_acts_on_a_running_process() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("statsd-switch");
        let socket = listener();
        let to = socket.local_addr().expect("address").to_string();
        let report = Statsd::new(&dir.0, "serve");
        assert_eq!(endpoint(&dir.0), None, "a fresh folder is off");
        report.gauge("a", 1.0);
        assert_eq!(line(&socket), None);

        std::fs::write(dir.0.join(SWITCH), &to).expect("on");
        report.gauge("a", 1.0);
        assert!(line(&socket).is_some(), "on, with no restart");

        set(&dir.0, false).expect("off");
        assert_eq!(endpoint(&dir.0), None);
        report.gauge("a", 1.0);
        assert_eq!(line(&socket), None, "off, with no restart");

        set(&dir.0, true).expect("on");
        assert_eq!(endpoint(&dir.0), ENDPOINT.parse().ok());

        for not_an_address in ["on", "localhost:8125", ""] {
            std::fs::write(dir.0.join(SWITCH), not_an_address).expect("the text");
            assert_eq!(endpoint(&dir.0), None, "{not_an_address}");
        }
    }
}
