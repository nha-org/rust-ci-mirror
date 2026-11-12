//! The mirror's Ubuntu packages from the .debs we hold, Ubuntu's servers only when they fail (task rust-ci-apt-retry):
//! from 13:50 to 17:05 on 2026-11-11 the runner's DNS failed every Ubuntu mirror and apt turned each run red (PR 1444,
//! PR 1446). ci.yml restores the held folder from the Actions cache and saves what this left there. Run by the
//! `held-debs` binary, not a dash command: dash links fontconfig, one of the packages it would install.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// `held-debs <package>...`: the held folder is $HELD, else ~/.cache/ci-debs, the path ci.yml caches.
fn main() {
    let packages: Vec<String> = std::env::args().skip(1).collect();
    let packages: Vec<&str> = packages.iter().map(String::as_str).collect();
    let held = std::env::var_os("HELD")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(".cache/ci-debs")
        });
    std::fs::create_dir_all(&held).expect("the held folder");
    let mut run = |cmd: &[&str]| {
        let out = Command::new(cmd[0])
            .args(&cmd[1..])
            .stderr(Stdio::inherit())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let archives = Path::new("/var/cache/apt/archives");
    match install(&held, archives, &packages, &mut run) {
        Ok(said) => {
            println!("{said}");
            // ci.yml saves the held folder only when apt refreshed it, under a key of this run's own
            if let (true, Some(out)) = (said.starts_with("apt:"), std::env::var_os("GITHUB_OUTPUT"))
            {
                let line = "refreshed=true\n";
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(out)
                    .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()))
                    .expect("GITHUB_OUTPUT");
            }
        }
        Err(why) => {
            eprintln!("held-debs: {why}");
            std::process::exit(1);
        }
    }
}

/// Runs one command, its stdout on success, None on a failure (its stderr already shown).
type Run<'a> = dyn FnMut(&[&str]) -> Option<String> + 'a;

fn status(run: &mut Run, package: &str) -> String {
    run(&["dpkg-query", "-W", "--showformat=${Status}", package]).unwrap_or_default()
}

fn installed(run: &mut Run, package: &str) -> bool {
    status(run, package) == "install ok installed"
}

fn debs_in(folder: &Path) -> Vec<PathBuf> {
    let mut debs: Vec<PathBuf> = std::fs::read_dir(folder)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    debs.retain(|p| p.extension().is_some_and(|e| e == "deb"));
    debs.sort();
    debs
}

