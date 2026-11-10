//! A morning run not done by 09:00 wakes the orchestrator to run it (task rust-morning-run-missed; @nha, 2026-11-04:
//! « prepare applications please. »). The run is a CronCreate job, and such a job lives in one Claude session only: the
//! 11-09 handover lost it, and `.claude/run/morning-run.last` stayed 2026-11-08 until 10:50. `dash orch` looks once a
//! minute (`watchers::MORNING`); from 09:00 local, a last run older than today sends one wake a day to each orchestrator (a
//! handover's new one gets its own), naming the prompt's source so it runs it word for word. A pause
//! (`morning-run.paused-until`, as the hook reads it) wakes none.
use crate::comments::wake_file;
use crate::tasks::{now_ms, repo};
use crate::watchers;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// From this local hour and minute (HH:MM), a morning run not done today wakes the orchestrator.
const BY: &str = "09:00";
/// The day and the wake file this watcher last woke, in the data folder: one wake a day per orchestrator.
const WOKE: &str = "morning-run.woke";

// POSIX's tzset, which the libc crate does not bind on every target.
unsafe extern "C" {
    fn tzset();
}

/// The local time now, as `YYYY-MM-DDTHH:MM`, the hook's `date +%Y-%m-%dT%H:%M`.
pub(crate) fn local_now() -> String {
    // SAFETY: localtime_r writes only into `tm`, which outlives the call. tzset first: localtime_r need not read TZ again,
    // and `dash orch` runs for days, across a change of hour.
    let tm = unsafe {
        tzset();
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm
    };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

/// The wake's words: what is late, and where the prompt to run comes from.
fn wake(last: &str, now: &str) -> String {
    format!(
        "Morning run missed: .claude/run/morning-run.last says « {last} » and it is {now}. Unless that file says today by \
         now, run the morning prompt, word for word as `.claude/hooks/morning-run.sh --prompt` prints it, and arm its \
         CronCreate job as that hook says (cron \"7 8 * * *\", recurring)."
    )
}

/// One pass at local time `now` (`YYYY-MM-DDTHH:MM`): `run` is `.claude/run`, `dir` the data folder. A line saying what it did.
fn pass(dir: &Path, run: &Path, now: &str) -> Result<String, String> {
    let read = |name: &str| std::fs::read_to_string(run.join(name)).unwrap_or_default();
    let (today, time) = now
        .split_once('T')
        .ok_or(format!("a time with no T: {now}"))?;
    let last = read("morning-run.last").trim().to_string();
    if last == today {
        return Ok(format!("ran today ({today})"));
    }
    if time < BY {
        return Ok(format!("last run {last:?}, before {BY}"));
    }
    // The hook's pause: only a YYYY-MM-DDTHH:MM holds, compared as text.
    let until = read("morning-run.paused-until").trim().to_string();
    let shape = until.len() == 16
        && until.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            10 => b == b'T',
            13 => b == b':',
            _ => b.is_ascii_digit(),
        });
    if shape && now < until.as_str() {
        return Ok(format!("paused until {until}"));
    }
    let Some(file) = wake_file(run) else {
        return Ok(format!("last run {last:?}, no live owner to wake"));
    };
    // per wake file: a handover's new orchestrator never read its predecessor's wake
    let woke = dir.join(WOKE);
    let mark = format!("{today} {}", file.display());
    let before = std::fs::read_to_string(&woke).unwrap_or_default();
    if before.trim() == mark {
        return Ok(format!("last run {last:?}, woken today already"));
    }
    // Kept as woken before the wake is written: a record that fails writes no wake, so it never repeats every pass. A
    // wake that fails to write puts the record back, for the next pass.
    std::fs::write(&woke, &mark).map_err(|e| format!("{}: {e}", woke.display()))?;
    let wrote = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)
        .and_then(|mut f| writeln!(f, "{}", wake(&last, now)));
    if let Err(e) = wrote {
        let _ = std::fs::write(&woke, before);
        return Err(format!("{}: {e}", file.display()));
    }
    let what = format!("the orchestrator ({}): morning run missed", file.display());
    if let Err(e) = watchers::woke(dir, &watchers::MORNING, now_ms(), &what) {
        eprintln!("morning: the wake's record: {e}");
    }
    Ok(format!("last run {last:?}, woke {}", file.display()))
}

/// A pass and its beat on /watchers, whatever it said.
fn beating(dir: &Path) -> Result<String, String> {
    let ran = pass(dir, &repo().join(".claude/run"), &local_now());
    let said = ran
        .as_ref()
        .map_or_else(|e| format!("morning: {e}"), String::clone);
    if let Err(e) = watchers::beat(dir, &watchers::MORNING, now_ms(), ran.is_ok(), &said) {
        eprintln!("morning: the beat: {e}");
    }
    ran
}

