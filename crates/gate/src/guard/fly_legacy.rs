//! Refuses a flyctl command (fly, flyctl, bin/fly) that could move the Fly org off Legacy Hobby or past its free allowances:
//! one 256 MB machine, one 1 GB volume, a shared IPv4 (fly.toml). Was .claude/hooks/fly-legacy-guard.py; its selftest cases
//! are the tests below.
//!
//! (@nha, 2026-10-25: « I do want to KEEP the legacy hobby plan forever. It is great value for me. ») A plan switch or a
//! payment method removed cannot be undone; usage past $6 a month is billed (task fly-legacy-guard). Refused: any org
//! change, a plan or billing command, a remote build (fly launch, a deploy that builds on Fly's builder), a new app or a
//! managed service (each its own billed app), a second machine (machine run/clone/create, scale past one, a deploy without
//! --ha=false), a bigger machine (scale vm/memory, --vm-*), a volume made, forked or extended, a dedicated IPv4 or an egress
//! IP. A quoted heredoc's body is text unless a shell reads it; `bash -c '<code>'` and `eval` are read as code. ponytail:
//! flyctl inside a script (bin/site-deploy) or over ssh is not seen, and a bare word fly is read as a call (`echo fly
//! deploy` is refused, a false refusal only); bin/site-check fly audits the org after a deploy. A shell fed on a pipe or
//! another interpreter (python -c) is not read either: a scoped Fly token is the deeper guard.
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

/// global flags that take a value: their value is no subcommand word
const VALUED: [&str; 12] = [
    "-a",
    "--app",
    "-c",
    "--config",
    "-t",
    "--access-token",
    "-o",
    "--org",
    "-r",
    "--region",
    "-i",
    "--image",
];
const SERVICES: [&str; 19] = [
    "postgres",
    "pg",
    "mpg",
    "redis",
    "mysql",
    "consul",
    "extensions",
    "ext",
    "tigris",
    "storage",
    "litefs-cloud",
    "lfsc",
    "upstash",
    "sentry",
    "supabase",
    "kubernetes",
    "k8s",
    "wireguard",
    "wg",
];
const READS: [&str; 4] = ["list", "ls", "show", "status"];
/// no dash: the repo's own dash CLI shares the name (dash ship ... - <<'EOF' carries text)
const SHELLS: [&str; 3] = ["bash", "sh", "zsh"];
const SEP: &str = ";&|()`\n";
const PUNCT: &str = ";&|()\n";

/// each --vm-* flag and the values that keep the free machine
fn vm_ok(flag: &str) -> Option<&'static [&'static str]> {
    Some(match flag {
        "--vm-size" => &["shared-cpu-1x"],
        "--vm-memory" => &["256", "256mb"],
        "--vm-cpus" => &["1"],
        "--vm-cpu-kind" => &["shared"],
        "--vm-gpu-kind" => &[],
        "--vm-gpus" => &["0"],
        _ => return None,
    })
}

static SHELL_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[\s/;&|(])(?:ba|z)?sh(?:$|[\s;&|)<])").expect("shell"));
static COUNT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\w+=)?\d+$").expect("count"));
static CODE_FLAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^-\w*c$").expect("-c"));
static WORDS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[;&|()\n]|[^\s;&|()]+").expect("words"));

pub fn refused(cmd: &str) -> Option<String> {
    // every call is read: `f\\ly deploy` and `fl''y deploy` run fly with no « fly » in the text (/code-review of 79ddd91c3;
    // the .py skipped them)
    // a flag flyctl knows and this list does not may take a value: both readings, fail closed
    let why = calls(cmd)
        .iter()
        .find_map(|c| why(c, Some(&VALUED)).or_else(|| why(c, None)))?;
    Some(format!(
        "Refused by dash hook fly-legacy-guard: {why}. The Fly org stays on Legacy Hobby, inside its free allowances: one 256 MB machine, one 1 GB volume, a shared IPv4 (@nha, 2026-10-25: « I do want to KEEP the legacy hobby plan forever »). Deploy through the app (dash site deploy <sha>); a change of plan or allowance is his alone."
    ))
}

