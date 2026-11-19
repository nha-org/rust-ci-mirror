//! `dash ci readings`: a CI box's memory and disk at a run's end (task ci-runner-pool; @nha, 2026-11-18: « I suggest
//! spinning the second server and seeing how it holds. »), so a week of superapp runs tells whether the 4 GB box holds a build
//! as the 8 GB one does.
//! What a box short of memory shows is the runner's cgroup's anonymous memory, its OOM kills and the swap used:
//! memory.peak counts page cache since the runner started, near the RAM after any build, so it is said as that, beside them.
use std::path::Path;

/// The line for runner `runner`, reading `root`'s `proc/self/cgroup`, its cgroup v2 files under `sys/fs/cgroup`,
/// `proc/meminfo`, and `df`'s answer for the home folder (`df -Pm`). A part that does not read says so; none fails.
pub fn readings(root: &Path, runner: &str, df: &str) -> String {
    let read = |path: &str| std::fs::read_to_string(root.join(path)).ok();
    let key = |text: &str, key: &str| -> Option<u64> {
        let line = text
            .lines()
            .find(|l| l.split_whitespace().next() == Some(key))?;
        line.split_whitespace().nth(1)?.parse().ok()
    };
    let mb = |bytes: u64| bytes / 1_048_576;
    // cgroup v2's one line, « 0::/system.slice/<the runner's service> »
    let cgroup = read("proc/self/cgroup").and_then(|t| {
        let path = t
            .lines()
            .find_map(|l| l.strip_prefix("0::"))?
            .trim()
            .to_string();
        Some(format!("sys/fs/cgroup{path}"))
    });
    let at = |file: &str| cgroup.as_ref().and_then(|c| read(&format!("{c}/{file}")));
    let anon = at("memory.stat").and_then(|t| key(&t, "anon"));
    let cgroup_said = match anon {
        Some(anon) => {
            let kills = at("memory.events").and_then(|t| key(&t, "oom_kill"));
            let peak = at("memory.peak").and_then(|t| t.trim().parse::<u64>().ok());
            format!(
                "anon {} MB, oom kills {}, peak with cache {} since the runner started",
                mb(anon),
                kills.map_or("unread".into(), |k| k.to_string()),
                peak.map_or("unread".into(), |p| format!("{} MB", mb(p))),
            )
        }
        None => "no cgroup v2 read".into(),
    };
    let box_said = match read("proc/meminfo") {
        Some(m) => {
            let kb = |k: &str| key(&m, k).unwrap_or(0);
            let (swap, free) = (kb("SwapTotal:"), kb("SwapFree:"));
            format!(
                "RAM {} MB, {} available, swap {} of {} MB used",
                kb("MemTotal:") / 1024,
                kb("MemAvailable:") / 1024,
                swap.saturating_sub(free) / 1024,
                swap / 1024
            )
        }
        None => "no /proc/meminfo".into(),
    };
    // df -Pm's second line, its fourth column: what is free, in MB
    let disk = (df.lines().nth(1))
        .and_then(|l| l.split_whitespace().nth(3)?.parse::<u64>().ok())
        .map_or("disk unread".into(), |d| format!("{d} MB disk free"));
    format!("readings on {runner}: {cgroup_said}; {box_said}; {disk}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::{TestDir, dash_test};

    /// The runner's cgroup on Debian (cgroup v2, its service's slice), /proc/meminfo and df's line, as stand-in files.
    fn box_with(cgroup: bool) -> TestDir {
        let dir = TestDir::new("ci-readings");
        let write = |path: &str, text: &str| {
            let at = dir.0.join(path);
            std::fs::create_dir_all(at.parent().expect("parent")).expect("dir");
            std::fs::write(at, text).expect("file");
        };
        write(
            "proc/self/cgroup",
            "0::/system.slice/actions.runner.ci.service\n",
        );
        write(
            "proc/meminfo",
            "MemTotal:        8144164 kB\nMemFree:          307200 kB\nMemAvailable:    5734400 kB\nSwapTotal:       4194300 kB\nSwapFree:        4071420 kB\n",
        );
        if cgroup {
            let slice = "sys/fs/cgroup/system.slice/actions.runner.ci.service";
            write(
                &format!("{slice}/memory.stat"),
                "anon 3221225472\nfile 4294967296\n",
            );
            write(
                &format!("{slice}/memory.events"),
                "low 0\nhigh 0\nmax 3\noom 2\noom_kill 1\n",
            );
            write(&format!("{slice}/memory.peak"), "8074035200\n");
        }
        dir
    }

    const DF: &str = "Filesystem     1048576-blocks  Used Available Capacity Mounted on\n/dev/vda1             161146 46228    114918      29% /\n";

    /// What a box short of memory shows: the cgroup's anonymous memory and its OOM kills, the swap used; memory.peak counts
    /// page cache since the runner started, so it is said as that, beside them. With no cgroup v2, the box's lines alone.
    #[dash_test]
    fn the_readings_say_anon_oom_kills_peak_swap_and_disk() {
        let dir = box_with(true);
        assert_eq!(
            readings(&dir.0, "ci", DF),
            "readings on ci: anon 3072 MB, oom kills 1, peak with cache 7700 MB since the runner started; RAM 7953 MB, \
             5600 available, swap 120 of 4095 MB used; 114918 MB disk free"
        );
        let bare = box_with(false);
        assert_eq!(
            readings(&bare.0, "ci-2", "no df"),
            "readings on ci-2: no cgroup v2 read; RAM 7953 MB, 5600 available, swap 120 of 4095 MB used; disk unread"
        );
    }
}
