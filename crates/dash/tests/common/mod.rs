//! What the tests that boot `dash serve` share: one copy, where each file had its own (task rust-line-cap-main).
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// A running `dash serve`, killed and reaped on drop.
pub struct Server(pub Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// `dash serve --data <data>`, with what `more` adds to its command, and the port its first line names. Its repo is
/// `<data>/repo` unless `more` names another: never the live checkout (task rust-tests-no-live-repo).
pub fn serve(data: &Path, more: impl FnOnce(&mut Command) -> &mut Command) -> (Server, u16) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dash"));
    command
        .args(["serve", "--data"])
        .arg(data)
        .env("DASH_REPO", data.join("repo"));
    let mut child = more(command.stdout(Stdio::piped())).spawn().expect("boot");
    let stdout = child.stdout.take().expect("stdout");
    let mut server = Server(child);
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("its port");
    let port = line.trim().strip_prefix("port ").map(str::parse);
    match port.and_then(Result::ok) {
        Some(port) => (server, port),
        None => panic!("port <n>: {line:?}, ended {:?}", server.0.try_wait()),
    }
}
