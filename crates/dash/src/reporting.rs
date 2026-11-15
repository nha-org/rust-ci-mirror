//! A process's numbers, shared by main.rs and tasks.rs: main.rs names tasks.rs, so tasks.rs may not name main.rs back.
use dash_ext as ext;
use dash_meter::cap::heap;
use dash_meter::memory::memory;
use dash_statsd::Statsd;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// How often a process says its memory.
pub(crate) const MEMORY_EVERY: Duration = Duration::from_secs(30);

// Statics: the allocator holds a plain fn. OnceLock's get takes no lock and is empty until the value is whole.
static LAST_WORDS: OnceLock<Arc<Statsd>> = OnceLock::new();

/// What a stop at the cap does before its stderr line. Inside the allocator: no allocation, no lock.
pub fn at_stop() {
    ext::pod::kill_running();
    if let Some(report) = LAST_WORDS.get() {
        report.say_stop();
    }
}

/// The process's one report: its first caller names it, so its numbers and its stop line share one folder and one command.
pub(crate) fn arm(dir: &Path, command: &str) -> Arc<Statsd> {
    let report = LAST_WORDS.get_or_init(|| {
        let report = Arc::new(Statsd::new(dir, command));
        report.hold();
        crate::statements::sink(dir, Arc::clone(&report), dash_statsd::held);
        report
    });
    Arc::clone(report)
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

/// What a round does with the footprint it read, set by main: watchers.rs names tasks.rs, which names this module.
pub static ROUND: OnceLock<fn(&Path, &str, f64)> = OnceLock::new();

/// This process's numbers, its memory from now on, each round's footprint handed to `ROUND`.
pub fn reporting(dir: &Path, command: &str) -> Arc<Statsd> {
    let report = arm(dir, command);
    let (reader, dir, command) = (Arc::clone(&report), dir.to_path_buf(), command.to_string());
    std::thread::spawn(move || {
        loop {
            reader.hold();
            if let (Some(mb), Some(round)) = (say_memory(&reader), ROUND.get()) {
                round(&dir, &command, mb);
            }
            std::thread::sleep(MEMORY_EVERY);
        }
    });
    report
}

/// Also a process's last words: what it grew to in its last 30 s would go unsaid. A meter that panics costs that round's
/// numbers, never a run's outcome line or this thread. Its footprint, when read.
pub fn say_memory(report: &Statsd) -> Option<f64> {
    // What the cap counts, beside the footprint. A process stopped at the cap sends `at_stop`'s one line instead.
    let (counted, m) = (heap(), std::panic::catch_unwind(memory).ok()?);
    report.memory(
        (counted.current_mb, counted.peak_mb),
        (m.current_mb, m.peak_mb),
    );
    Some(m.current_mb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;
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

    /// `serve --port` finds its folder late, and `reporting` is what arms it.
    #[dash_test]
    fn reporting_arms_its_process() {
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
    #[dash_test]
    fn a_stop_says_only_what_the_switch_last_said() {
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
        report.hold();
        assert_eq!(
            stop_lines(&listener),
            ["dash_rust.memory.cap_stop:1|c|#service:dash-rust,command:run"]
        );

        statsd::set(&dir.0, false).expect("off");
        report.hold();
        assert_eq!(stop_lines(&listener), Vec::<String>::new(), "off");
    }
}