/// The command with each quoted heredoc body blanked, unless a shell on its line reads it (bash <<'EOF', cat <<'EOF' | ssh
/// host sh): the opening line and its line break kept, the rest an `x` per char. A body ends at its delimiter alone, tabs
/// before it only after `<<-`; at the first such line, as bash ends it.
fn bodies_blanked(cmd: &str) -> String {
    let c: Vec<char> = cmd.chars().collect();
    let mut out = String::with_capacity(cmd.len());
    let (mut i, mut done) = (0, 0);
    while i + 1 < c.len() {
        let Some((delim, dash, line_end)) =
            (c[i] == '<' && c[i + 1] == '<' && (i == 0 || c[i - 1] != '<'))
                .then(|| opener(&c, i + 2))
                .flatten()
        else {
            i += 1;
            continue;
        };
        let Some(end) = terminator(&c, line_end, &delim, dash) else {
            i += 1;
            continue;
        };
        let start = c[..i]
            .iter()
            .rposition(|&ch| ch == '\n')
            .map_or(0, |n| n + 1);
        let line: String = c[start..line_end].iter().collect();
        out.extend(&c[done..=line_end]);
        if SHELL_LINE.is_match(&line) {
            out.extend(&c[line_end + 1..end]);
        } else {
            out.extend(std::iter::repeat_n('x', end - line_end - 1));
        }
        (i, done) = (end, end);
    }
    out.extend(&c[done..]);
    out
}

/// From just after `<<`: a quoted delimiter, whether `-` came, and the line break ending the opening line.
fn opener(c: &[char], mut j: usize) -> Option<(String, bool, usize)> {
    let dash = c.get(j) == Some(&'-');
    if dash {
        j += 1;
    }
    while c.get(j).is_some_and(|ch| ch.is_whitespace()) {
        j += 1;
    }
    let quote = *c.get(j).filter(|q| matches!(q, '\'' | '"'))?;
    let start = j + 1;
    let len = c[start..]
        .iter()
        .take_while(|&&ch| ch.is_alphanumeric() || ch == '_')
        .count();
    if len == 0 || c.get(start + len) != Some(&quote) {
        return None;
    }
    let line_end = start + len + c[start + len..].iter().position(|&ch| ch == '\n')?;
    Some((c[start..start + len].iter().collect(), dash, line_end))
}

/// Just past the delimiter line that closes a body starting at the line break `at`.
fn terminator(c: &[char], at: usize, delim: &str, dash: bool) -> Option<usize> {
    let mut line = at + 1;
    while line <= c.len() {
        let eol = c[line..]
            .iter()
            .position(|&ch| ch == '\n')
            .map_or(c.len(), |n| line + n);
        let text: String = c[line..eol].iter().collect();
        let text = if dash {
            text.trim_start_matches('\t')
        } else {
            &text
        };
        if text == delim {
            return Some(eol);
        }
        line = eol + 1;
    }
    None
}

/// Python's shlex.shlex(posix=True, punctuation_chars=";&|()\n") with whitespace_split and whitespace " \t\r": words with
/// their quotes removed (a backslash in double quotes kept unless before `\` or `"`), a run of punctuation one token, a
/// `#` comment to the line's end, its line break with it. Err on an open quote or a final backslash.
fn shlex(text: &str) -> Result<Vec<String>, ()> {
    let c: Vec<char> = text.chars().collect();
    let blank = |ch: char| matches!(ch, ' ' | '\t' | '\r');
    let (mut out, mut tok, mut quoted, mut i) = (vec![], String::new(), false, 0);
    let mut punct = false;
    let flush = |out: &mut Vec<String>, tok: &mut String, quoted: &mut bool| {
        if !tok.is_empty() || *quoted {
            out.push(std::mem::take(tok));
        }
        *quoted = false;
    };
    while i < c.len() {
        let ch = c[i];
        if punct {
            if PUNCT.contains(ch) {
                tok.push(ch);
                i += 1;
                continue;
            }
            punct = false;
            flush(&mut out, &mut tok, &mut quoted);
        }
        if blank(ch) {
            flush(&mut out, &mut tok, &mut quoted);
            i += 1;
        } else if ch == '#' {
            flush(&mut out, &mut tok, &mut quoted);
            i = c[i..]
                .iter()
                .position(|&ch| ch == '\n')
                .map_or(c.len(), |n| i + n + 1);
        } else if PUNCT.contains(ch) {
            flush(&mut out, &mut tok, &mut quoted);
            tok.push(ch);
            punct = true;
            i += 1;
        } else if ch == '\'' {
            let n = c[i + 1..].iter().position(|&ch| ch == '\'').ok_or(())?;
            tok.extend(&c[i + 1..i + 1 + n]);
            quoted = true;
            i += n + 2;
        } else if ch == '"' {
            quoted = true;
            i += 1;
            loop {
                match *c.get(i).ok_or(())? {
                    '"' => break,
                    '\\' => {
                        let next = *c.get(i + 1).ok_or(())?;
                        if next != '\\' && next != '"' {
                            tok.push('\\');
                        }
                        tok.push(next);
                        i += 2;
                    }
                    other => {
                        tok.push(other);
                        i += 1;
                    }
                }
            }
            i += 1;
        } else if ch == '\\' {
            tok.push(*c.get(i + 1).ok_or(())?);
            i += 2;
        } else {
            tok.push(ch);
            i += 1;
        }
    }
    flush(&mut out, &mut tok, &mut quoted);
    Ok(out)
}

