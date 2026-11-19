//! Task rust-one-launchd, done-when 2: launchd's one dash job, `dash serve` with DASH_ORCH set, starts the look and its watchers
//! itself (`dash orch --data <dir>`, its child), a look already holding the folder keeps the child waiting, so two never pick,
//! and a waiting look whose server is gone ends.
use dash_meter::TestDir;
use dash_meter::dash_test;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;

mod common;

#[dash_test]
fn serve_with_orch_spawns_the_look_and_it_waits_on_a_held_folder() {
    let dir = TestDir::new("serve-orch");
    // The look already there (launchd's rust-orch, or a look an older server left): its lock, held by this test, so the
    // child never runs a real pass.
    let held = std::fs::File::create(dir.0.join("orch.lock")).expect("the lock");
    held.lock().expect("held");
    let (mut server, _port) =
        common::serve(&dir.0, |c| c.env("DASH_ORCH", "1").stderr(Stdio::piped()));
    let stderr = server.0.stderr.take().expect("stderr");
    let (said, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = said.send(line);
        }
    });
    let want = format!(
        ": another look holds {}, waiting",
        dir.0.join("orch.lock").display()
    );
    let mut seen = vec![];
    // the waiting look's pid, from « dash orch <pid>: another look holds ... »
    let found = loop {
        match lines.recv_timeout(Duration::from_secs(20)) {
            Ok(line) if line.contains(&want) => {
                break line
                    .strip_prefix("dash orch ")
                    .and_then(|l| l.split(':').next()?.parse::<u32>().ok());
            }
            Ok(line) => seen.push(line),
            Err(_) => break None,
        }
    };
    assert!(
        found.is_some(),
        "no `dash orch` child waiting on the lock; serve said: {seen:#?}"
    );
    // The look runs in a process group of its own, so the server's end does not kill it: it ends itself, its server gone, so
    // restarts pile none up. Killed here past 10 s, so none outlives the test and takes the lock once it is freed.
    let pid = found.expect("its pid").to_string();
    let _ = server.0.kill();
    let _ = server.0.wait();
    let alive = || {
        Command::new("kill")
            .args(["-0", &pid])
            .status()
            .is_ok_and(|s| s.success())
    };
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while alive() && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(200));
    }
    let outlived = alive();
    let _ = Command::new("kill").args(["-9", &pid]).status();
    assert!(!outlived, "the look {pid} outlived its server by 10 s");
}
