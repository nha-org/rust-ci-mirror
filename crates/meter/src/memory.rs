//! Section 4's reading: lifetime max phys_footprint on the Mac (Dashboard.ProcTop and graal's sysmem read the same
//! counter), the kernel's resident high-water mark on Linux CI.
pub struct Memory {
    pub current_mb: f64,
    pub peak_mb: f64,
    /// Pages mapped from files, the binary's code among them: Linux's RssFile. The Mac's phys_footprint counts no clean
    /// file page, so 0 there.
    pub file_mb: f64,
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
        file_mb: 0.0,
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
        file_mb: field("RssFile:"),
    }
}

/// A test's own budget, read at its end: section 4's « a test declares its budget, 50 MB by default; past it, the test
/// fails with its number ». The number is the peak above the footprint when the budget starts, so the binary's own size
/// (it grows with each merge) never fails a test that did not grow: PR 1368 failed an untouched test at 50.2 MB of 50
/// (task rust-test-budget-growth). Code the test runs faults in pages of the binary as it goes: Linux counts them in its
/// peak, so the file pages mapped since the start and still mapped at the reading are taken off; a file the test unmapped
/// before it counts as its own use. The guard is a test's first statement
/// (crates/gate/tests/every_test_has_a_budget.rs), so what the test builds counts. Only honest with each test in its own process, so outside nextest it refuses: in `cargo test`
/// one test's peak failed another, by turns. crates/gate/tests/every_test_has_a_budget.rs fails a test that declares none.
pub struct TestBudget {
    mb: f64,
    start_mb: f64,
    start_file_mb: f64,
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
        let start = memory();
        Self {
            mb,
            start_mb: start.current_mb,
            start_file_mb: start.file_mb,
        }
    }
}

impl Default for TestBudget {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TestBudget {
    fn drop(&mut self) {
        let now = memory();
        let peak = now.peak_mb;
        // ponytail: file pages mapped after the peak are taken off it too, so a test can read up to that much under; and
        // clean file pages the kernel reclaims before the reading count, so memory pressure can read a test over. A
        // sampled anonymous peak if either ever shows on CI.
        let grown = peak - self.start_mb - (now.file_mb - self.start_file_mb).max(0.0);
        if grown > self.mb && !std::thread::panicking() {
            panic!(
                "this test's peak {grown:.1} MB above its start ({:.1} of {peak:.1} MB) is past its budget {} MB",
                self.start_mb, self.mb
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

    /// Touches 64 MB on purpose, past the default, to see the reading move.
    #[dash_meter::dash_test(TestBudget::of(100.0))]
    fn the_reading_follows_what_was_touched() {
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

    /// Task rust-test-budget-attr, done-when 1: a test with no budget line still runs inside the attribute's 50 MB.
    #[dash_meter::dash_test]
    #[should_panic(expected = "past its budget 50 MB")]
    fn a_test_past_its_default_budget_fails_with_its_number() {
        std::hint::black_box(touch(64));
    }

    /// A body's inner attribute stays first, the budget after it (/code-review of rust-test-budget-attr).
    #[dash_meter::dash_test]
    fn a_small_test_stays_inside_its_default_budget() {
        #![allow(clippy::unit_arg)]
        #[allow(unused_variables)]
        let unused = ();
        std::hint::black_box(touch(8));
    }

    /// Done-when 1: a process 64 MB bigger before the test starts fails no test that did not grow. Its own budget holds the
    /// 72 MB it touches with room to spare: the helper's is the one under test (/code-review).
    #[dash_meter::dash_test(TestBudget::of(200.0))]
    fn a_bigger_process_at_the_start_fails_no_test() {
        let binary = touch(64);
        grows_by_8_mb_from(64.0);
        drop(binary);
    }

    /// The guard comes first in a test (the gate's rule), so this one starts inside a helper.
    fn grows_by_8_mb_from(at_least: f64) {
        let _budget = TestBudget::new();
        assert!(_budget.start_mb >= at_least, "{}", _budget.start_mb);
        assert!(memory().peak_mb > DEFAULT_TEST_MB);
        std::hint::black_box(touch(8));
    }

    /// Done-when 1: a bigger binary's code is pages mapped from a file and faulted in as the test runs, mapped to its end.
    /// 64 MB of a file read through a map still live at the reading fail no test.
    #[dash_meter::dash_test]
    fn file_pages_the_test_faults_in_fail_no_test() {
        let (map, len) = read_through_a_map(64);
        // The reading comes with the map live (CI read 63.9 MB above start on 44992407b when it came after the unmap).
        // `_budget` keeps the gate's name, so its drop is spelled out here.
        drop(_budget);
        unsafe { libc::munmap(map, len) };
    }

    /// A file the test unmapped is its own use: Linux's peak kept the pages, RssFile no longer holds them.
    #[cfg(target_os = "linux")]
    #[dash_meter::dash_test]
    #[should_panic(expected = "past its budget 50 MB")]
    fn a_file_unmapped_before_the_reading_counts() {
        let (map, len) = read_through_a_map(64);
        unsafe { libc::munmap(map, len) };
    }

    /// Maps `mb` of a file and reads a byte a page. The file is unlinked once mapped, so a panic leaves nothing on disk.
    fn read_through_a_map(mb: usize) -> (*mut libc::c_void, usize) {
        // Beside the test binary, on the disk its code is read from: a tmpfs /tmp counts its pages as shared, not file.
        let path = std::env::current_exe()
            .expect("the binary")
            .with_extension(format!("file-pages-{}", std::process::id()));
        let len = mb * 1024 * 1024;
        // Written a megabyte at a time: one buffer of the whole would be the test's own growth.
        let mut out = std::fs::File::create(&path).expect("the file");
        let chunk = [1u8; 1024 * 1024];
        (0..mb).for_each(|_| std::io::Write::write_all(&mut out, &chunk).expect("write"));
        drop(out);
        let file = std::fs::File::open(&path).expect("open");
        let map = unsafe {
            use std::os::fd::AsRawFd;
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_PRIVATE,
                file.as_raw_fd(),
                0,
            )
        };
        std::fs::remove_file(&path).expect("remove");
        assert_ne!(map, libc::MAP_FAILED);
        let pages = unsafe { std::slice::from_raw_parts(map as *const u8, len) };
        let read: u64 = pages.iter().step_by(4096).map(|b| *b as u64).sum();
        assert_eq!(read, (len / 4096) as u64);
        (map, len)
    }
}