/// A pass every `watchers::MORNING.every_ms` for as long as `dash orch` runs; a line on stderr each time what it says changes.
pub fn watch(dir: PathBuf) {
    std::thread::spawn(move || {
        let mut said = String::new();
        loop {
            let line = match std::panic::catch_unwind(|| beating(&dir)) {
                Ok(Ok(line)) => line,
                Ok(Err(e)) => format!("morning: {e}"),
                Err(_) => "morning: a pass panicked, the next one runs".to_string(),
            };
            if line != said {
                eprintln!("morning run: {line}");
                said = line;
            }
            std::thread::sleep(Duration::from_millis(watchers::MORNING.every_ms as u64));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;

    #[test]
    fn a_morning_run_not_done_by_nine_wakes_once_and_none_once_it_ran() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("morning");
        let run = dir.0.join("run");
        std::fs::create_dir_all(&run).unwrap();
        // this test's process is the live owner: its state file and its pid
        let pid = std::process::id();
        std::fs::write(run.join("owner"), pid.to_string()).unwrap();
        std::fs::write(run.join(format!("{pid}.state")), "").unwrap();
        let wakes = || std::fs::read_to_string(run.join(format!("{pid}.wake"))).unwrap_or_default();
        std::fs::write(run.join("morning-run.last"), "2026-11-08\n").unwrap();

        // before 09:00: none
        pass(&dir.0, &run, "2026-11-09T08:59").unwrap();
        assert_eq!(wakes(), "");
        // 09:00 and the last run is yesterday: one wake, naming the prompt's source
        let said = pass(&dir.0, &run, "2026-11-09T09:00").unwrap();
        assert!(said.contains("woke"), "{said}");
        assert_eq!(wakes().lines().count(), 1);
        assert!(wakes().contains("« 2026-11-08 »"), "{}", wakes());
        assert!(
            wakes().contains(".claude/hooks/morning-run.sh"),
            "{}",
            wakes()
        );
        // a pass later the same day: still one
        pass(&dir.0, &run, "2026-11-09T10:30").unwrap();
        assert_eq!(wakes().lines().count(), 1);
        // the run wrote today: none, and none the day after before 09:00
        std::fs::write(run.join("morning-run.last"), "2026-11-09\n").unwrap();
        assert_eq!(
            pass(&dir.0, &run, "2026-11-09T11:00").unwrap(),
            "ran today (2026-11-09)"
        );
        pass(&dir.0, &run, "2026-11-10T08:00").unwrap();
        assert_eq!(wakes().lines().count(), 1);
        // the next day past 09:00 with no run: its own one wake
        pass(&dir.0, &run, "2026-11-10T09:05").unwrap();
        assert_eq!(wakes().lines().count(), 2);
        // paused: none; a pause that is no date never holds, as in the hook
        std::fs::write(run.join("morning-run.paused-until"), "2026-11-12T18:00").unwrap();
        assert_eq!(
            pass(&dir.0, &run, "2026-11-11T09:00").unwrap(),
            "paused until 2026-11-12T18:00"
        );
        std::fs::write(run.join("morning-run.paused-until"), "monday").unwrap();
        pass(&dir.0, &run, "2026-11-11T09:00").unwrap();
        assert_eq!(wakes().lines().count(), 3);
    }

    #[test]
    fn no_live_owner_wakes_nobody_and_the_next_pass_tries_again() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("morning-owner");
        let run = dir.0.join("run");
        std::fs::create_dir_all(&run).unwrap();
        assert_eq!(
            pass(&dir.0, &run, "2026-11-09T09:00").unwrap(),
            "last run \"\", no live owner to wake"
        );
        let pid = std::process::id();
        std::fs::write(run.join("owner"), pid.to_string()).unwrap();
        std::fs::write(run.join(format!("{pid}.state")), "").unwrap();
        assert!(
            pass(&dir.0, &run, "2026-11-09T09:01")
                .unwrap()
                .contains("woke")
        );
        // a handover: the new orchestrator gets its own wake that day, once, saying how to print the prompt
        let next = unsafe { libc::getppid() };
        std::fs::write(run.join("owner"), next.to_string()).unwrap();
        std::fs::write(run.join(format!("{next}.state")), "").unwrap();
        let said = pass(&dir.0, &run, "2026-11-09T09:30").unwrap();
        assert!(said.contains("woke"), "{said}");
        let said = pass(&dir.0, &run, "2026-11-09T09:31").unwrap();
        assert!(said.ends_with("woken today already"), "{said}");
        let wake = std::fs::read_to_string(run.join(format!("{next}.wake"))).unwrap();
        assert_eq!(wake.lines().count(), 1);
        assert!(
            wake.contains("`.claude/hooks/morning-run.sh --prompt`"),
            "{wake}"
        );
        assert!(wake.contains("Unless that file says today"), "{wake}");
    }
}
