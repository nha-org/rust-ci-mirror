//! A process's numbers, shared by main.rs and tasks.rs: main.rs names tasks.rs, so tasks.rs may not name main.rs back.
use dash_ext as ext;
use dash_meter::cap::heap;
use dash_meter::memory::memory;
use dash_statsd::Statsd;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// How often a process says its memory.
const MEMORY_EVERY: Duration = Duration::from_secs(30);

// Statics: the allocator holds a plain fn. OnceLock's get takes no lock and is empty until the value is whole.
static LAST_WORDS: OnceLock<Arc<Statsd>> = OnceLock::new();
// What the switch said when last read, 0 while off. The stop reads no file: a stalled data folder must not hold it, as a stuck
// stderr may not (cap.rs). So a switch change reaches a stop at the next round of `reporting`, and never in a process with none.
static HELD_ADDRESS: AtomicU64 = AtomicU64::new(0);

/// An address as one integer, 0 for off. IPv4 only: `Statsd` sends from 127.0.0.1, which could not reach any other.
fn packed(to: Option<SocketAddr>) -> u64 {
    match to {
        Some(SocketAddr::V4(to)) => (u64::from(to.ip().to_bits()) << 16) | u64::from(to.port()),
        _ => 0,
    }
}

fn unpacked(held: u64) -> Option<SocketAddr> {
    let ip = Ipv4Addr::from_bits((held >> 16) as u32);
    (held != 0).then(|| SocketAddr::V4(SocketAddrV4::new(ip, held as u16)))
}

/// What a stop at the cap does before its stderr line. Inside the allocator: no allocation, no lock.
pub fn at_stop() {
    ext::pod::kill_running();
    let held = unpacked(HELD_ADDRESS.load(Ordering::Relaxed));
    if let (Some(report), Some(to)) = (LAST_WORDS.get(), held) {
        report.say_stop(to);
    }
}

fn hold_switch(report: &Statsd) {
    HELD_ADDRESS.store(packed(report.switch()), Ordering::Relaxed);
}

/// The process's one report: its first caller names it, so its numbers and its stop line share one folder and one command.
fn arm(dir: &Path, command: &str) -> Arc<Statsd> {
    let report = LAST_WORDS.get_or_init(|| {
        let report = Arc::new(Statsd::new(dir, command));
        hold_switch(&report);
        report
    });
    Arc::clone(report)
}

/// A long-lived process's round.
fn tick(report: &Statsd) {
    hold_switch(report);
    say_memory(report);
}

/// The folder of a command that sends no number, armed so its stop says so all the same.
pub fn armed<'a>(dir: &'a str, command: &str) -> &'a Path {
    let path = Path::new(dir);
    arm(path, command);
    path
}

/// A read on a thread of its own, its root span named by its command, as `dash task issues` is « task issues » (task
/// rust-page-latency: a page waited 10 s and more on reads no span covered). No report armed, as in a test, is no span.
/// The span is the thread's current one while it reads, so a store read inside is its child; it finishes on a panic too, so
/// the read that failed still leaves its trace (/code-review of rust-page-latency).
pub fn read<T>(command: &str, work: impl FnOnce() -> T) -> T {
    let span = LAST_WORDS.get().map(|r| r.span(READ_SPAN, command));
    dash_statsd::set_current(span.clone());
    let _finish = Finished(span);
    work()
}

/// A read's span, finished and no longer current when the read ends, by its return or its panic.
struct Finished(Option<dash_statsd::Span>);

impl Drop for Finished {
    fn drop(&mut self) {
        dash_statsd::set_current(None);
        if let Some(span) = self.0.take() {
            span.finish();
        }
    }
}

/// The operation of every background read's span: its resource is its command.
pub const READ_SPAN: &str = "older_app.read";

/// The process's report, its traces to `to`: a test's listener in place of the agent.
#[cfg(test)]
pub fn arm_to(dir: &Path, to: std::net::SocketAddr) {
    let _ = LAST_WORDS.set(Arc::new(Statsd::with_traces(dir, "test", to)));
}

/// This process's numbers, its memory from now on.
pub fn reporting(dir: &Path, command: &str) -> Arc<Statsd> {
    let report = arm(dir, command);
    let reader = Arc::clone(&report);
    std::thread::spawn(move || {
        loop {
            tick(&reader);
            std::thread::sleep(MEMORY_EVERY);
        }
    });
    report
}

/// Also a process's last words: what it grew to in its last 30 s would go unsaid. A meter that panics costs a number, never a
/// run's outcome line or this thread.
pub fn say_memory(report: &Statsd) {
    // What the cap counts, beside the footprint below. A process stopped at the cap sends `at_stop`'s one line instead.
    let counted = heap();
    report.gauge("memory.heap_mb", counted.current_mb);
    report.highest("memory.heap_peak_mb", counted.peak_mb);
    let Ok(m) = std::panic::catch_unwind(memory) else {
        return;
    };
    report.gauge("memory.current_mb", m.current_mb);
    report.highest("memory.peak_mb", m.peak_mb);
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::memory::TestBudget;
    use dash_meter::{TestDir, drain};
    use dash_statsd as statsd;
    use std::net::UdpSocket;

    fn listener() -> UdpSocket {
        let listener = UdpSocket::bind("127.0.0.1:0").expect("a listener");
        listener
            .set_read_timeout(Some(Duration::from_millis(100)))
            .expect("its timeout");
        listener
    }

    /// A stop now, and the lines of one the listener hears: a round's gauges come too.
    fn stop_lines(listener: &UdpSocket) -> Vec<String> {
        at_stop();
        let mut lines = drain(listener);
        lines.retain(|line| line.contains("cap_stop"));
        lines
    }

    #[test]
    fn an_address_survives_its_packing_and_only_ipv4_is_on() {
        let _budget = TestBudget::new();
        let to: SocketAddr = "127.0.0.1:8125".parse().expect("an address");
        assert_eq!(unpacked(packed(Some(to))), Some(to));
        assert_eq!(unpacked(packed(None)), None);
        let v6: SocketAddr = "[::1]:8125".parse().expect("an address");
        assert_eq!(unpacked(packed(Some(v6))), None);
    }

    /// `serve --port` finds its folder late, and `reporting` is what arms it.
    #[test]
    fn reporting_arms_its_process() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("reporting-arms");
        let listener = listener();
        let to = listener.local_addr().expect("its address").to_string();
        std::fs::write(dir.0.join("telemetry"), to).expect("on");
        assert_eq!(stop_lines(&listener), Vec::<String>::new());
        let _report = reporting(&dir.0, "serve");
        assert_eq!(
            stop_lines(&listener),
            ["dash_rust.memory.cap_stop:1|c|#service:dash-rust,command:serve"]
        );
    }

    /// `orch` and `serve` live for days: what the switch says at a round reaches the stop, on and off.
    #[test]
    fn a_stop_says_only_what_the_switch_last_said() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("reporting-stop");
        let listener = listener();
        let report = arm(&dir.0, "run");
        assert_eq!(
            stop_lines(&listener),
            Vec::<String>::new(),
            "no file is off"
        );

        let to = listener.local_addr().expect("its address").to_string();
        std::fs::write(dir.0.join("telemetry"), to).expect("on");
        tick(&report);
        assert_eq!(
            stop_lines(&listener),
            ["dash_rust.memory.cap_stop:1|c|#service:dash-rust,command:run"]
        );

        statsd::set(&dir.0, false).expect("off");
        tick(&report);
        assert_eq!(stop_lines(&listener), Vec::<String>::new(), "off");
    }
}
