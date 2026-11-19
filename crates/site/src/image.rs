//! The site's image, built on this Mac with no container and no VM (task rust-site-no-container): the server cross-built
//! to a static x86_64-unknown-linux-musl binary with rust-lld, the Nanolathe relay cross-built by Go, then `crane append`
//! of two layers onto Debian, pushed to the tag given (Fly's registry, Dashboard.SiteDeploy's login in DOCKER_CONFIG).
//! Layer one is `git archive` of site/ and bin/site-start at the sha, CLAUDE.md left out, so nothing else of the
//! repository can reach the image; layer two holds the two binaries. bin/site-start runs as before: `/app/bin/site start`
//! is this server where the Elixir release's launcher was. The base, the relay and its Go are pinned below, one pin each
//! (the Dockerfile held them until task rust-site-tick retired it; a sha older than that builds from its own image.rs,
//! which reads its own Dockerfile). Go builds with its version through GOTOOLCHAIN, fetched and checksummed by Go itself
//! when Homebrew's differs.
use sha2::Digest;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

const TARGET: &str = "x86_64-unknown-linux-musl";
/// Debian's image under the two layers: `docker.io/library/debian:<DEBIAN>`.
const DEBIAN: &str = "bookworm-20261005-slim";
/// The Nanolathe relay beside the site (rfc-nanolathe section 6, option A): upstream's cmd/nanolathe-server, unchanged, at
/// a fixed commit. A new relay ships only by moving this pin, never with a site edit. From public upstream: our fork is
/// private and the build holds no token (run 41636994439: « could not read Username », PR 1017's CI). Its relay files at
/// a00b8e4d are our fork's at b013462, byte for byte.
const RELAY: &str = "https://github.com/nanolathe-gg/nanolathe.git";
const NANOLATHE: &str = "a00b8e4d50e1062b7eb0b4b444be861ef2927c86";
/// The Go that builds the relay, as the Dockerfile's `FROM golang:1.27.1-bookworm` did.
const GO: &str = "1.27.1";

