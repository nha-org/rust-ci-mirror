//! `dash cv check` and `dash ats check`: the gates on his data that `mix cv.check` and `mix ats.check` ran, until the Elixir
//! app's source was archived (dash/rfcs/RFC-0025). The same Python checkers, from the repository's top, each run whatever
//! the one before said; run by hand on this Mac, where his data is (private.yml ran them until ci-runner-retire). Not mix's:
//! the whole tree is checked (mix narrowed a branch to its own files with CHECK_BASE), and draft_ai.py reads blocks.edn from the
//! live dash (`dash model blocks`), as it does when run by hand.
use dash_sdk::{Command, Host, Program};
use std::io::Write;
use std::time::Duration;

const CV: &str = "dash cv check [--ci]";
const ATS: &str = "dash ats check [--ci]";
const ATS_PY: &str = ".claude/skills/custom-cv/ats.py";
const DRAFT_AI: &str = ".claude/skills/nicolas-daily/draft_ai.py";

/// `mix cv.check`'s run: every unsent PDF in private/cv carries its Europass XML.
pub const CV_RUNS: [&[&str]; 1] = [&[".claude/skills/custom-cv/europass.py", "--check-all"]];

/// `mix ats.check`'s runs (`Dashboard.Lint.ats_checks`): ats.py and draft_ai.py, selftest then check, then doc_ready.py's
/// selftest. draft_ai.py reads blocks.edn through `dash model blocks` itself.
pub const ATS_RUNS: [&[&str]; 5] = [
    &[ATS_PY, "--selftest"],
    &[ATS_PY, "check"],
    &[DRAFT_AI, "--selftest"],
    &[DRAFT_AI, "check"],
    &[".claude/skills/nicolas-daily/doc_ready.py", "--selftest"],
];

/// Each run through python3 from the repository's top, its words written out; the runs that failed, named. `ci`: each run
/// gets CI=true, as GitHub's runner gave mix's, so the checkers skip their model half and never ask Jev.
pub fn gate(
    host: &dyn Host,
    runs: &[&[&str]],
    ci: bool,
    out: &mut dyn Write,
) -> Result<(), String> {
    let repo = host.repo();
    let given: &[(&str, &str)] = if ci { &[("CI", "true")] } else { &[] };
    let mut failed = vec![];
    for args in runs {
        let ran = host.run(&Program {
            bin: "python3",
            args,
            dir: Some(&repo),
            given,
            stdin: None,
            limit: Duration::from_secs(600),
        });
        match ran {
            Ok(r) => {
                let _ = write!(out, "{}{}", r.stdout, r.stderr);
                if r.code != 0 {
                    failed.push(args.join(" "));
                }
            }
            Err(e) => failed.push(format!("{}: {e}", args.join(" "))),
        }
    }
    match failed.is_empty() {
        true => Ok(()),
        false => Err(format!("failed: {}", failed.join("; "))),
    }
}

fn answer(host: &dyn Host, runs: &[&[&str]], usage: &str, args: &[&str], out: &mut dyn Write) {
    let ci = match args {
        [] => false,
        ["--ci"] => true,
        _ => {
            eprintln!("usage: {usage}");
            std::process::exit(2);
        }
    };
    if let Err(e) = gate(host, runs, ci, out) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn cv_run(host: &dyn Host, args: &[&str], out: &mut dyn Write) {
    answer(host, &CV_RUNS, CV, args, out)
}

fn ats_run(host: &dyn Host, args: &[&str], out: &mut dyn Write) {
    answer(host, &ATS_RUNS, ATS, args, out)
}

dash_sdk::submit! { Command { usage: CV, words: &["cv", "check"], run: Some(cv_run) } }
dash_sdk::submit! { Command { usage: ATS, words: &["ats", "check"], run: Some(ats_run) } }

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::{MemoryLog, Ran, StandIn, WallClock, dash_meter, dash_test};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    /// A host whose programs are recorded, each answering the code its script is given (0 else).
    struct Runs {
        stand: StandIn,
        codes: Vec<(&'static str, i32)>,
        ran: Mutex<Vec<String>>,
    }

    impl WallClock for Runs {
        fn now_ms(&self) -> i64 {
            0
        }
    }

    impl Host for Runs {
        fn repo(&self) -> PathBuf {
            self.stand.repo()
        }
        fn data(&self) -> PathBuf {
            self.stand.data()
        }
        fn switched(&self) -> Option<PathBuf> {
            self.stand.switched()
        }
        fn home(&self) -> Result<PathBuf, String> {
            self.stand.home()
        }
        fn arm(&self, _: &Path, _: &str) {}
        fn log(&self, _: &Path, _: &str, _: &[&str]) -> Result<Box<dyn dash_sdk::OpLog>, String> {
            Ok(Box::new(MemoryLog::default()))
        }
        fn run(&self, p: &Program) -> Result<Ran, String> {
            let line = format!(
                "{} {} in {:?} {:?}",
                p.bin,
                p.args.join(" "),
                p.dir,
                p.given
            );
            self.ran.lock().expect("lock").push(line);
            let code = (self.codes.iter())
                .find(|(s, _)| p.args.join(" ") == *s)
                .map_or(0, |c| c.1);
            Ok(Ran {
                code,
                stdout: format!("{} said\n", p.args[0]),
                stderr: String::new(),
            })
        }
    }

    fn host(codes: Vec<(&'static str, i32)>) -> Runs {
        Runs {
            stand: StandIn::default(),
            codes,
            ran: Mutex::new(vec![]),
        }
    }

    /// RFC-0025: mix ats.check's runs, in order, through python3 from the repository's top; a failing selftest leaves the
    /// rest to run and fails the gate, naming it; `dash cv check` is europass.py --check-all.
    #[dash_test]
    fn each_gate_runs_the_checkers_mix_ran_every_one_whatever_the_one_before_said() {
        let h = host(vec![(".claude/skills/custom-cv/ats.py --selftest", 1)]);
        let mut out = vec![];
        let why = gate(&h, &ATS_RUNS, false, &mut out).expect_err("a selftest failed");
        assert_eq!(why, "failed: .claude/skills/custom-cv/ats.py --selftest");
        let ran = h.ran.lock().expect("lock").clone();
        assert_eq!(ran.len(), 5, "{ran:?}");
        assert_eq!(
            ran[3],
            "python3 .claude/skills/nicolas-daily/draft_ai.py check in Some(\"/repo\") []"
        );
        assert!(
            String::from_utf8(out)
                .expect("text")
                .contains("doc_ready.py said")
        );
        // --ci, as private.yml ran it: the checkers skip their model half and never ask Jev, as under mix
        let h = host(vec![]);
        assert_eq!(gate(&h, &CV_RUNS, true, &mut vec![]), Ok(()));
        assert_eq!(
            h.ran.lock().expect("lock")[..],
            [
                "python3 .claude/skills/custom-cv/europass.py --check-all in Some(\"/repo\") [(\"CI\", \"true\")]"
            ]
        );
    }
}
