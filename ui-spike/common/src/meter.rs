//! Section 4's reading: lifetime max phys_footprint on the Mac, max resident on Linux. Frame time is the interval
//! between two presented frames, the only figure both renderers can give alike.
use std::time::Instant;

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
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) },
        0,
        "getrusage failed"
    );
    let peak_mb = usage.ru_maxrss as f64 * 1024.0 / MB;
    Memory {
        current_mb: peak_mb,
        peak_mb,
    }
}

/// Milliseconds since the kernel started this process, so dyld and static setup count in the first paint.
#[cfg(target_os = "macos")]
pub fn process_age_ms() -> f64 {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = size_of::<libc::proc_bsdinfo>() as i32;
    let ptr = &mut info as *mut libc::proc_bsdinfo as *mut libc::c_void;
    let got = unsafe { libc::proc_pidinfo(libc::getpid(), libc::PROC_PIDTBSDINFO, 0, ptr, size) };
    assert_eq!(got, size, "proc_pidinfo failed");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs_f64();
    (now - (info.pbi_start_tvsec as f64 + info.pbi_start_tvusec as f64 / 1e6)) * 1e3
}

#[cfg(not(target_os = "macos"))]
pub fn process_age_ms() -> f64 {
    f64::NAN
}

/// Why a window run would measure nothing: on a locked or sleeping Mac wgpu gets no surface (GetCurrentSurfaceTextureErr),
/// so Blitz skips each paint while its frames keep counting, and it printed 2 ms frames and a 21 MB peak.
#[cfg(target_os = "macos")]
pub fn display_refusal() -> Option<&'static str> {
    use std::ffi::{c_char, c_void};
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGDisplayIsAsleep(display: u32) -> u32;
        fn CGSessionCopyCurrentDictionary() -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            s: *const c_char,
            encoding: u32,
        ) -> *const c_void;
        fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
        fn CFBooleanGetValue(b: *const c_void) -> bool;
        fn CFRelease(o: *const c_void);
    }
    const UTF8: u32 = 0x0800_0100;
    unsafe {
        let session = CGSessionCopyCurrentDictionary();
        if session.is_null() {
            return Some("no window session");
        }
        let key =
            CFStringCreateWithCString(std::ptr::null(), c"CGSSessionScreenIsLocked".as_ptr(), UTF8);
        let locked = CFDictionaryGetValue(session, key);
        let locked = !locked.is_null() && CFBooleanGetValue(locked);
        CFRelease(key);
        CFRelease(session);
        if locked {
            return Some("the screen is locked");
        }
        (CGDisplayIsAsleep(CGMainDisplayID()) != 0).then_some("the display is asleep")
    }
}

#[cfg(not(target_os = "macos"))]
pub fn display_refusal() -> Option<&'static str> {
    None
}

/// Every window run starts here: no number from a window that cannot draw.
pub fn require_display() {
    if let Some(why) = display_refusal() {
        eprintln!(
            "refused: {why}, so a window draws nothing and its numbers would be false; use --headless or unlock"
        );
        std::process::exit(3);
    }
}

#[derive(Default)]
pub struct Run {
    first_paint_ms: Option<f64>,
    after_first_paint: Option<Memory>,
    frames: Vec<Instant>,
}

impl Run {
    /// Call once per presented frame.
    pub fn frame(&mut self) {
        if self.first_paint_ms.is_none() {
            self.first_paint_ms = Some(process_age_ms());
            self.after_first_paint = Some(memory());
        }
        self.frames.push(Instant::now());
    }

    pub fn frames(&self) -> usize {
        self.frames.len()
    }

    /// One JSON line on stdout: what section 9's table takes.
    pub fn report(&self, renderer: &str, first_row: usize) {
        println!("{}", self.json(renderer, first_row));
    }

    fn json(&self, renderer: &str, first_row: usize) -> String {
        let mut gaps: Vec<f64> = self
            .frames
            .windows(2)
            .map(|w| w[1].duration_since(w[0]).as_secs_f64() * 1e3)
            .collect();
        gaps.sort_by(f64::total_cmp);
        let at = |q: f64| {
            num(gaps
                .get(((gaps.len() as f64 - 1.0) * q) as usize)
                .copied()
                .unwrap_or(f64::NAN))
        };
        let end = memory();
        let paint = num(self.first_paint_ms.unwrap_or(f64::NAN));
        let idle = num(self
            .after_first_paint
            .as_ref()
            .map_or(f64::NAN, |m| m.current_mb));
        format!(
            "{{\"renderer\":\"{renderer}\",\"rows\":{},\"first_row_at_end\":{first_row},\"frames\":{},\"first_paint_ms\":{paint},\
             \"footprint_first_paint_mb\":{idle},\"peak_mb\":{:.1},\"end_mb\":{:.1},\"frame_ms\":{{\"p50\":{},\"p95\":{},\"p99\":{},\"max\":{}}}}}",
            crate::ROWS,
            self.frames.len(),
            end.peak_mb,
            end.current_mb,
            at(0.5),
            at(0.95),
            at(0.99),
            at(1.0)
        )
    }
}

/// A JSON number, or null: JSON has no NaN (Linux has no process age, one frame has no gap).
fn num(x: f64) -> String {
    if x.is_finite() {
        format!("{x:.2}")
    } else {
        "null".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_reads_a_plausible_peak() {
        let m = memory();
        assert!(m.peak_mb > 0.1 && m.peak_mb < 500.0, "peak {}", m.peak_mb);
        assert!(m.peak_mb >= m.current_mb);
    }

    #[test]
    fn a_run_reports_its_frames_and_percentiles() {
        let mut run = Run::default();
        run.frame();
        run.frame();
        assert_eq!(run.frames(), 2);
        let json = run.json("test", 7);
        assert!(
            json.contains("\"frames\":2") && json.contains("\"first_row_at_end\":7"),
            "{json}"
        );
        let empty = Run::default().json("empty", 0);
        assert!(
            !empty.contains("NaN") && empty.contains("\"p50\":null"),
            "{empty}"
        );
    }
}