/// Builds `sha` of the checkout `repo` in the empty folder `dir` and pushes it as `tag` with `crane`.
pub fn build(repo: &Path, sha: &str, tag: &str, dir: &Path, crane: &str) -> Result<String, String> {
    let out = |p: &str| dir.join(p).to_string_lossy().into_owned();
    if run(Command::new("git").current_dir(repo).args([
        "cat-file",
        "-e",
        &format!("{sha}^{{commit}}"),
    ]))
    .is_err()
    {
        run(Command::new("git")
            .current_dir(repo)
            .args(["fetch", "-q", "origin", sha]))?;
    }

    std::fs::create_dir_all(dir.join("src")).map_err(|e| e.to_string())?;
    run(Command::new("git").current_dir(repo).args([
        "archive",
        "-o",
        &out("rust.tar"),
        sha,
        "rust",
    ]))?;
    run(Command::new("tar").args(["-xf", &out("rust.tar"), "-C", &out("src")]))?;
    run(Command::new("cargo")
        .current_dir(dir.join("src/rust"))
        .args([
            "build",
            "-q",
            "--release",
            "--locked",
            "--target",
            TARGET,
            "-p",
            "dash-site",
            "--bin",
            "site",
        ])
        .env("CARGO_TARGET_DIR", out("target"))
        .env("CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER", "rust-lld")
        .env(
            "CARGO_ENCODED_RUSTFLAGS",
            remaps(&dir.join("src/rust"), &dir.join("target"))?,
        ))?;

    let app = dir.join("layer/app");
    std::fs::create_dir_all(app.join("bin")).map_err(|e| e.to_string())?;
    std::fs::copy(
        dir.join(format!("target/{TARGET}/release/site")),
        app.join("bin/site"),
    )
    .map_err(|e| e.to_string())?;
    let bytes = std::fs::read(app.join("bin/site")).map_err(|e| e.to_string())?;
    let site_sha = sha2::Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();

    // upstream's cmd/nanolathe-server at the pinned commit, unchanged, as the Dockerfile's relay stage built it; no
    // credential helper, so a public fetch never asks the keychain
    let src = out("relay");
    std::fs::create_dir_all(&src).map_err(|e| e.to_string())?;
    run(Command::new("git").current_dir(&src).args(["init", "-q"]))?;
    run(Command::new("git").current_dir(&src).args([
        "-c",
        "credential.helper=",
        "fetch",
        "-q",
        "--depth",
        "1",
        RELAY,
        NANOLATHE,
    ]))?;
    let files = [
        "go.mod",
        "go.sum",
        "internal/relay",
        "internal/netproto",
        "cmd/nanolathe-server",
    ];
    run(Command::new("git")
        .current_dir(&src)
        .args(["checkout", "-q", "FETCH_HEAD", "--"])
        .args(files))?;
    run(Command::new("go")
        .current_dir(&src)
        .args([
            "build",
            "-trimpath",
            "-buildvcs=false",
            "-ldflags=-s -w",
            "-o",
        ])
        .arg(app.join("nanolathe-server"))
        .arg("./cmd/nanolathe-server")
        .envs([("GOOS", "linux"), ("GOARCH", "amd64"), ("CGO_ENABLED", "0")])
        .env("GOTOOLCHAIN", format!("go{GO}")))?;
    // modes set, not left to this process's umask: layer two's folders lie over layer one's, and a 0700 app/ would stop
    // nobody from starting the site (/code-review)
    for p in [
        app.clone(),
        app.join("bin"),
        app.join("bin/site"),
        app.join("nanolathe-server"),
    ] {
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }

    run(Command::new("git")
        .current_dir(repo)
        .args([
            "-c",
            "tar.umask=022",
            "archive",
            "--prefix=app/",
            "-o",
            &out("files.tar"),
            sha,
            "--",
        ])
        .args(["site", "bin/site-start", ":(exclude,glob)**/CLAUDE.md"]))?;
    run(Command::new("tar")
        .env("COPYFILE_DISABLE", "1")
        .args([
            "--uid",
            "0",
            "--gid",
            "0",
            "--uname",
            "root",
            "--gname",
            "root",
            "--no-xattrs",
            "--no-mac-metadata",
        ])
        .args(["-cf", &out("bin.tar"), "-C", &out("layer"), "app"]))?;

    // the layers under their own tag first: `tag` names only a whole image, since SiteDeploy never builds a held tag again
    let layers = format!("{tag}-layers");
    let base = format!("docker.io/library/debian:{DEBIAN}");
    run(Command::new(crane).args([
        "--platform",
        "linux/amd64",
        "append",
        "-b",
        &base,
        "-f",
        &out("files.tar"),
        "-f",
        &out("bin.tar"),
        "-t",
        &layers,
    ]))?;
    run(Command::new(crane)
        .args([
            "mutate",
            &layers,
            "-t",
            tag,
            "--cmd",
            "/app/bin/site-start",
            "--workdir",
            "/app",
        ])
        .args(
            ["SITE_ROOT=/app/site", "SITE_DATA=/data", "LANG=C.UTF-8"]
                .iter()
                .flat_map(|e| ["--env", e]),
        ))?;
    Ok(format!(
        "built {tag} on {base}, relay {NANOLATHE}, site sha256 {site_sha}\n"
    ))
}

/// The paths of this Mac taken out of the binary, so one sha gives the same bytes in any scratch folder here (another host
/// still gives other bytes) and no home folder ships: rust-src's std sources read as a toolchain without it reads them,
/// `/rustc/<commit>` (the image held `/Users/<him>/.rustup/...` until 2026-11-08). ci/site-musl.sh passes the same.
pub fn remaps(src: &Path, target: &Path) -> Result<String, String> {
    let home = std::env::var("CARGO_HOME")
        .unwrap_or_else(|_| format!("{}/.cargo", std::env::var("HOME").unwrap_or_default()));
    let sysroot = run(Command::new("rustc").args(["--print", "sysroot"]))?;
    let version = run(Command::new("rustc").arg("-vV"))?;
    let commit = version
        .lines()
        .find_map(|l| l.strip_prefix("commit-hash: "))
        .ok_or("rustc -vV names no commit-hash")?;
    Ok([
        (home, "/cargo".to_string()),
        (src.display().to_string(), "/src".to_string()),
        (target.display().to_string(), "/target".to_string()),
        (
            format!("{}/lib/rustlib/src/rust", sysroot.trim()),
            format!("/rustc/{commit}"),
        ),
    ]
    .iter()
    .map(|(from, to)| format!("--remap-path-prefix={from}={to}"))
    .collect::<Vec<_>>()
    .join("\x1f"))
}

/// Its stdout, or the command, its exit and its stderr's tail.
fn run(cmd: &mut Command) -> Result<String, String> {
    let name = format!(
        "{:?} {:?}",
        cmd.get_program(),
        cmd.get_args().take(2).collect::<Vec<_>>()
    );
    let out = cmd.output().map_err(|e| format!("{name}: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let tail: String = err
        .chars()
        .rev()
        .take(400)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Err(format!("{name}: {}: {}", out.status, tail.trim()))
}