fn basename(w: &str) -> &str {
    w.rsplit('/').next().unwrap_or(w)
}

/// Each flyctl call in `cmd`: its words after the command name, up to the next ; & | ( ) or line break.
fn calls(cmd: &str) -> Vec<Vec<String>> {
    let cmd = bodies_blanked(cmd).replace("\\\n", " ");
    // an open quote: read it as plain words
    let toks = shlex(&cmd).unwrap_or_else(|()| {
        WORDS
            .find_iter(&cmd)
            .map(|m| m.as_str().trim_matches(['\'', '"']).to_string())
            .collect()
    });
    let (mut out, mut cur): (Vec<Vec<String>>, Option<usize>) = (vec![], None);
    let (mut shell, mut code) = (false, false);
    for (n, t) in toks.iter().enumerate() {
        let prev = if n == 0 { "" } else { toks[n - 1].as_str() };
        if t.chars().all(|ch| SEP.contains(ch)) {
            (cur, shell, code) = (None, false, false);
        } else if cur.is_none() && (code || prev == "eval") {
            // bash -c, bash -e -c, sh -lc, eval: its code
            (shell, code) = (false, false);
            let sub = calls(t);
            let words_only = !t.chars().any(char::is_whitespace);
            out.extend(sub.iter().cloned());
            // the open-quote fallback splits the code into words: the words after a lone fly go on its call
            cur = (!sub.is_empty() && words_only).then(|| out.len() - 1);
        } else if let Some(k) = cur {
            out[k].push(t.clone());
        } else if shell && t.starts_with('-') {
            code = CODE_FLAG.is_match(t);
        } else if SHELLS.contains(&basename(t)) {
            shell = true;
        } else if matches!(basename(t.trim_matches('`')), "fly" | "flyctl") {
            out.push(vec![]);
            cur = Some(out.len() - 1);
        }
    }
    out
}

