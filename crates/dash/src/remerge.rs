//! `dash forge remerge` and `dash forge train`: the stale-head merge and the merge train's push, from the repository's own
//! objects (`dash_forge::GitHub::remerge`, `train_build`, `train_drop`). They port the Elixir `Handover.merge_main` and
//! `MergeTrain`'s pushes, which wrote from an object overlay and left remote refs at missing commits (task
//! rust-fetch-heals-missing-ref). Run in a checkout of the PR's repository.
use crate::usage::Command;
use dash_forge::{GitHub, Remerge, TrainPr};
use std::fs::File;
use std::io::Write;
use std::path::Path;

fn forge() -> Result<GitHub, String> {
    let home = std::env::var("HOME").map_err(|e| format!("HOME: {e}"))?;
    GitHub::on(Path::new("."), Path::new(&home))
}

fn done(said: Result<String, String>, mut out: File) {
    match said {
        Ok(text) => {
            let _ = out.write_all(text.as_bytes());
        }
        Err(e) => {
            eprintln!("dash forge: {e}");
            std::process::exit(1);
        }
    }
}

/// `pushed <sha>`, `moved`, `holds` or `conflict <file>...`, as the Elixir merge_main's answers.
fn remerge(args: &[&str], out: File) {
    let said = match args {
        [url, branch, head] => forge().and_then(|f| f.remerge(url, branch, head)),
        _ => Err(format!("{USAGE_REMERGE}: {args:?}")),
    };
    done(
        said.map(|r| match r {
            Remerge::Pushed(sha) => format!("pushed {sha}\n"),
            Remerge::Moved => "moved\n".into(),
            Remerge::Holds => "holds\n".into(),
            Remerge::Conflicts(files) => format!("conflict {}\n", files.join(" ")),
        }),
        out,
    )
}

/// Each PR as four arguments, `<number> <head> <url> <title>`: a link holds colons, so no separator inside one argument.
fn train(args: &[&str], out: File) {
    let said = match args {
        ["drop", n] => n
            .parse()
            .map_err(|e| format!("{n}: {e}"))
            .and_then(|n| forge()?.train_drop(n))
            .map(|()| format!("train/{n} dropped\n")),
        ["build", n, prs @ ..] if !prs.is_empty() => built(n, prs),
        _ => Err(format!("{USAGE_TRAIN}: {args:?}")),
    };
    done(said, out)
}

fn train_prs(args: &[&str]) -> Result<Vec<TrainPr>, String> {
    if !args.len().is_multiple_of(4) {
        return Err(format!(
            "{args:?}: not <number> <head> <url> <title> for each PR"
        ));
    }
    (args.chunks(4))
        .map(|pr| {
            Ok(TrainPr {
                number: pr[0].parse().map_err(|e| format!("{}: {e}", pr[0]))?,
                head: pr[1].into(),
                url: pr[2].into(),
                title: pr[3].into(),
            })
        })
        .collect()
}

fn built(n: &str, prs: &[&str]) -> Result<String, String> {
    let n: u64 = n.parse().map_err(|e| format!("{n}: {e}"))?;
    let train = forge()?.train_build(n, &train_prs(prs)?)?;
    // nothing taken pushes nothing: a train/<n> an older build left stays as it was
    let mut text = match train.took.is_empty() {
        true => format!("nothing built: no PR squashes on {}\n", train.base),
        false => format!("base {}\ntip {}\n", train.base, train.tip),
    };
    train
        .took
        .iter()
        .for_each(|u| text += &format!("took {u}\n"));
    train
        .out
        .iter()
        .for_each(|(u, why)| text += &format!("out {u} {why}\n"));
    Ok(text)
}

const USAGE_REMERGE: &str = "dash forge remerge <pr-url> <branch> <head>";
const USAGE_TRAIN: &str =
    "dash forge train build <n> (<number> <head> <url> <title>)... | dash forge train drop <n>";

inventory::submit! {
    Command { usage: USAGE_REMERGE, words: &["forge", "remerge"], run: Some(remerge) }
}

inventory::submit! {
    Command { usage: USAGE_TRAIN, words: &["forge", "train"], run: Some(train) }
}

#[cfg(test)]
mod tests {
    /// /code-review of fde71c827: a PR link holds colons, so each PR is four arguments, the link and title whole.
    #[test]
    fn a_train_s_prs_are_four_arguments_each_a_link_kept_whole() {
        let _budget = dash_meter::memory::TestBudget::new();
        let link = "https://github.com/nha-org/site_perso/pull/7";
        let prs = super::train_prs(&["7", "abc", link, "fix: a title"]).expect("read");
        let pr = &prs[0];
        assert_eq!(
            (pr.number, &*pr.head, &*pr.url, &*pr.title),
            (7, "abc", link, "fix: a title")
        );
        assert!(
            super::train_prs(&["7", "abc", link]).is_err(),
            "a PR short of its title"
        );
        assert!(
            super::train_prs(&["x", "abc", link, "t"]).is_err(),
            "a number that is none"
        );
    }
}
