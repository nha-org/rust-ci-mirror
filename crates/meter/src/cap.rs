//! The running program's limit (@nha, 2026-11-06: « we cap memory, and make sure it is efficient »): a wrapper around whichever
//! global allocator the binary's features pick, counting what Rust holds and stopping the process past `CAP_MB`. Section 4's
//! budgets are held by tests; this holds the program.
//!
//! ponytail: only Rust's allocations count, not SQLite's C heap, mapped files or the stack; memory.current_mb, the footprint,
//! shows those. A check of the footprint in the reporting thread if they ever outgrow the heap.
use crate::memory::Memory;
use std::alloc::{GlobalAlloc, Layout};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

/// The one place the cap is written.
pub const CAP_MB: usize = 200;
/// What a process stopped at the cap exits with: dash's own failures use 1, 2 and 3, so a caller can name this one.
pub const STOP_CODE: i32 = 86;

const MB: usize = 1024 * 1024;

// Statics, not fields: one process has one global allocator, and reporting.rs may not name main.rs, which holds it.
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static STOPPING: AtomicBool = AtomicBool::new(false);
const STDERR_WAIT_MS: i32 = 100;

/// The allocator, and what must happen at a stop, since no Drop runs there: it may not allocate or take a lock.
pub struct Capped<A>(pub A, pub fn());

/// What the wrapper counts, as of now and at its highest.
pub fn heap() -> Memory {
    Memory {
        current_mb: CURRENT.load(Relaxed) as f64 / MB as f64,
        peak_mb: PEAK.load(Relaxed) as f64 / MB as f64,
        file_mb: 0.0,
    }
}

fn grow(by: usize, at_stop: fn()) {
    let now = CURRENT.fetch_add(by, Relaxed) + by;
    // Read first: the add above just took this cache line, so the read is free, and only a new peak pays a second locked write.
    if now > PEAK.load(Relaxed) {
        PEAK.fetch_max(now, Relaxed);
    }
    if now > CAP_MB * MB {
        stop(now, at_stop);
    }
}

fn shrink(by: usize) {
    CURRENT.fetch_sub(by, Relaxed);
}

/// Inside the allocator: nothing here may allocate or take a lock, so the line is formatted on the stack, written straight to
/// stderr, and the process ends with no exit handlers. A pipe nobody reads must not hold the stop: past the wait the line goes
/// unsaid, and the exit code still names it. The line fits PIPE_BUF, so a pipe poll calls writable takes it whole.
fn stop(now: usize, at_stop: fn()) -> ! {
    // One line, one end: a second thread here waits for the first's exit.
    if STOPPING.swap(true, Relaxed) {
        loop {
            unsafe { libc::pause() };
        }
    }
    at_stop();
    let mut line = [0u8; 96];
    let room = line.len();
    let mut rest = &mut line[..];
    let _ = writeln!(
        rest,
        "dash: memory cap: reached {} MB, past the cap of {CAP_MB} MB, stopping",
        now / MB
    );
    let written = room - rest.len();
    let mut stderr = libc::pollfd {
        fd: libc::STDERR_FILENO,
        events: libc::POLLOUT,
        revents: 0,
    };
    unsafe {
        if libc::poll(&mut stderr, 1, STDERR_WAIT_MS) == 1 {
            libc::write(libc::STDERR_FILENO, line.as_ptr().cast(), written);
        }
        libc::_exit(STOP_CODE)
    }
}

unsafe impl<A: GlobalAlloc> GlobalAlloc for Capped<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        grow(layout.size(), self.1);
        let ptr = unsafe { self.0.alloc(layout) };
        if ptr.is_null() {
            shrink(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        grow(layout.size(), self.1);
        let ptr = unsafe { self.0.alloc_zeroed(layout) };
        if ptr.is_null() {
            shrink(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { self.0.dealloc(ptr, layout) };
        shrink(layout.size());
    }

    /// The allocator's own realloc, which may grow in place: the trait's default would copy every time. A grown block counts
    /// before the call, as an allocation does, and a failed one is taken back, since the old block stays. A move holds both blocks
    /// during its copy and only the new size counts: charging both would stop a block that grows in place.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let old = layout.size();
        if new_size > old {
            grow(new_size - old, self.1);
        }
        let moved = unsafe { self.0.realloc(ptr, layout, new_size) };
        if moved.is_null() {
            if new_size > old {
                shrink(new_size - old);
            }
        } else if old > new_size {
            shrink(old - new_size);
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budgets::{BUDGETS, Check};
    use std::alloc::System;

    /// A state the binary can reach must fit under the cap: the task that turns the 350 MB row into a check moves the cap first.
    #[dash_meter::dash_test]
    fn the_cap_covers_each_state_the_binary_can_reach() {
        for b in BUDGETS.iter().filter(|b| b.check == Check::Now) {
            assert!(b.limit_mb <= CAP_MB as f64, "{}", b.state);
        }
    }

    /// A test binary's global allocator is the plain one, so only these direct calls move the count.
    #[dash_meter::dash_test]
    fn the_count_follows_each_call_and_the_peak_stays() {
        let capped = Capped(System, || {});
        let layout = Layout::from_size_align(4 * MB, 8).expect("a layout");
        unsafe {
            let ptr = capped.alloc(layout);
            assert_eq!(heap().current_mb, 4.0);
            let ptr = capped.realloc(ptr, layout, 8 * MB);
            assert_eq!(heap().current_mb, 8.0);
            let bigger = Layout::from_size_align(8 * MB, 8).expect("a layout");
            let ptr = capped.realloc(ptr, bigger, MB);
            assert_eq!(heap().current_mb, 1.0);
            capped.dealloc(ptr, Layout::from_size_align(MB, 8).expect("a layout"));
            let zeroed = capped.alloc_zeroed(layout);
            assert_eq!(heap().current_mb, 4.0);
            capped.dealloc(zeroed, layout);
        }
        assert_eq!(heap().current_mb, 0.0);
        assert_eq!(heap().peak_mb, 8.0);
    }
}