/// The reason this flyctl call is refused, or None. `valued`: the flags taking a value; None reads every bare --flag so.
fn why(args: &[String], valued: Option<&[&str]>) -> Option<&'static str> {
    let (mut words, mut skip) = (vec![], false);
    for a in args {
        if skip {
            skip = false;
        } else if match valued {
            None => a.starts_with('-') && !a.contains('='),
            Some(v) => v.contains(&a.as_str()),
        } {
            skip = true;
        } else if !a.starts_with('-') {
            words.push(a.as_str());
        }
    }
    let (w0, w1) = (
        words.first().copied().unwrap_or(""),
        words.get(1).copied().unwrap_or(""),
    );
    let flags: HashSet<&str> = args.iter().map(String::as_str).collect();
    let has = |f: &str| flags.contains(f);
    let nxt: HashMap<&str, &str> = args
        .iter()
        .zip(args.iter().skip(1))
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    // --vm-size x, --vm-memory=x, on deploy, machine update, scale
    for a in args {
        let (k, v) = a.split_once('=').unwrap_or((a, ""));
        if let Some(ok) = vm_ok(k) {
            let v = if v.is_empty() {
                nxt.get(a.as_str()).copied().unwrap_or("")
            } else {
                v
            };
            if !ok.contains(&v.to_lowercase().as_str()) {
                return Some("a bigger machine leaves the free shared-cpu-1x 256 MB");
            }
        }
    }
    let read = READS.contains(&w1);
    if matches!(
        w0,
        "orgs" | "org" | "billing" | "plan" | "plans" | "payment" | "payments"
    ) && !read
    {
        return Some("it changes the org, its plan or its billing");
    }
    if w0 == "launch" || has("--remote-only") || has("--depot") {
        return Some("it builds on Fly's builder, a billed machine of its own");
    }
    if w0 == "deploy" {
        let image = has("--local-only")
            || has("--image")
            || has("-i")
            || args
                .iter()
                .any(|a| a.starts_with("--image=") || a.starts_with("-i="));
        if !image {
            return Some(
                "it builds on Fly's builder (deploy --image <pushed image> or --local-only, as site.yml does)",
            );
        }
        if !has("--ha=false") {
            return Some("without --ha=false Fly starts a second machine");
        }
    }
    if matches!(w0, "apps" | "app") && matches!(w1, "create" | "move" | "fork") {
        return Some("it adds an app or moves one to another org");
    }
    if SERVICES.contains(&w0) && !read {
        return Some("a managed service is another billed app");
    }
    let machine = matches!(w0, "machine" | "machines" | "m");
    if machine && matches!(w1, "run" | "create" | "clone") {
        return Some("it adds a machine");
    }
    if w0 == "console"
        && !(has("--machine")
            || has("-s")
            || has("--select")
            || args.iter().any(|a| a.starts_with("--machine=")))
    {
        return Some(
            "fly console starts a machine of its own (fly ssh console, or console --machine <id>)",
        );
    }
    if w0 == "scale" {
        let counts: Vec<&str> = words
            .iter()
            .skip(2)
            .filter(|w| COUNT.is_match(w))
            .map(|w| w.rsplit('=').next().unwrap_or(w))
            .collect();
        let mut regions: Vec<&str> = args
            .iter()
            .filter_map(|a| {
                let (k, v) = a.split_once('=').unwrap_or((a, ""));
                matches!(k, "-r" | "--region").then_some(v)
            })
            .collect();
        regions.extend(
            args.iter()
                .filter(|a| matches!(a.as_str(), "-r" | "--region"))
                .map(|a| nxt.get(a.as_str()).copied().unwrap_or("")),
        );
        if w1 == "count"
            && (counts != ["1"]
                || has("--max-per-region")
                || regions.iter().any(|r| r.contains(',')))
        {
            return Some("the site keeps one machine (scale count 1 only)");
        }
        if !matches!(w1, "show" | "count") {
            return Some("a bigger machine leaves the free shared-cpu-1x 256 MB");
        }
    }
    if matches!(w0, "volumes" | "volume" | "vol") && matches!(w1, "create" | "fork" | "extend") {
        return Some("the site keeps its one 1 GB volume, site_data");
    }
    if w0 == "ips" && w1 == "allocate-v4" && !has("--shared") {
        return Some("a dedicated IPv4 is billed (ips allocate-v4 --shared)");
    }
    if (w0 == "ips" && w1 == "allocate-egress") || (machine && w1 == "egress-ip") {
        return Some("a static egress IP is billed");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_meter::dash_test;

    /// fly-legacy-guard.py's selftest, its cases as they were
    #[dash_test]
    fn refuses_what_leaves_legacy_hobby_or_its_free_allowances() {
        for c in [
            "fly orgs create x",
            "bin/fly orgs delete personal",
            "flyctl orgs invite personal a@b.c",
            "fly orgs replace",
            "fly billing",
            "fly plan set hobby",
            "fly launch --no-deploy",
            "fly deploy",
            "fly deploy --remote-only --ha=false",
            "bin/fly deploy -a nicolas-ha --ha=false",
            "fly deploy --image registry.fly.io/x:1",
            "fly apps create other -o personal",
            "fly apps move nicolas-ha",
            "fly pg create",
            "fly ext tigris create",
            "fly redis create",
            "fly machine run nginx",
            "fly m clone 123 -a nicolas-ha",
            "fly machines create x",
            "fly scale count 2 --yes",
            "fly scale count 1 --region cdg,ams",
            "fly scale vm shared-cpu-2x",
            "fly scale memory 512",
            "fly volumes create site_data --size 1",
            "fly vol extend v1 --size 3",
            "fly volumes fork v1",
            "fly ips allocate-v4",
            "cd x && bin/fly -a nicolas-ha volumes create d",
            "echo ok; /opt/homebrew/bin/flyctl apps create x",
            "FLY_API_TOKEN=x fly launch",
            "bin/bounded 60 bin/fly orgs create x",
            "x=$(fly apps create y)",
            "bin/fly -a nicolas-ha deploy --build-only --depot",
            "bin/fly \"orgs\" 'create' x",
            "cat <<'EOF' > f\nx\nEOF\nfly volumes create d",
            "fly scale count 1 -r cdg,ams",
            "fly machine update 1 --vm-memory 1024",
            "fly m update 1 --vm-size=performance-8x",
            "fly deploy --image x --ha=false --vm-size shared-cpu-4x",
            "fly console -a nicolas-ha",
            "cat <<EOF\n$(fly orgs create x)\nEOF",
            "bash -c 'fly orgs create x'",
            "sh -c \"cd x && bin/fly apps create y\"",
            "bash -lc 'fly orgs create x'",
            "eval 'fly orgs create x'",
            "bash <<'EOF'\nfly orgs create x\nEOF",
            "bash -c 'fly orgs create x'; echo it's",
            "fly scale count 1 -r=cdg,ams",
            "cat <<'EOF' | bash\nfly orgs create x\nEOF",
            "cat <<'EOF' | ssh host sh\nfly orgs create x\nEOF",
            "bash -e -c 'fly orgs create x'",
            "bash --login -c 'fly orgs create x'",
            "fly --strategy immediate deploy",
            "fly --process-group app scale count 3",
            "fly ips allocate-egress -a nicolas-ha",
            "fly machine egress-ip allocate 1",
            "fly scale count 1 --max-per-region 3",
            "fly scale count app=2",
            // the shell's fly with no « fly » in the text (/code-review of 79ddd91c3)
            "f\\ly deploy",
            "fl''y deploy",
            "\"fl\"y orgs delete personal",
            "fl\"\"yctl scale count 3",
        ] {
            let why = refused(c).unwrap_or_else(|| panic!("passed: {c}"));
            assert!(
                why.starts_with("Refused by dash hook fly-legacy-guard: "),
                "{why}"
            );
        }
    }

    #[dash_test]
    fn passes_reads_and_the_deploys_the_site_runs() {
        for c in [
            "fly status -a nicolas-ha",
            "bin/fly apps list --json",
            "fly orgs list",
            "fly orgs show personal",
            "fly machines list --json -a nicolas-ha",
            "fly volumes list --json",
            "fly scale count 1 --yes -a nicolas-ha",
            "fly scale show",
            "fly ips allocate-v4 --shared --yes",
            "fly ips allocate-v6",
            "fly secrets list --json",
            "flyctl deploy --image registry.fly.io/nicolas-ha:abc --ha=false --strategy rolling",
            "fly deploy --local-only --ha=false",
            "fly logs -a nicolas-ha",
            "fly pg list",
            "fly certs add nicolas-ha.com",
            "git commit -m 'refuse fly deploy and fly orgs create'",
            "grep -n 'fly apps create' bin/site-deploy",
            "echo fly-legacy",
            "ls bin/fly",
            "cat fly.toml",
            "bin/site-deploy --deploy",
            "cat > f.md <<'EOF'\nfly deploy\nfly orgs create x\nEOF\ngh pr comment 5 -F f.md",
            "fly ssh console -a nicolas-ha",
            "fly console --machine 123",
            "fly m update 1 --vm-memory 256 --vm-size shared-cpu-1x",
            "gh pr comment 5 -F - <<'EOF'\nit's\nfly deploy\nEOF",
            "bash -c 'fly status'",
            "grep -rn fly deploy/",
            "fly machine update 1 --vm-gpus 0",
            "fly m update 1 --vm-memory=256MB",
            "git -c x=y status",
            "private/dashboard/bin/dash ship \"msg\" x - <<'EOF'\nnote: fly deploy is refused now\nEOF",
            "dash contact set c1 --notes - <<'EOF'\nhe said fly orgs create\nEOF",
            "fly scale count 1 --region cdg",
            "fly scale count --process-group app 1",
            "bash -e script.sh fly",
        ] {
            assert_eq!(refused(c), None, "{c}");
        }
    }

    #[dash_test]
    fn shlex_reads_as_python_does() {
        let words = |s: &str| shlex(s).unwrap();
        assert_eq!(words("a \"b c\" 'd'e"), ["a", "b c", "de"]);
        assert_eq!(words("a&&b;c\nd"), ["a", "&&", "b", ";", "c", "\n", "d"]);
        assert_eq!(words("a \"x\\y\\\"z\" \"\""), ["a", "x\\y\"z", ""]);
        assert_eq!(words("a # c\nb"), ["a", "b"]);
        assert!(shlex("echo 'open").is_err());
    }
}
