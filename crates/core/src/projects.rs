//! His projects: the one table every Rust reader of the project list reads (task rust-projects-registry), in place of the
//! older app's `config :dashboard, projects:` and its copies (@nha, 2026-11-14: « whatsnr has moved to codeberg », « we need
//! to setup a repo/ for each and corresponding sandbox and review tasks. the old repo marked inactive/reference only »).
//! A reference project keeps its folder and its board, read only: nothing starts, reviews or wakes there, and nothing files
//! or moves its tasks. ponytail: a const table, edited by a commit; a store table once projects change more than monthly.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forge {
    GitHub,
    Codeberg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Worked on: its tasks start, its PRs are reviewed, its sessions wake the orchestrator.
    Active,
    /// Kept to read: its board shows, nothing acts there.
    Reference,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Project {
    pub name: &'static str,
    /// Its checkout, under his home folder.
    pub folder: &'static str,
    pub forge: Forge,
    /// Its origin, as `git remote get-url origin` prints it.
    pub remote: &'static str,
    pub state: State,
}

const fn project(
    (name, folder): (&'static str, &'static str),
    forge: Forge,
    remote: &'static str,
    state: State,
) -> Project {
    Project {
        name,
        folder,
        forge,
        remote,
        state,
    }
}

use Forge::{Codeberg, GitHub};
use State::{Active, Reference};

/// In the sidebar's order. whatsnr moved to Codeberg on 2026-11-14, split in two; its old GitHub repo is kept for reference.
pub const PROJECTS: [Project; 6] = [
    project(
        ("site_perso", "repo/site_perso"),
        GitHub,
        "git@github.com:nha-org/site_perso.git",
        Active,
    ),
    project(
        ("klar", "repo/klar"),
        GitHub,
        "git@github.com:klar-im/internal.git",
        Active,
    ),
    project(
        ("graal", "repo/graal"),
        GitHub,
        "git@github.com:nha/graal.git",
        Active,
    ),
    project(
        ("whatsnr-webapp", "repo/whatsnr-webapp"),
        Codeberg,
        "ssh://git@codeberg.org/whatsnr/whatsnr-webapp.git",
        Active,
    ),
    project(
        ("whatsnr-plugins", "repo/whatsnr-plugins"),
        Codeberg,
        "ssh://git@codeberg.org/whatsnr/whatsnr-plugins.git",
        Active,
    ),
    project(
        ("geodb-whatsnr", "repo/geodb-whatsnr"),
        GitHub,
        "git@github.com:asiegf/geodb-whatsnr.git",
        Reference,
    ),
];

/// The project of that name, if the table holds one.
pub fn named(name: &str) -> Option<&'static Project> {
    PROJECTS.iter().find(|p| p.name == name)
}

/// Why `act` (start, review, wake, write) is refused in `name`: a reference project refuses every act; one the table lacks
/// (a folder of ~/repo, a `dash request` target) acts as before, but a write takes a folder's word, never a flag nor a path,
/// as the older app's repo check reads it. None: allowed.
pub fn refusal(name: &str, act: &str) -> Option<String> {
    let word = name.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c));
    match named(name).map(|p| p.state) {
        Some(Active) => None,
        Some(Reference) => Some(format!(
            "refused: {name} is kept for reference: no {act} there, its tasks are read-only"
        )),
        None if act == "write" && !word => Some(format!("refused: no such project: {name}")),
        None => None,
    }
}

/// Whether `project`'s board takes a new task, or why not: one of the table's, not kept for reference. None, the unfiltered
/// board, files on site_perso.
pub fn takes_tasks(project: Option<&str>) -> Result<(), String> {
    project
        .and_then(|p| refusal(p, "write"))
        .map_or(Ok(()), Err)
}

/// `name`'s checkout under his home folder: the table's, else `repo/<name>`.
pub fn folder(name: &str) -> String {
    named(name).map_or(format!("repo/{name}"), |p| p.folder.into())
}

