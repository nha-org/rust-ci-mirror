//! A process's numbers, shared by main.rs and tasks.rs: main.rs names tasks.rs, so tasks.rs may not name main.rs back.
use dash_meter::cap::heap;
use dash_meter::memory::memory;
use dash_statsd::Statsd;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// How often a process says its memory.
const MEMORY_EVERY: Duration = Duration::from_secs(30);

/// This process's numbers, its memory from now on.
pub fn reporting(dir: &Path, command: &str) -> Arc<Statsd> {
    let report = Arc::new(Statsd::new(dir, command));
    let reader = Arc::clone(&report);
    std::thread::spawn(move || {
        loop {
            say_memory(&reader);
            std::thread::sleep(MEMORY_EVERY);
        }
    });
    report
}

/// Also a process's last words: what it grew to in its last 30 s would go unsaid. A meter that panics costs a number, never a
/// run's outcome line or this thread.
pub fn say_memory(report: &Statsd) {
    // What the cap counts, beside the footprint below. A process stopped at the cap says nothing more: its line is on stderr.
    let counted = heap();
    report.gauge("memory.heap_mb", counted.current_mb);
    report.highest("memory.heap_peak_mb", counted.peak_mb);
    let Ok(m) = std::panic::catch_unwind(memory) else {
        return;
    };
    report.gauge("memory.current_mb", m.current_mb);
    report.highest("memory.peak_mb", m.peak_mb);
}
