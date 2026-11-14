//! `dash deploy --live <dir> --port <n>`: the binary rust/build just put in `<dir>` taken live on :<n>, the Makefile's rust/deploy
//! (task rust-deploy-keeps-new-build). The wait and the rollback are `dash_forge::live::go_live`, the forge's deploy's too.
use std::fs::File;
use std::io::Write;

const USAGE: &str = "dash deploy --live <dir> --port <n>";

fn deploy(args: &[&str], mut out: File) {
    let ["--live", live, "--port", port] = args else {
        eprintln!("usage: {USAGE}");
        std::process::exit(2);
    };
    let data = format!("{live}/data");
    crate::reporting::armed(&data, "deploy");
    let page = format!("http://127.0.0.1:{port}/built");
    match dash_forge::live::go_live(std::path::Path::new(live), &page) {
        Ok(said) => {
            let map = dash_forge::live::code_map(std::path::Path::new(live));
            writeln!(out, "deploy: {said}; {map}").unwrap_or_default()
        }
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(1);
        }
    }
}

inventory::submit! {
    crate::usage::Command { usage: USAGE, words: &["deploy"], run: Some(deploy) }
}