/// Installs `packages`: the held .debs not installed yet in one `dpkg -i` (it orders their dependencies; an installed
/// package is skipped, never downgraded), then apt only if that left one missing. Apt's download, with each held package a
/// failed dpkg left installed or unpacked, becomes the held set. What it did, or why it failed.
fn install(
    held: &Path,
    archives: &Path,
    packages: &[&str],
    run: &mut Run,
) -> Result<String, String> {
    if packages.iter().all(|p| installed(run, p)) {
        return Ok(format!("installed already: {}", packages.join(" ")));
    }
    let debs: Vec<(PathBuf, String)> = debs_in(held)
        .into_iter()
        .filter_map(|d| {
            let name = run(&["dpkg-deb", "-f", &d.to_string_lossy(), "Package"])?;
            Some((d, name.trim().to_string()))
        })
        .collect();
    let fresh: Vec<String> = debs
        .iter()
        .filter(|(_, p)| !installed(run, p))
        .map(|(d, _)| d.to_string_lossy().into_owned())
        .collect();
    if !fresh.is_empty() {
        let mut dpkg = vec!["sudo", "dpkg", "-i"];
        dpkg.extend(fresh.iter().map(String::as_str));
        if run(&dpkg).is_some() && packages.iter().all(|p| installed(run, p)) {
            return Ok(format!(
                "held: {} from {} .debs",
                packages.join(" "),
                fresh.len()
            ));
        }
    }
    let why = if fresh.is_empty() {
        "no held .deb to install".to_string()
    } else {
        format!("the {} held .debs did not install them", fresh.len())
    };
    println!(
        "::warning::apt for {}: {why}, Ubuntu's servers asked",
        packages.join(" ")
    );
    // public resolvers: on 2026-11-11 the runner's own failed where 1.1.1.1 and 8.8.8.8 resolved
    let resolv = std::env::temp_dir().join("resolv.conf.public");
    std::fs::write(&resolv, "nameserver 1.1.1.1\nnameserver 8.8.8.8\n")
        .map_err(|e| e.to_string())?;
    let apt = ["sudo", "apt-get", "-o", "Acquire::Retries=3"];
    let steps: [&[&str]; 3] = [
        &["sudo", "mv", &resolv.to_string_lossy(), "/etc/resolv.conf"],
        &[&apt[..], &["update", "-qq"]].concat(),
        &["sudo", "apt-get", "clean"],
    ];
    for step in steps {
        run(step).ok_or_else(|| format!("{} failed", step.join(" ")))?;
    }
    // --reinstall: a held package a failed dpkg left in place is downloaded again, never lost from the set; the asked
    // packages alone if a held name left the archive
    let left: Vec<&str> = debs
        .iter()
        .map(|(_, p)| p.as_str())
        .filter(|p| status(run, p).starts_with("install ok"))
        .collect();
    let download = [
        &apt[..],
        &[
            "install",
            "-y",
            "-qq",
            "-f",
            "--reinstall",
            "--download-only",
        ],
        packages,
    ]
    .concat();
    if run(&[&download[..], &left[..]].concat()).is_none() {
        run(&download).ok_or("apt-get could not download them")?;
    }
    // every held file, one dpkg-deb could not read too: never carried into the next save (/code-review)
    for d in debs_in(held) {
        std::fs::remove_file(d).map_err(|e| e.to_string())?;
    }
    let got = debs_in(archives);
    for d in &got {
        std::fs::copy(d, held.join(d.file_name().unwrap_or_default()))
            .map_err(|e| e.to_string())?;
    }
    run(&[&apt[..], &["install", "-y", "-qq", "-f"], packages].concat())
        .ok_or("apt-get install failed")?;
    match packages.iter().find(|p| !installed(run, p)) {
        Some(p) => Err(format!("{p} is not installed after apt")),
        None => Ok(format!(
            "apt: {}, {} .debs now held",
            packages.join(" "),
            got.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::TestDir;
    use dash_meter::memory::TestBudget;
    use std::collections::BTreeSet;

    /// A runner whose dpkg installs a held .deb unless its name says `broken`, and whose apt installs what it is asked,
    /// its download left in `archives`. Each command is logged.
    struct Fake {
        installed: BTreeSet<String>,
        archives: PathBuf,
        log: Vec<String>,
    }

    impl Fake {
        fn run(&mut self, cmd: &[&str]) -> Option<String> {
            self.log.push(cmd.join(" "));
            let name = |p: &str| {
                Path::new(p)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            };
            match cmd {
                ["dpkg-query", .., p] => Some(if self.installed.contains(*p) {
                    "install ok installed".into()
                } else {
                    String::new()
                }),
                ["dpkg-deb", "-f", d, "Package"] => {
                    (!d.contains("corrupt")).then(|| name(d).replace("broken-", ""))
                }
                ["sudo", "apt-get", "clean"] => {
                    for d in debs_in(&self.archives) {
                        std::fs::remove_file(d).unwrap();
                    }
                    Some(String::new())
                }
                ["sudo", "dpkg", "-i", debs @ ..] => {
                    let ok = !debs.iter().any(|d| d.contains("broken"));
                    if ok {
                        self.installed.extend(debs.iter().map(|d| name(d)));
                    }
                    ok.then(String::new)
                }
                c if c.contains(&"--download-only") => {
                    let at = c.iter().position(|w| *w == "--download-only").unwrap();
                    for p in &c[at + 1..] {
                        std::fs::write(self.archives.join(format!("{p}.deb")), "").unwrap();
                    }
                    Some(String::new())
                }
                [
                    "sudo",
                    "apt-get",
                    "-o",
                    _,
                    "install",
                    "-y",
                    "-qq",
                    "-f",
                    wanted @ ..,
                ] => {
                    self.installed.extend(wanted.iter().map(|p| p.to_string()));
                    Some(String::new())
                }
                _ => Some(String::new()),
            }
        }
    }

    #[test]
    fn the_held_debs_install_first_and_apt_runs_only_when_they_fail() {
        let _budget = TestBudget::new();
        let dir = TestDir::new("held-debs");
        let (held, archives) = (dir.0.join("held"), dir.0.join("archives"));
        std::fs::create_dir_all(&held).unwrap();
        std::fs::create_dir_all(&archives).unwrap();
        for deb in ["bubblewrap", "libpoppler134"] {
            std::fs::write(held.join(format!("{deb}.deb")), "").unwrap();
        }
        let mut fake = Fake {
            installed: BTreeSet::new(),
            archives: archives.clone(),
            log: vec![],
        };
        let said = install(&held, &archives, &["bubblewrap"], &mut |c| fake.run(c));
        assert_eq!(said, Ok("held: bubblewrap from 2 .debs".to_string()));
        assert!(
            !fake.log.iter().any(|c| c.contains("apt-get")),
            "the held set installed, so Ubuntu's servers are never asked: {:?}",
            fake.log
        );

        // a held .deb dpkg refuses: apt after it, and its download alone is the new held set, an unreadable .deb and an
        // archive apt-get clean empties both gone
        std::fs::write(held.join("broken-poppler-utils.deb"), "").unwrap();
        std::fs::write(held.join("corrupt.deb"), "").unwrap();
        std::fs::write(archives.join("stale.deb"), "").unwrap();
        let mut fake = Fake {
            installed: BTreeSet::new(),
            archives: archives.clone(),
            log: vec![],
        };
        let said = install(&held, &archives, &["poppler-utils"], &mut |c| fake.run(c));
        assert_eq!(said, Ok("apt: poppler-utils, 1 .debs now held".to_string()));
        let at = |w: &str| fake.log.iter().position(|c| c.contains(w)).unwrap();
        let order = [
            "dpkg -i",
            "/etc/resolv.conf",
            "update -qq",
            "apt-get clean",
            "--download-only",
            "install -y -qq -f poppler-utils",
        ]
        .map(at);
        assert!(
            order.is_sorted(),
            "dpkg of the held set, then public DNS, update, clean, download, install: {:?}",
            fake.log
        );
        assert_eq!(debs_in(&held), [held.join("poppler-utils.deb")]);
    }
}