/// `dash projects`: one line a project, its name, state, forge, remote and folder, tab-separated.
pub fn text() -> String {
    let line = |p: &Project| {
        let state = match p.state {
            Active => "active",
            Reference => "reference",
        };
        let forge = match p.forge {
            GitHub => "github",
            Codeberg => "codeberg",
        };
        format!(
            "{}\t{state}\t{forge}\t{}\t~/{}\n",
            p.name, p.remote, p.folder
        )
    };
    PROJECTS.iter().map(line).collect()
}

/// `dash projects`: the table, one line a project. `dash projects may <name> <act>`: exit 1 with its refusal, 0 when allowed.
pub fn command(words: &[&str], mut out: impl std::io::Write) -> i32 {
    let (code, said) = match words {
        [] => (0, text()),
        ["may", name, act] => refusal(name, act).map_or((0, String::new()), |r| (1, r + "\n")),
        _ => (
            2,
            "usage: dash projects [may <name> start|review|wake|write | follow <name>]\n".into(),
        ),
    };
    match out.write_all(said.as_bytes()) {
        Ok(()) => code,
        Err(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// Done-when 2 of rust-projects-registry: whatsnr-webapp and whatsnr-plugins are active on Codeberg, geodb-whatsnr is
    /// reference, and it refuses a start, a review, a wake and a write of its tasks; an active project refuses none.
    #[dash_test]
    fn whatsnr_is_active_on_codeberg_and_geodb_whatsnr_is_reference() {
        for name in ["whatsnr-webapp", "whatsnr-plugins"] {
            let p = named(name).expect("in the table");
            assert_eq!((p.forge, p.state), (Codeberg, Active), "{name}");
            assert!(
                p.remote.starts_with("ssh://git@codeberg.org/whatsnr/"),
                "{p:?}"
            );
        }
        let old = named("geodb-whatsnr").expect("kept");
        assert_eq!(old.state, Reference);
        for act in ["start", "review", "wake", "write"] {
            let said = refusal("geodb-whatsnr", act).expect("refused");
            assert!(said.contains(&format!("no {act} there")), "{said}");
            for active in [
                "site_perso",
                "klar",
                "graal",
                "whatsnr-webapp",
                "whatsnr-plugins",
            ] {
                assert_eq!(refusal(active, act), None, "{active} {act}");
            }
        }
        assert_eq!(
            refusal("../klar", "write").as_deref(),
            Some("refused: no such project: ../klar")
        );
        assert_eq!(
            refusal("replicator", "write"),
            None,
            "a folder the table lacks files as before"
        );
        assert_eq!(
            refusal("nowhere", "start"),
            None,
            "a repo the table lacks starts as before"
        );
        assert!(takes_tasks(Some("geodb-whatsnr")).is_err() && takes_tasks(Some("--on")).is_err());
        assert_eq!(
            (takes_tasks(Some("whatsnr-webapp")), takes_tasks(None)),
            (Ok(()), Ok(()))
        );
        assert_eq!(folder("whatsnr-plugins"), "repo/whatsnr-plugins");
        assert_eq!(folder("x"), "repo/x");
    }

    /// Done-when 3: `dash projects` lists each project with its state, every folder its own.
    #[dash_test]
    fn dash_projects_lists_each_with_its_state() {
        let text = text();
        print!("{text}");
        assert_eq!(text.lines().count(), PROJECTS.len());
        for p in PROJECTS {
            assert_eq!(p.folder, format!("repo/{}", p.name));
        }
        assert!(
            text.contains("whatsnr-webapp\tactive\tcodeberg\t"),
            "{text}"
        );
        assert!(
            text.contains("whatsnr-plugins\tactive\tcodeberg\t"),
            "{text}"
        );
        assert!(
            text.contains("geodb-whatsnr\treference\tgithub\t"),
            "{text}"
        );
        let mut said = vec![];
        assert_eq!(command(&["may", "geodb-whatsnr", "start"], &mut said), 1);
        assert!(String::from_utf8_lossy(&said).starts_with("refused: geodb-whatsnr"));
        assert_eq!(
            command(&["may", "whatsnr-webapp", "review"], &mut vec![]),
            0
        );
        assert!(text.contains(
            "site_perso\tactive\tgithub\tgit@github.com:nha-org/site_perso.git\t~/repo/site_perso\n"
        ));
    }
}
