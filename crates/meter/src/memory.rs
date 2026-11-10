//! Section 4's reading: lifetime max phys_footprint on the Mac (Dashboard.ProcTop and graal's sysmem read the same
//! counter), the kernel's resident high-water mark on Linux CI.
pub struct Memory {
    pub current_mb: f64,
    pub peak_mb: f64,
}

const MB: f64 = 1024.0 * 1024.0;

#[cfg(target_os = "macos")]
pub fn memory() -> Memory {
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    let ptr = &mut info as *mut libc::rusage_info_v4 as *mut libc::rusage_info_t;
    let rc = unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, ptr) };
    assert_eq!(rc, 0, "proc_pid_rusage failed");
    let current_mb = info.ri_phys_footprint as f64 / MB;
    // The kernel updates the lifetime max lazily: a test read it 0.01 MB under the current figure.
    Memory {
        current_mb,
        peak_mb: current_mb.max(info.ri_lifetime_max_phys_footprint as f64 / MB),
    }
}

#[cfg(not(target_os = "macos"))]
pub fn memory() -> Memory {
    let status = std::fs::read_to_string("/proc/self/status").expect("/proc/self/status");
    let field = |name: &str| -> f64 {
        let line = status.lines().find(|l| l.starts_with(name)).expect(name);
        let kb: f64 = line
            .split_whitespace()
            .nth(1)
            .expect("a value")
            .parse()
            .expect("a number");
        kb * 1024.0 / MB
    };
    Memory {
        current_mb: field("VmRSS:"),
        peak_mb: field("VmHWM:"),
    }
}

/// A test's own budget, read at its end: section 4's « a test declares its budget, 50 MB by default; past it, the test
/// fails with its number ». Only honest with each test in its own process, so outside nextest it refuses: in `cargo test`
/// one test's peak failed another, by turns. crates/gate/tests/every_test_has_a_budget.rs fails a test that declares none.
pub struct TestBudget {
    mb: f64,
}

pub const DEFAULT_TEST_MB: f64 = 50.0;

impl TestBudget {
    pub fn new() -> Self {
        Self::of(DEFAULT_TEST_MB)
    }

    pub fn of(mb: f64) -> Self {
        assert!(
            std::env::var_os("NEXTEST").is_some(),
            "run under cargo nextest: a test's peak needs its own process"
        );
        Self { mb }
    }
}

impl Default for TestBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TestBudget {
    fn drop(&mut self) {
        let peak = memory().peak_mb;
        if peak > self.mb && !std::thread::panicking() {
            panic!(
                "this test's peak {peak:.1} MB is past its budget {} MB",
                self.mb
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Touched, not only allocated: untouched pages count in no footprint.
    fn touch(mb: usize) -> Vec<u8> {
        let mut v = vec![0u8; mb * 1024 * 1024];
        v.iter_mut().step_by(4096).for_each(|b| *b = 1);
        v
    }

    #[test]
    fn the_reading_follows_what_was_touched() {
        let _budget = TestBudget::of(500.0);
        let before = memory().peak_mb;
        let held = touch(64);
        let m = memory();
        assert!(
            m.peak_mb >= before + 50.0,
            "peak {} from {before}",
            m.peak_mb
        );
        assert!(m.peak_mb >= m.current_mb);
        drop(held);
    }

    #[test]
    #[should_panic(expected = "past its budget 50 MB")]
    fn a_test_past_its_default_budget_fails_with_its_number() {
        let _budget = TestBudget::new();
        std::hint::black_box(touch(64));
    }

    #[test]
    fn a_small_test_stays_inside_its_default_budget() {
        let _budget = TestBudget::new();
        assert!(memory().peak_mb < DEFAULT_TEST_MB);
    }
}
